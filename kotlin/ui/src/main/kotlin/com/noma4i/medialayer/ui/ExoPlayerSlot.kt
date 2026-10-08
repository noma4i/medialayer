package com.noma4i.medialayer.ui

import android.os.Handler
import android.os.Looper
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.datasource.HttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import com.noma4i.medialayer.PlaybackRoute
import com.noma4i.medialayer.PlayerSlot
import com.noma4i.medialayer.RetainLevel
import com.noma4i.medialayer.SlotSample

class ExoPlayerSlot internal constructor(private val id: UInt, private val host: ExoPlayerSlotFactory) : PlayerSlot {
  private val main = Handler(Looper.getMainLooper())
  private var key: String? = null
  private var hasBeenReady = false
  private var released = false
  private var viewToken: ULong? = null

  @Volatile internal var player: ExoPlayer? = null
  @Volatile internal var viewAttached = false
  @Volatile private var looping = false
  @Volatile private var positionMs = 0L
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
    }

    override fun onRenderedFirstFrame() {
      if (released) return
      frameRendered = true
      host.events?.visualReady(id)
    }

    override fun onIsPlayingChanged(isPlaying: Boolean) {
      if (isPlaying) scheduleTick()
    }

    override fun onPlayerError(error: PlaybackException) {
      if (released) return
      host.events?.failed(id, httpStatus(error), error.message ?: error.errorCodeName)
    }
  }

  private val tick = object : Runnable {
    override fun run() {
      refresh()
      if (player?.isPlaying == true) main.postDelayed(this, TICK_MS)
    }
  }

  override fun load(key: String, route: PlaybackRoute) = onMain {
    val exo = player ?: createPlayer()
    retract()
    this.key = key
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
    hideFromView()
    viewToken = null
    clear()
    retract()
    key = null
    frameRendered = false
    positionMs = 0
    durationMs = null
  }

  override fun preroll() = onMain {
    val exo = player ?: return@onMain
    if (exo.playbackState == Player.STATE_IDLE) exo.prepare()
  }

  override fun play() = onMain {
    val exo = player ?: return@onMain
    if (exo.playbackState == Player.STATE_IDLE) exo.prepare()
    exo.playWhenReady = true
    scheduleTick()
  }

  override fun pause() = onMain { player?.playWhenReady = false }

  override fun mute() = onMain { player?.volume = 0f }

  override fun unmute() = onMain { player?.volume = 1f }

  override fun attach(viewToken: ULong) = onMain {
    this.viewToken = viewToken
    showInView()
  }

  override fun detach() = onMain {
    hideFromView()
    viewToken = null
  }

  override fun seek(positionMs: ULong) = onMain { player?.seekTo(positionMs.toLong()) }

  override fun retain(level: RetainLevel) = onMain {
    if (level == RetainLevel.COLD) clear()
  }

  override fun sample(): SlotSample = SlotSample(positionMs.toULong(), durationMs?.toULong(), looping, viewAttached, frameRendered)

  internal fun release() {
    main.post { releaseNow() }
  }

  private fun releaseNow() {
    released = true
    main.removeCallbacks(tick)
    hideFromView()
    player?.removeListener(listener)
    player?.release()
    player = null
    retract()
  }

  private fun onMain(work: () -> Unit) {
    main.post { if (!released) work() }
  }

  private fun createPlayer(): ExoPlayer {
    val attributes = AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MOVIE).build()
    val exo = ExoPlayer.Builder(host.appContext, host.renderersFactory)
      .setLoadControl(host.loadControl.create(host.allocator))
      .setAudioAttributes(attributes, true)
      .setClock(host.clock)
      .build()
    exo.volume = if (host.startMuted) 0f else 1f
    exo.addListener(listener)
    player = exo
    return exo
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

  private fun scheduleTick() {
    main.removeCallbacks(tick)
    main.post(tick)
  }

  private fun currentPosition(): Long = player?.currentPosition ?: 0L

  private fun refresh() {
    val exo = player ?: return
    positionMs = exo.currentPosition
    durationMs = exo.duration.takeIf { it != C.TIME_UNSET }
  }

  private fun httpStatus(error: PlaybackException): Int =
    generateSequence(error.cause) { it.cause }.filterIsInstance<HttpDataSource.InvalidResponseCodeException>().firstOrNull()?.responseCode ?: 0

  private companion object {
    const val TICK_MS = 250L
  }
}
