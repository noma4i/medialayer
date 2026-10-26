#if os(iOS)
import CoreGraphics
import Foundation
import MediaLayer
import Testing
import UIKit
@testable import MediaLayerUI

@MainActor
private final class Outcome {
  var calls: [MediaCropResult?] = []
}

private final class Admission: @unchecked Sendable {
  private let lock = NSLock()
  private var items: [UInt64] = []
  private var releases = 0
  var refuses = false
  var asked: [UInt64] { lock.withLock { items } }
  var released: Int { lock.withLock { releases } }
  var admit: DecodeAdmit {
    { bytes in
      self.lock.withLock { self.items.append(bytes) }
      if self.refuses { throw MediaError.Overloaded(detail: "no room") }
      return { self.lock.withLock { self.releases += 1 } }
    }
  }
}

private func chat() -> CGSize { CGSize(width: 1080, height: 1920) }

@MainActor
private func ready(_ crop: MediaCrop, photo: PixelSize, view: CGSize = CGSize(width: 400, height: 800)) {
  crop.place(in: view, inset: 0)
  crop.ready(photo: photo)
}

private func near(_ actual: CropFractions, _ x: Double, _ y: Double, _ width: Double, _ height: Double, _ name: String = "") {
  #expect(abs(actual.x - x) < 1e-6 && abs(actual.y - y) < 1e-6 && abs(actual.width - width) < 1e-6 && abs(actual.height - height) < 1e-6, "\(name) \(actual)")
}

private func jpegFile(_ width: Int, _ height: Int) throws -> URL {
  let renderer = UIGraphicsImageRenderer(size: CGSize(width: width, height: height), format: { let format = UIGraphicsImageRendererFormat(); format.scale = 1; return format }())
  let data = renderer.jpegData(withCompressionQuality: 0.9) { context in
    UIColor.red.setFill()
    context.fill(CGRect(x: 0, y: 0, width: width, height: height))
  }
  let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).jpg")
  try data.write(to: url)
  return url
}

@MainActor
@Suite(.serialized, .timeLimit(.minutes(1)))
struct MediaCropTests {
  @Test
  func inv_crop_11_gestures_in_the_model_give_the_rectangle_of_the_table() {
    let fixed = MediaCrop(aspect: chat()) { _ in }
    ready(fixed, photo: PixelSize(width: 2000, height: 3000))
    near(fixed.rect, 0.078125, 0, 0.84375, 1, "first frame")
    fixed.pan(dx: -23.7037037, dy: 30)
    near(fixed.rect, 0.128125, 0, 0.84375, 1, "the photo follows the finger to the left")
    fixed.pan(dx: 5000, dy: 0)
    near(fixed.rect, 0, 0, 0.84375, 1, "the photo stops at its edge")
    ready(fixed, photo: PixelSize(width: 2000, height: 3000))
    fixed.zoom(by: 2, at: CGPoint(x: 200, y: 400))
    near(fixed.rect, 0.2890625, 0.25, 0.421875, 0.5, "pinch in around the middle")
    fixed.zoom(by: 100, at: CGPoint(x: 200, y: 400))
    #expect(abs(fixed.rect.width - 0.84375 / 3) < 1e-9, "the smallest frame is a third")

    let free = MediaCrop(initial: CGRect(x: 0.2, y: 0.2, width: 0.5, height: 0.5)) { _ in }
    ready(free, photo: PixelSize(width: 2000, height: 1000))
    near(free.rect, 0.2, 0.2, 0.5, 0.5, "free start")
    free.drag(.right, dx: 40, dy: 0)
    near(free.rect, 0.2, 0.2, 0.6, 0.5, "handle")
    free.pan(dx: 40, dy: 20)
    near(free.rect, 0.3, 0.3, 0.6, 0.5, "the free frame follows the finger")
    #expect(free.handle(at: CGPoint(x: 400 * 0.3 + 400 * 0.6, y: 300 + 200 * 0.55), slop: 12) == .right)
    #expect(free.handle(at: CGPoint(x: 200, y: 400), slop: 12) == nil)
    #expect(fixed.handle(at: CGPoint(x: 0, y: 0), slop: 12) == nil, "a fixed frame has no handles")
  }

