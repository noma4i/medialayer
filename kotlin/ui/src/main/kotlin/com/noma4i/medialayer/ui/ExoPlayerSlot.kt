package com.noma4i.medialayer.ui

import android.os.Handler
import android.os.Looper
import android.os.Trace
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.PlaybackException
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.Player
import androidx.media3.datasource.HttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import com.noma4i.medialayer.PlaybackRoute
import com.noma4i.medialayer.PlayerSlot
import com.noma4i.medialayer.SlotSample

class ExoPlayerSlot internal constructor(private val id: UInt, private val host: ExoPlayerSlotFactory) : PlayerSlot {
  private val main = Handler(Looper.getMainLooper())
  private var key: String? = null
  private var hasBeenReady = false
  private var released = false
  private var viewToken: ULong? = null

  @Volatile internal var player: ExoPlayer? = null
  @Volatile internal var route: PlaybackRoute? = null
  private var loadControl: ReadaheadLoadControl? = null
  private var readaheadMs: Long? = null
  @Volatile internal var viewAttached = false
  @Volatile private var looping = false
  @Volatile private var anchorMs = 0L
  @Volatile private var anchorAt = 0L
  @Volatile private var speed = 1f
  @Volatile private var advancing = false
  @Volatile private var durationMs: Long? = null
  @Volatile private var frameRendered = false

  internal val listener = object : Player.Listener {
    override fun onPlaybackStateChanged(playbackState: Int) {
      if (released) return
      when (playbackState) {
        Player.STATE_READY -> { hasBeenReady = true; host.events?.ready(id) }
        Player.STATE_BUFFERING -> if (hasBeenReady) host.events?.stalled(id, currentPosition().toULong())
        Player.STATE_ENDED -> if (!looping) host.events?.ended(id)
      }
      refresh()
      reportAudible()
    }

    override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) = reportAudible()

    override fun onVolumeChanged(volume: Float) = reportAudible()

    override fun onRenderedFirstFrame() {
      if (released) return
      Trace.beginSection("MediaLayer.firstFrame")
      try {
        host.videoOutputCounters?.count("firstFrame")
        frameRendered = true
        host.events?.visualReady(id)
      } finally {
        Trace.endSection()
      }
    }

    override fun onIsPlayingChanged(isPlaying: Boolean) = refresh()

    override fun onPositionDiscontinuity(oldPosition: Player.PositionInfo, newPosition: Player.PositionInfo, reason: Int) = refresh()

    override fun onPlaybackParametersChanged(playbackParameters: PlaybackParameters) = refresh()

