package com.noma4i.medialayer.ui

import android.graphics.PointF
import android.graphics.RectF
import android.util.Size
import com.noma4i.medialayer.CropFractions
import com.noma4i.medialayer.CropGesture
import com.noma4i.medialayer.CropGestureReport
import com.noma4i.medialayer.CropHandle
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaSourceValue
import com.noma4i.medialayer.PhotoEdit
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.photoPlan
import java.awt.image.BufferedImage
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import javax.imageio.ImageIO
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

private class Admission {
  val asked = CopyOnWriteArrayList<Long>()
  val closed = AtomicInteger()
  var refuses = false
  val admit: DecodeAdmit = { bytes ->
    asked.add(bytes)
    if (refuses) throw MediaException.Overloaded("no room")
    AutoCloseable { closed.incrementAndGet() }
  }
}

private fun jpegFile(width: Int, height: Int): File =
  File.createTempFile("crop", ".jpg").apply {
    ImageIO.write(BufferedImage(width, height, BufferedImage.TYPE_INT_RGB), "jpg", this)
    deleteOnExit()
  }

private fun MediaCrop.photoReady(width: Int, height: Int, viewWidth: Int = 400, viewHeight: Int = 800) {
  place(viewWidth, viewHeight, 0f)
  ready(PixelSize(width.toUInt(), height.toUInt()))
}

private fun near(actual: CropFractions, x: Double, y: Double, width: Double, height: Double, name: String = "") {
  assertTrue("$name $actual", kotlin.math.abs(actual.x - x) < 1e-6 && kotlin.math.abs(actual.y - y) < 1e-6 && kotlin.math.abs(actual.width - width) < 1e-6 && kotlin.math.abs(actual.height - height) < 1e-6)
}

@RunWith(MediaLayerTestRunner::class)
class MediaCropTest {
  private val chat = Size(1080, 1920)

  @Test
  fun inv_crop_11_gesturesInTheModelGiveTheRectangleOfTheTable() {
    val fixed = MediaCrop(chat) { }
    fixed.photoReady(2000, 3000)
    near(fixed.rect, 0.078125, 0.0, 0.84375, 1.0, "first frame")
    fixed.pan(-23.7037037f, 30f)
    near(fixed.rect, 0.128125, 0.0, 0.84375, 1.0, "the photo follows the finger to the left")
    fixed.pan(5000f, 0f)
    near(fixed.rect, 0.0, 0.0, 0.84375, 1.0, "the photo stops at its edge")
    fixed.photoReady(2000, 3000)
    fixed.zoom(2f, PointF(200f, 400f))
    near(fixed.rect, 0.2890625, 0.25, 0.421875, 0.5, "pinch in around the middle")
    fixed.zoom(100f, PointF(200f, 400f))
    assertTrue(kotlin.math.abs(fixed.rect.width - 0.84375 / 3) < 1e-9)

    val free = MediaCrop(initial = RectF(0.2f, 0.2f, 0.7f, 0.7f)) { }
    free.photoReady(2000, 1000)
    near(free.rect, 0.2, 0.2, 0.5, 0.5, "free start")
    free.drag(CropHandle.RIGHT, 40f, 0f)
    near(free.rect, 0.2, 0.2, 0.6, 0.5, "handle")
    free.pan(40f, 20f)
    near(free.rect, 0.3, 0.3, 0.6, 0.5, "the free frame follows the finger")
    assertEquals(CropHandle.RIGHT, free.handleAt(PointF(400f * 0.3f + 400f * 0.6f, 300f + 200f * 0.55f), 12f))
    assertNull(free.handleAt(PointF(200f, 400f), 12f))
    assertNull("a fixed frame has no handles", fixed.handleAt(PointF(0f, 0f), 12f))
  }

  @Test
  fun inv_crop_12_cancelGivesNullOnceAndACommandAfterTheEndIsIgnored() {
    val calls = CopyOnWriteArrayList<MediaCropResult?>()
    val crop = MediaCrop(chat) { calls.add(it) }
    assertNull("not ready", crop.confirm())
    crop.photoReady(2000, 3000)
    crop.cancel()
    crop.cancel()
    assertNull(crop.confirm())
    crop.pan(10f, 0f)
    near(crop.rect, 0.078125, 0.0, 0.84375, 1.0, "no change after the end")
    assertEquals(listOf<MediaCropResult?>(null), calls.toList())

    val done = CopyOnWriteArrayList<MediaCropResult?>()
    val confirmed = MediaCrop(chat) { done.add(it) }
    confirmed.photoReady(2000, 3000)
    val result = confirmed.confirm()!!
    assertEquals(0, result.rotation)
    assertTrue(kotlin.math.abs(result.crop.left - 0.078125f) < 1e-5f && kotlin.math.abs(result.crop.width() - 0.84375f) < 1e-5f && result.crop.height() == 1f)
    confirmed.cancel()
    assertNull(confirmed.confirm())
    assertEquals(1, done.size)
    assertTrue(done[0] === result)
  }