  @Test
  func inv_crop_12_cancel_gives_nil_once_and_a_command_after_the_end_is_ignored() {
    let outcome = Outcome()
    let crop = MediaCrop(aspect: chat()) { outcome.calls.append($0) }
    #expect(crop.confirm() == nil, "not ready")
    ready(crop, photo: PixelSize(width: 2000, height: 3000))
    crop.cancel()
    crop.cancel()
    #expect(crop.confirm() == nil)
    crop.pan(dx: 10, dy: 0)
    near(crop.rect, 0.078125, 0, 0.84375, 1, "no change after the end")
    #expect(outcome.calls.count == 1 && outcome.calls[0] == nil)

    let done = Outcome()
    let confirmed = MediaCrop(aspect: chat()) { done.calls.append($0) }
    ready(confirmed, photo: PixelSize(width: 2000, height: 3000))
    let result = confirmed.confirm()
    #expect(result?.rotation == 0)
    #expect(result.map { abs($0.crop.minX - 0.078125) < 1e-6 && abs($0.crop.width - 0.84375) < 1e-6 && $0.crop.height == 1 } == true)
    confirmed.cancel()
    #expect(confirmed.confirm() == nil)
    #expect(done.calls.count == 1 && done.calls[0] == result)
  }

  @Test
  func inv_crop_13_a_turn_changes_the_rotation_and_the_result_is_a_plan_the_core_accepts() throws {
    let crop = MediaCrop(aspect: chat()) { _ in }
    ready(crop, photo: PixelSize(width: 2000, height: 3000))
    crop.turnRight()
    #expect(crop.rotation == 90)
    #expect(crop.photo == PixelSize(width: 3000, height: 2000))
    let result = try #require(crop.confirm())
    #expect(result.rotation == 90)
    let plan = try #require(photoPlan(width: 2000, height: 3000, exifOrientation: 1, edit: PhotoEdit(
      maxSize: nil, enlarge: false,
      crop: CropFractions(x: result.crop.minX, y: result.crop.minY, width: result.crop.width, height: result.crop.height),
      outputSize: PixelSize(width: 1080, height: 1920), mirror: false, rotationDegrees: UInt32(result.rotation)
    )))
    #expect(plan.upright == PixelSize(width: 3000, height: 2000))
    let error = abs(Int(plan.crop.width) * 1920 - Int(plan.crop.height) * 1080)
    #expect(error <= 1080 + 1920, "\(plan.crop)")

    let back = MediaCrop(aspect: chat()) { _ in }
    ready(back, photo: PixelSize(width: 2000, height: 3000))
    back.turnLeft()
    #expect(back.rotation == 270)
    back.turnLeft()
    #expect(back.rotation == 180)
    back.reset()
    #expect(back.rotation == 0 && back.photo == PixelSize(width: 2000, height: 3000))

    let still = MediaCrop(aspect: chat(), allowsRotation: false) { _ in }
    ready(still, photo: PixelSize(width: 2000, height: 3000))
    still.turnRight()
    #expect(still.rotation == 0)
  }

  @Test
  func inv_crop_14_the_photo_for_the_screen_takes_the_memory_budget_and_gives_it_back() async throws {
    let file = try jpegFile(200, 100)
    defer { try? FileManager.default.removeItem(at: file) }
    let admission = Admission()
    let crop = MediaCrop(aspect: chat()) { _ in }

    let image = await crop.load(MediaSourceValue.file(file, kind: .image), maxSide: 100, admit: admission.admit)

    #expect(image != nil && crop.isReady)
    #expect(admission.asked.count == 1 && admission.asked[0] > 0)
    #expect(admission.released == 1)
    #expect(crop.photo == PixelSize(width: 100, height: 50))

    let refused = Admission()
    refused.refuses = true
    let blocked = MediaCrop { _ in }
    let none = await blocked.load(MediaSourceValue.file(file, kind: .image), maxSide: 100, admit: refused.admit)
    #expect(none == nil && !blocked.isReady && blocked.failure != nil)
    #expect(refused.released == 0)

    let remote = MediaCrop { _ in }
    _ = await remote.load(MediaSourceValue.remote("https://example.com/a.jpg", kind: .image), maxSide: 100, admit: admission.admit)
    #expect(!remote.isReady && remote.failure != nil)
    #expect(admission.asked.count == 1, "a remote source asks for no raster")
  }

  @Test
  func inv_crop_15_a_crop_at_rest_holds_no_task_or_timer_and_a_gesture_only_flips_the_flag() {
    let crop = MediaCrop(aspect: chat()) { _ in }
    ready(crop, photo: PixelSize(width: 2000, height: 3000))
    crop.gesture(true)
    crop.pan(dx: 5, dy: 5)
    #expect(crop.isGesturing)
    crop.gesture(false)
    #expect(!crop.isGesturing)
    let before = crop.rect
    #expect(crop.layout != nil)
    #expect(crop.rect == before, "reading the layout changes nothing")
  }
}
#endif
