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
import com.noma4i.medialayer.SlotCall
import com.noma4i.medialayer.SlotSample
import kotlin.coroutines.resume
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext

class ExoPlayerSlot internal constructor(private val id: UInt, private val host: ExoPlayerSlotFactory) : PlayerSlot {
  private val looper = host.looperFor(id)
  private val commands = Handler(looper)
  @Volatile private var key: String? = null
  @Volatile private var wantsPlay = false
  @Volatile private var volumeNow = 1f
  private var hasBeenReady = false
  @Volatile private var released = false
  private var viewToken: ULong? = null
  internal var loadEvents: Player.Listener? = null

  @Volatile internal var player: ExoPlayer? = null
  @Volatile internal var output: LooperPlayer? = null
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

  private val housekeeping = object : Player.Listener {
    override fun onPlaybackStateChanged(playbackState: Int) {
      if (released) return
      refresh()
      reportAudible()
    }

    override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
      wantsPlay = playWhenReady
      reportAudible()
    }

    override fun onVolumeChanged(volume: Float) {
      volumeNow = volume
      reportAudible()
    }

    override fun onIsPlayingChanged(isPlaying: Boolean) = refresh()

    override fun onPositionDiscontinuity(oldPosition: Player.PositionInfo, newPosition: Player.PositionInfo, reason: Int) = refresh()