  @Test
  fun inv_crop_13_aTurnChangesTheRotationAndTheResultIsAPlanTheCoreAccepts() {
    val crop = MediaCrop(chat) { }
    crop.photoReady(2000, 3000)
    crop.turnRight()
    assertEquals(90, crop.rotation)
    val result = crop.confirm()!!
    assertEquals(90, result.rotation)
    val plan = photoPlan(
      2000u, 3000u, 1u,
      PhotoEdit(null, false, CropFractions(result.crop.left.toDouble(), result.crop.top.toDouble(), result.crop.width().toDouble(), result.crop.height().toDouble()), PixelSize(1080u, 1920u), false, result.rotation.toUInt())
    )!!
    assertEquals(PixelSize(3000u, 2000u), plan.upright)
    assertTrue("${plan.crop}", kotlin.math.abs(plan.crop.width.toInt() * 1920 - plan.crop.height.toInt() * 1080) <= 1080 + 1920)

    val back = MediaCrop(chat) { }
    back.photoReady(2000, 3000)
    back.turnLeft()
    assertEquals(270, back.rotation)
    back.turnLeft()
    assertEquals(180, back.rotation)
    back.reset()
    assertEquals(0, back.rotation)

    val still = MediaCrop(chat, allowsRotation = false) { }
    still.photoReady(2000, 3000)
    still.turnRight()
    assertEquals(0, still.rotation)
  }

  @Test
  fun inv_crop_18_confirmGivesTheTelemetryOfTheGesturesAgainstTheFirstFrame() {
    fun confirmed(aspect: Size? = chat, initial: RectF? = null, width: Int = 2000, height: Int = 3000, moves: (MediaCrop) -> Unit): MediaCropResult {
      val crop = MediaCrop(aspect, initial) { }
      crop.photoReady(width, height)
      moves(crop)
      return crop.confirm()!!
    }
    val middle = PointF(200f, 400f)
    val table = listOf(
      "no gesture" to confirmed { },
      "pan" to confirmed { it.pan(30f, 0f) },
      "pinch" to confirmed { it.zoom(2f, middle) },
      "pan and pinch" to confirmed { it.pan(30f, 0f); it.zoom(2f, middle) },
      "moved back" to confirmed { it.pan(30f, 0f); it.pan(-30f, 0f) },
      "turn only" to confirmed { it.turnRight() },
      "free frame" to confirmed(null, RectF(0.2f, 0.2f, 0.7f, 0.7f), 2000, 1000) { it.pan(40f, 20f) }
    )
    val expected = listOf(
      listOf(1.0, 0, 0, false), listOf(1.0, 30, 0, true), listOf(2.0, 0, 0, true), listOf(2.0, 60, 0, true),
      listOf(1.0, 0, 0, true), listOf(1.0, 0, 0, false), listOf(1.0, 40, 20, true)
    )
    for ((index, entry) in table.withIndex()) {
      val (name, result) = entry
      assertEquals("$name $result", expected[index], listOf(result.finalScale, result.finalTranslateX, result.finalTranslateY, result.wasPositionChanged))
    }
  }

