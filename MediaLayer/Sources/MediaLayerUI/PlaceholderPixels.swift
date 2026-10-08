import CoreGraphics
import MediaLayer
import Foundation
import SwiftUI
#if canImport(UIKit)
import UIKit
public typealias PlatformImage = UIImage
#else
import AppKit
public typealias PlatformImage = NSImage
#endif

public struct PlaceholderPixels: Sendable {
  public let width: Int
  public let height: Int
  public let rgba: Data

  public init(width: Int, height: Int, rgba: Data) {
    self.width = width
    self.height = height
    self.rgba = rgba
  }

  public static func blurHash(_ hash: String) -> PlaceholderPixels? {
    guard let rgba = try? decodeBlurhash(hash: hash, width: side, height: side, punch: 1) else { return nil }
    return PlaceholderPixels(width: Int(side), height: Int(side), rgba: rgba)
  }

  private static let side: UInt32 = 32

  public var cgImage: CGImage? {
    guard width > 0, height > 0, rgba.count == width * height * 4, let provider = CGDataProvider(data: rgba as CFData) else { return nil }
    return CGImage(
      width: width,
      height: height,
      bitsPerComponent: 8,
      bitsPerPixel: 32,
      bytesPerRow: width * 4,
      space: CGColorSpaceCreateDeviceRGB(),
      bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
      provider: provider,
      decode: nil,
      shouldInterpolate: true,
      intent: .defaultIntent
    )
  }
}

extension Image {
  init(platform image: PlatformImage) {
    #if canImport(UIKit)
    self.init(uiImage: image)
    #else
    self.init(nsImage: image)
    #endif
  }
}