    override fun onPlaybackParametersChanged(playbackParameters: PlaybackParameters) = refresh()
  }

  private inner class LoadEvents(private val generation: ULong) : Player.Listener {
    override fun onPlaybackStateChanged(playbackState: Int) {
      if (released) return
      when (playbackState) {
        Player.STATE_READY -> { hasBeenReady = true; host.events?.ready(id, generation) }
        Player.STATE_BUFFERING -> if (hasBeenReady) host.events?.stalled(id, generation, currentPosition().toULong())
        Player.STATE_ENDED -> if (!looping) { refresh(); host.events?.ended(id, generation) }
      }
      MediaTrace.emit("player") { "slot=$id state=$playbackState generation=$generation looping=$looping key=${key.orEmpty().takeLast(24)}" }
    }

    override fun onRenderedFirstFrame() {
      if (released) return
      Trace.beginSection("MediaLayer.firstFrame")
      try {
        host.videoOutputCounters?.count("firstFrame")
        frameRendered = true
        host.events?.visualReady(id, generation)
      } finally {
        Trace.endSection()
      }
    }

    override fun onPlayerError(error: PlaybackException) {
      if (released) return
      host.events?.failed(id, generation, httpStatus(error), error.message ?: error.errorCodeName)
    }
  }

  override suspend fun apply(command: SlotCall) {
    host.videoOutputCounters?.count("mainPost")
    suspendCancellableCoroutine<Unit> { continuation ->
      val posted = commands.post {
        try {
          MediaTrace.emit("player") { "slot=$id ${command::class.simpleName} key=${key.orEmpty().takeLast(24)}" }
          if (!released && continuation.isActive) perform(command)
        } finally {
          if (continuation.isActive) continuation.resume(Unit)
        }
      }
      if (onTestChange != null && looper == Looper.getMainLooper()) onTestChange?.invoke()
      if (!posted) continuation.resume(Unit)
    }
  }

  private fun perform(command: SlotCall) = when (command) {
    is SlotCall.Load -> doLoad(command.key, command.route, command.generation)
    SlotCall.Unload -> doUnload()
    SlotCall.Preroll -> doPreroll()
    SlotCall.Play -> doPlay()
    SlotCall.Pause -> doPause()
    SlotCall.Mute -> { player?.volume = 0f; Unit }
    SlotCall.Unmute -> { player?.volume = 1f; Unit }
    is SlotCall.Attach -> doAttach(command.viewToken)
    SlotCall.Detach -> doDetach()
    is SlotCall.Seek -> { player?.seekTo(command.positionMs.toLong()); Unit }
    is SlotCall.Rate -> { player?.setPlaybackSpeed(command.rate); Unit }
    is SlotCall.Looping -> {
      looping = command.looping
      player?.repeatMode = if (looping) Player.REPEAT_MODE_ONE else Player.REPEAT_MODE_OFF
    }
    is SlotCall.Readahead -> doSetReadahead(command.ms)
  }

  private fun doLoad(key: String, route: PlaybackRoute, generation: ULong) {
    val exo = player ?: createPlayer()
    retract()
    this.key = key
    this.route = route
    hasBeenReady = false
    frameRendered = false
    looping = route.looping
    exo.repeatMode = if (looping) Player.REPEAT_MODE_ONE else Player.REPEAT_MODE_OFF
    loadEvents?.let(exo::removeListener)
    loadEvents = LoadEvents(generation).also(exo::addListener)
    exo.setMediaSource(host.mediaSourceFor(route))
    exo.playWhenReady = false
    host.players.publish(key, this, output!!)
    showInView()
    refresh()
  }

  private fun doUnload() {
    player?.setPlaybackSpeed(1f)
    player?.let { exo -> loadEvents?.let(exo::removeListener) }
    loadEvents = null
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

  private fun doPreroll() {
    val exo = player ?: return
    if (exo.playbackState == Player.STATE_IDLE) prepare(exo)
  }

  private fun doPlay() {
    val exo = player ?: return
    if (exo.playbackState == Player.STATE_IDLE) prepare(exo)
    exo.playWhenReady = true
  }

  private fun doPause() {
    player?.playWhenReady = false
  }

  private fun doAttach(viewToken: ULong) {
    Trace.beginSection("MediaLayer.attach")
    try {
      host.videoOutputCounters?.attach(id, key, viewToken)
      this.viewToken = viewToken
      showInView()
    } finally {
      Trace.endSection()
    }
  }

  private fun doDetach() {
    Trace.beginSection("MediaLayer.detach")
    try {
      host.videoOutputCounters?.detach(id)
      hideFromView()
      viewToken = null
    } finally {
      Trace.endSection()
    }
  }

  private fun doSetReadahead(ms: ULong) {
    readaheadMs = ms.toLong()
    loadControl?.setReadaheadMs(ms.toLong())
  }

  override fun sample(): SlotSample = SlotSample(livePosition().toULong(), durationMs?.toULong(), looping, viewAttached, frameRendered)

  internal fun snapshot(): PlayerSnapshot? = key?.let { PlayerSnapshot(it, wantsPlay, volumeNow == 0f, livePosition(), speed) }

  private fun livePosition(): Long {
    val position = if (advancing) anchorMs + ((host.clock.elapsedRealtime() - anchorAt) * speed).toLong() else anchorMs
    return durationMs?.let { position.coerceAtMost(it) } ?: position
  }

  override suspend fun dispose() {
    suspendCancellableCoroutine<Unit> { continuation ->
      val posted = commands.post {
        try {
          releaseNow()
        } finally {
          continuation.resume(Unit)
        }
      }
      if (posted && looper == Looper.getMainLooper()) onTestChange?.invoke()
      if (!posted) continuation.resume(Unit)
    }
    if (looper == Looper.getMainLooper()) return
    looper.quitSafely()
    withContext(Dispatchers.IO) { looper.thread.join() }
  }

  private fun releaseNow() {
    if (released) return
    released = true
    host.audio.setAudible(id, false)
    hideFromView()
    player?.removeListener(housekeeping)
    player?.let { exo -> loadEvents?.let(exo::removeListener) }
    loadEvents = null
    player?.let {
      it.release()
      host.videoOutputCounters?.count("release")
    }
    player = null
    output = null
    retract()
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
        .setLooper(looper)
        .build()
      exo.volume = if (host.startMuted) 0f else 1f
      volumeNow = exo.volume
      output = LooperPlayer(exo)
      exo.addListener(housekeeping)
      player = exo
      return exo
    } finally {
      Trace.endSection()
    }
  }

  private fun showInView() {
    val shown = output ?: return
    val view = viewToken?.let { host.players.view(it) } ?: return
    view.show(shown)
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
    durationMs = exo.duration.takeIf { it != C.TIME_UNSET }
    anchorMs = if (exo.playbackState == Player.STATE_ENDED) durationMs ?: exo.currentPosition else exo.currentPosition
    anchorAt = host.clock.elapsedRealtime()
    speed = exo.playbackParameters.speed
    advancing = exo.isPlaying
  }

  private fun httpStatus(error: PlaybackException): Int =
    generateSequence(error.cause) { it.cause }.filterIsInstance<HttpDataSource.InvalidResponseCodeException>().firstOrNull()?.responseCode ?: 0
}
