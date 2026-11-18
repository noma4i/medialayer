package com.noma4i.medialayer.ui

import kotlin.random.Random
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class VoicePcmQueueTest {
  @Test
  fun inv_voice_30_random_sequences_lose_no_byte_keep_order_and_fail_only_past_the_memory() {
    val seed = System.getenv("ML_SEED")?.trim()?.toLongOrNull() ?: 1L
    val repeat = "ML_SEED=$seed ./gradlew :ui:testDebugUnitTest --tests '*VoicePcmQueueTest'"
    val rate = 44_100
    for (round in 0 until 300) {
      val random = Random(seed * 1_000_003L + round)
      val limit = 4_096L + random.nextInt(8_192)
      val queue = VoicePcmQueue(limit, rate)
      var nextPtsUs = 0L
      var accepted = 0L
      var handed = 0L
      var held = 0L
      var expectedPtsUs = 0L
      repeat(200) {
        if (random.nextBoolean()) {
          val frames = 1 + random.nextInt(600)
          val bytes = ByteArray(frames * 2)
          val fits = held + bytes.size <= limit
          assertEquals("$repeat round $round add", fits, queue.add(bytes, nextPtsUs, 2))
          if (fits) { held += bytes.size; accepted += bytes.size; nextPtsUs += frames * 1_000_000L / rate }
        } else if (!queue.isEmpty) {
          val capacity = random.nextInt(0, 1_500)
          val part = queue.take(capacity)
          if (part == null) assertTrue("$repeat round $round no frame fits", capacity < 2) else {
            assertTrue("$repeat round $round whole frames within the input", part.size % 2 == 0 && part.size in 2..capacity)
            assertTrue("$repeat round $round pts never goes back", part.ptsUs >= expectedPtsUs)
            expectedPtsUs = part.ptsUs
            held -= part.size
            handed += part.size
          }
        }
        assertEquals("$repeat round $round held", held, queue.heldBytes)
        assertTrue("$repeat round $round bound", queue.heldBytes <= limit)
      }
      while (!queue.isEmpty) handed += queue.take(1_000_000)!!.size
      assertEquals("$repeat round $round every accepted byte is handed on", accepted, handed)
      assertEquals(0L, queue.heldBytes)
    }
  }

  @Test
  fun inv_voice_30_a_head_split_over_inputs_keeps_the_time_of_each_part() {
    val queue = VoicePcmQueue(10_000, 1_000)
    assertTrue(queue.add(ByteArray(40), 5_000, 2))
    val first = queue.take(16)!!
    val second = queue.take(16)!!
    val third = queue.take(16)!!
    assertEquals(listOf(16, 16, 8), listOf(first.size, second.size, third.size))
    assertEquals(listOf(5_000L, 13_000L, 21_000L), listOf(first.ptsUs, second.ptsUs, third.ptsUs))
    assertTrue(queue.isEmpty)
    assertFalse(queue.add(ByteArray(10_001), 0, 2))
    assertNull(runCatching { queue.take(1) }.getOrNull())
  }
}
