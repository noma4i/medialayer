#if os(iOS)
import AVFoundation
import CoreGraphics
import CoreVideo
import Foundation
import MediaLayer

/// A frame of the camera for analysis in the app. `orientation` tells how the buffer is turned against an upright picture.
public struct CameraFrame: @unchecked Sendable {
  public let pixelBuffer: CVPixelBuffer
  public let orientation: CGImagePropertyOrientation
  public let lens: CameraLens
}

/// Stops the delivery of frames. The delivery also stops when the camera is released.
public final class CameraFrameObservation: @unchecked Sendable {
  private let stop: @Sendable () -> Void

  init(stop: @escaping @Sendable () -> Void) {
    self.stop = stop
  }

  public func cancel() {
    stop()
  }

  deinit {
    stop()
  }
}

/// The hardware side of a camera. Every method returns at once: the answer of the hardware comes back as an event to the sink.
/// `bind` ends with `opened` and `streaming`, or with `refuse`. A finished or failed recording ends with `recordFinished`.
protocol CaptureBackend: AnyObject, Sendable {
  var session: AVCaptureSession { get }
  var photoSize: PixelSize? { get }
  func setSink(_ sink: @escaping @Sendable (CaptureEvent) -> Void)
  func bind(lens: CameraLens, profile: CaptureProfile)
  func unbind()
  func unbind(completion: @escaping @Sendable () -> Void)
  func attachPreviewLayer(_ layer: AVCaptureVideoPreviewLayer)
  func detachPreviewLayer(_ layer: AVCaptureVideoPreviewLayer)
  func takePhoto(flash: Bool, profile: CaptureProfile) async throws -> Data
  func beginRecording(to url: URL, profile: CaptureProfile, lens: CameraLens)
  func beginRecording(to url: URL, profile: CaptureProfile, lens: CameraLens, ticket: RecordingTicket?, refused: @escaping @Sendable (RecordingRefusal) -> Void)
  func endRecording()
  func recordedMs() -> UInt64
  func setTorch(_ on: Bool) throws
  func setZoom(_ ratio: Float) throws
  func setFrameTap(_ tap: (@Sendable (CVPixelBuffer, UInt64) -> Void)?)
}

extension CaptureBackend {
  func unbind(completion: @escaping @Sendable () -> Void) { unbind(); completion() }

  func beginRecording(to url: URL, profile: CaptureProfile, lens: CameraLens, ticket: RecordingTicket?, refused: @escaping @Sendable (RecordingRefusal) -> Void) {
    do {
      if let ticket { try ticket.start { beginRecording(to: url, profile: profile, lens: lens) } }
      else { beginRecording(to: url, profile: profile, lens: lens) }
    } catch let reason as RecordingRefusal { refused(reason) }
    catch { refused(.interrupted) }
  }
}

/// What a camera takes from its surroundings, so a test can give it a fake of each.
struct CameraServices {
  typealias Release = @Sendable () -> Void

  var admitSession: @Sendable (UInt64) async throws -> Release
  var admitShot: @Sendable (UInt64) async throws -> Release
  var link: @Sendable () -> AudioLink?
  var clipInfo: @Sendable (URL) async throws -> MediaInfo?
  var upright: @Sendable (URL, Double) async throws -> URL
  var directory: URL
  var center: NotificationCenter
  var signals: SystemSignalNames
  var clock: @Sendable () -> UInt64

  static var live: CameraServices {
    CameraServices(
      admitSession: { bytes in
        guard let layer = Media.current as? MediaLayer else { throw MediaError.Closed }
        let hold = try await layer.reserveCapture(bytes: bytes)
        return { hold.release() }
      },
      admitShot: { bytes in
        guard let layer = Media.current as? MediaLayer else { throw MediaError.Closed }
        let hold = try await layer.reserveDecode(bytes: bytes)
        return { hold.release() }
      },
      link: { Media.link },
      clipInfo: { url in
        guard let layer = Media.current as? MediaLayer else { return nil }
        return try await layer.mediaInfo(MediaSourceValue.file(url, kind: .video))
      },
      upright: { url, quality in
        guard let layer = Media.current as? MediaLayer else { throw MediaError.Closed }
        let bytes = try Data(contentsOf: url, options: .mappedIfSafe)
        let jpeg = try await layer.encodeImage(bytes: bytes, format: .jpeg, quality: quality)
        let output = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).jpg")
        do {
          try jpeg.write(to: output, options: .atomic)
        } catch {
          NativeFailures.removeFile(output)
          throw error
        }
        return output
      },
      directory: CaptureFiles.directory,
      center: .default,
      signals: .system,
      clock: { DispatchTime.now().uptimeNanoseconds / 1_000_000 }
    )
  }
}

/// The capture directory in the cache: the files of captures that the app has not taken yet.
enum CaptureFiles {
  static var directory: URL {
    let base = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first ?? FileManager.default.temporaryDirectory
    return base.appendingPathComponent("capture", isDirectory: true)
  }

  static func name(_ kind: CaptureFileKind, in directory: URL) -> URL {
    try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return directory.appendingPathComponent(captureFileName(kind: kind, id: UUID().uuidString))
  }

  /// Removes the files a capture of an earlier run left behind. Runs when the layer opens, when no camera of this run holds a file yet.
  static func sweepOrphans(in directory: URL = CaptureFiles.directory) {
    let names = (try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? []
    for name in names where isCaptureOrphan(name: name) {
      NativeFailures.removeFile(directory.appendingPathComponent(name))
    }
  }
}
#endif
