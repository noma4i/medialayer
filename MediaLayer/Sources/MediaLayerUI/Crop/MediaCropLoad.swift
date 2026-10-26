#if os(iOS)
import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import UIKit

extension MediaCrop {
  /// Decodes the photo for the screen and tells the crop its size. The decode takes its raster from the memory budget before it allocates (`MediaImageDecode`).
  /// A source that is no local file, no image, or a refused budget is a `MediaError` in `failure`.
  func load(_ source: some MediaSource, maxSide: Int, admit: DecodeAdmit? = nil) async -> UIImage? {
    do {
      let file = try Self.localFile(source)
      let decode = try admit ?? Self.openedAdmit()
      let image = try await MediaImageDecode.decode(maxSide: maxSide, circle: false, admit: decode) { CGImageSourceCreateWithURL(file as CFURL, nil) }
      guard let cgImage = image.cgImage else { throw MediaError.Fetch(detail: "the source is not an image") }
      ready(photo: PixelSize(width: UInt32(clamping: cgImage.width), height: UInt32(clamping: cgImage.height)))
      return image
    } catch is CancellationError {
      return nil
    } catch {
      fail(error as? MediaError ?? MediaError.Unexpected(detail: "the photo cannot be shown"))
      return nil
    }
  }

  private static func localFile(_ source: some MediaSource) throws -> URL {
    let url = normalizeItem(item: source.item).url
    guard url.hasPrefix("file://"), let file = URL(string: url), file.isFileURL else { throw MediaError.Fetch(detail: "the source is not a local file") }
    return file
  }

  private static func openedAdmit() throws -> DecodeAdmit {
    guard let layer = Media.current as? MediaLayer else { throw MediaError.Closed }
    return layer.decodeAdmit
  }
}
#endif