  @Test
  fun inv_crop_19_aFinishedGestureCallsPositionChangeOnceWithItsKind() {
    val kinds = CopyOnWriteArrayList<CropGestureReport>()
    val fixed = MediaCrop(chat, onPositionChange = { kinds.add(it) }) { }
    fixed.photoReady(2000, 3000)
    var now = 0L
    fixed.clock = { now }
    val middle = PointF(200f, 400f)
    now = 1_000
    fixed.gesture(true)
    fixed.pan(10f, 0f)
    fixed.pan(10f, 0f)
    fixed.pan(-5f, 0f)
    assertTrue("the call comes when the gesture ends", kinds.isEmpty())
    now = 1_400
    fixed.gesture(false)
    assertEquals(listOf(CropGesture.PAN), kinds.map { it.kind })
    val pan = kinds[0]
    assertEquals("$pan", listOf(400UL, 20, 15, 0.0), listOf(pan.durationMs, pan.maxDistance, pan.translateChange, pan.scaleChange))
    assertEquals("$pan", listOf(1.0, 15, 0), listOf(pan.finalScale, pan.finalTranslateX, pan.finalTranslateY))
    now = 2_000
    fixed.gesture(true)
    fixed.pan(5f, 0f)
    fixed.zoom(2f, middle)
    now = 2_090
    fixed.gesture(false)
    val pinch = kinds[1]
    assertEquals("a pinch measures the distance in scale and counts from the frame where it began: $pinch", listOf(90UL, 1, 1.0, 2.0), listOf(pinch.durationMs, pinch.maxDistance, pinch.scaleChange, pinch.finalScale))
    assertEquals("a pinch with a pan is a pinch", listOf(CropGesture.PAN, CropGesture.PINCH), kinds.map { it.kind })
    fixed.gesture(true)
    fixed.gesture(false)
    fixed.gesture(true)
    fixed.zoom(1f, middle)
    fixed.pan(0f, 0f)
    fixed.gesture(false)
    assertEquals("a touch that moved nothing gives no call", listOf(CropGesture.PAN, CropGesture.PINCH), kinds.map { it.kind })

    val free = MediaCrop(onPositionChange = { kinds.add(it) }) { }
    free.photoReady(2000, 1000)
    free.gesture(true)
    free.drag(CropHandle.RIGHT, -40f, 0f)
    free.gesture(false)
    assertEquals("a handle drag is a pan", listOf(CropGesture.PAN, CropGesture.PINCH, CropGesture.PAN), kinds.map { it.kind })
  }

  @Test
  fun inv_crop_21_snapshotGivesTheTransformNowAndCancelGivesItToo() {
    val finished = CopyOnWriteArrayList<MediaCropResult?>()
    val crop = MediaCrop(chat) { finished.add(it) }
    assertNull("before the photo is ready there is nothing to give", crop.snapshot())
    crop.photoReady(2000, 3000)
    assertEquals(MediaCropSnapshot(1.0, 0, 0, false), crop.snapshot())
    crop.gesture(true)
    crop.pan(30f, 0f)
    assertEquals("inside a gesture", MediaCropSnapshot(1.0, 30, 0, true), crop.snapshot())
    crop.zoom(2f, PointF(200f, 400f))
    crop.gesture(false)
    val now = MediaCropSnapshot(2.0, 60, 0, true)
    assertEquals(now, crop.snapshot())
    assertTrue("the crop stays open", !crop.isFinished)
    assertEquals(now, crop.cancel())
    assertEquals(listOf<MediaCropResult?>(null), finished.toList())
    assertEquals("the transform is still readable after cancel", now, crop.snapshot())
    assertNull("a second cancel does nothing", crop.cancel())
    assertEquals(1, finished.size)
  }

  @Test
  fun inv_crop_14_thePhotoForTheScreenTakesTheMemoryBudgetAndGivesItBack() = runBlocking {
    val file = jpegFile(200, 100)
    val admission = Admission()
    val crop = MediaCrop(chat) { }

    val bitmap = crop.load(MediaSourceValue.file(file, Kind.IMAGE), 100, admission.admit)

    assertNotNull(bitmap)
    assertTrue(crop.isReady)
    assertEquals(1, admission.asked.size)
    assertTrue(admission.asked[0] > 0)
    assertEquals(1, admission.closed.get())
    assertEquals(PixelSize(100u, 50u), PixelSize(bitmap!!.width.toUInt(), bitmap.height.toUInt()))

    val refused = Admission().apply { refuses = true }
    val blocked = MediaCrop { }
    assertNull(blocked.load(MediaSourceValue.file(file, Kind.IMAGE), 100, refused.admit))
    assertTrue(!blocked.isReady)
    assertNotNull(blocked.failure)
    assertEquals(0, refused.closed.get())

    val remote = MediaCrop { }
    assertNull(remote.load(MediaSourceValue.remote("https://example.com/a.jpg", Kind.IMAGE), 100, admission.admit))
    assertNotNull(remote.failure)
    assertEquals(1, admission.asked.size)
  }

  @Test
  fun inv_crop_15_aCropAtRestHoldsNoTaskOrTimerAndAGestureOnlyFlipsTheFlag() {
    val crop = MediaCrop(chat) { }
    crop.photoReady(2000, 3000)
    crop.gesture(true)
    crop.pan(5f, 5f)
    assertTrue(crop.isGesturing)
    crop.gesture(false)
    assertTrue(!crop.isGesturing)
    val before = crop.rect
    assertNotNull(crop.layout)
    assertEquals(before, crop.rect)
  }
}
