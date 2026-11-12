package com.noma4i.medialayer.ui

import android.os.Looper
import androidx.media3.common.Player
import androidx.media3.test.utils.StubPlayer
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@RunWith(MediaLayerTestRunner::class)
internal class PlayerProgressEndTest {
  private class Stopped(var state: Int, var position: Long) : StubPlayer() {
    private val listeners = mutableSetOf<Player.Listener>()
    override fun getApplicationLooper(): Looper = Looper.getMainLooper()
    override fun addListener(listener: Player.Listener) { listeners += listener }
    override fun removeListener(listener: Player.Listener) { listeners -= listener }
    override fun getPlaybackState(): Int = state
    override fun getCurrentPosition(): Long = position
    override fun getDuration(): Long = 10_000L
    override fun getPlayWhenReady(): Boolean = false
    override fun getPlaybackSuppressionReason(): Int = Player.PLAYBACK_SUPPRESSION_REASON_NONE
  }

  private fun reported(player: Stopped): Double {
    var value = -1.0
    val release = PlayerProgress.subscribe(player) { value = it }
    shadowOf(Looper.getMainLooper()).idle()
    release()
    return value
  }

  @Test
  fun inv_api_48_the_progress_is_one_when_the_end_is_reported_wherever_the_playhead_stopped() {
    assertEquals(0.9, reported(Stopped(Player.STATE_READY, 9_000L)), 0.0)
    assertEquals(1.0, reported(Stopped(Player.STATE_ENDED, 9_000L)), 0.0)
  }
}