    override fun onPlayerError(error: PlaybackException) {
      if (released) return
      host.events?.failed(id, httpStatus(error), error.message ?: error.errorCodeName)
    }
  }

  override fun load(key: String, route: PlaybackRoute) = onMain {
    val exo = player ?: createPlayer()
    retract()
    this.key = key
    this.route = route
    hasBeenReady = false
    frameRendered = false
    looping = route.looping
    exo.repeatMode = if (looping) Player.REPEAT_MODE_ONE else Player.REPEAT_MODE_OFF
    exo.setMediaSource(host.mediaSourceFor(route))
    exo.playWhenReady = false
    host.players.publish(key, this, exo)
    showInView()
    refresh()
  }

  override fun unload() = onMain {
    player?.setPlaybackSpeed(1f)
    hideFromView()
    viewToken = null
    clear()
    retract()
    key = null
    route = null
    frameRendered = false
    anchorMs = 0
    advancing = false
    durationMs = null
  }

  override fun preroll() = onMain {
    val exo = player ?: return@onMain
    if (exo.playbackState == Player.STATE_IDLE) prepare(exo)
  }

  override fun play() = onMain {
    val exo = player ?: return@onMain
    if (exo.playbackState == Player.STATE_IDLE) prepare(exo)
    exo.playWhenReady = true
  }

  override fun pause() = onMain { player?.playWhenReady = false }

  override fun mute() = onMain { player?.volume = 0f }

  override fun unmute() = onMain { player?.volume = 1f }

  override fun setRate(rate: Float) = onMain { player?.setPlaybackSpeed(rate) }

  override fun attach(viewToken: ULong) = onMain {
    Trace.beginSection("MediaLayer.attach")
    try {
      host.videoOutputCounters?.attach(id, key, viewToken)
      this.viewToken = viewToken
      showInView()
    } finally {
      Trace.endSection()
    }
  }

  override fun detach() = onMain {
    Trace.beginSection("MediaLayer.detach")
    try {
      host.videoOutputCounters?.detach(id)
      hideFromView()
      viewToken = null
    } finally {
      Trace.endSection()
    }
  }

  override fun seek(positionMs: ULong) = onMain { player?.seekTo(positionMs.toLong()) }

  override fun setReadahead(ms: ULong) = onMain {
    readaheadMs = ms.toLong()
    loadControl?.setReadaheadMs(ms.toLong())
  }

  override fun sample(): SlotSample = SlotSample(livePosition().toULong(), durationMs?.toULong(), looping, viewAttached, frameRendered)

  private fun livePosition(): Long {
    val position = if (advancing) anchorMs + ((host.clock.elapsedRealtime() - anchorAt) * speed).toLong() else anchorMs
    return durationMs?.let { position.coerceAtMost(it) } ?: position
  }

  internal fun release() {
    main.post { releaseNow() }
  }

  private fun releaseNow() {
    released = true
    host.audio.setAudible(id, false)
    hideFromView()
    player?.removeListener(listener)
    player?.let {
      it.release()
      host.videoOutputCounters?.count("release")
    }
    player = null
    retract()
  }

  private fun onMain(work: () -> Unit) {
    host.videoOutputCounters?.count("mainPost")
    main.post { if (!released) work() }
  }

  private fun prepare(exo: ExoPlayer) {
    Trace.beginSection("MediaLayer.prepare")
    try {
      host.videoOutputCounters?.count("prepare")
      exo.prepare()
    } finally {
      Trace.endSection()
    }
  }

  private fun createPlayer(): ExoPlayer {
    Trace.beginSection("MediaLayer.create")
    try {
      host.videoOutputCounters?.count("create")
      val attributes = AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MOVIE).build()
      val control = host.loadControl.create(host.allocator)
      readaheadMs?.let { control.setReadaheadMs(it) }
      loadControl = control
      val exo = ExoPlayer.Builder(host.appContext, host.renderersFactory)
        .setLoadControl(control)
        .setAudioAttributes(attributes, host.audio.focus == MediaAudioFocus.Exclusive)
        .setClock(host.clock)
        .build()
      exo.volume = if (host.startMuted) 0f else 1f
      exo.addListener(listener)
      player = exo
      return exo
    } finally {
      Trace.endSection()
    }
  }

  private fun showInView() {
    val exo = player ?: return
    val view = viewToken?.let { host.players.view(it) } ?: return
    view.show(exo)
    viewAttached = true
  }

  private fun hideFromView() {
    viewToken?.let { host.players.view(it) }?.show(null)
    viewAttached = false
  }

  private fun clear() {
    val exo = player ?: return
    exo.playWhenReady = false
    exo.stop()
    exo.clearMediaItems()
  }

  private fun retract() {
    val current = key ?: return
    host.players.retract(current, this)
  }

  private fun currentPosition(): Long = player?.currentPosition ?: 0L

  private fun reportAudible() {
    val exo = player ?: return
    host.audio.setAudible(id, isAudible(exo.volume, exo.playWhenReady, exo.playbackState, exo.mediaItemCount > 0))
  }

  private fun refresh() {
    val exo = player ?: return
    anchorMs = exo.currentPosition
    anchorAt = host.clock.elapsedRealtime()
    speed = exo.playbackParameters.speed
    advancing = exo.isPlaying
    durationMs = exo.duration.takeIf { it != C.TIME_UNSET }
  }

  private fun httpStatus(error: PlaybackException): Int =
    generateSequence(error.cause) { it.cause }.filterIsInstance<HttpDataSource.InvalidResponseCodeException>().firstOrNull()?.responseCode ?: 0
}
