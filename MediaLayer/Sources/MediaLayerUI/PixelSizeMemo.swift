import Foundation
import MediaLayer
import ObjectiveC

nonisolated(unsafe) private var pixelSizesKey: UInt8 = 0

final class PixelSizeMemo: @unchecked Sendable {
  fileprivate struct Args: Hashable {
    let width: Double
    let height: Double
    let scale: Double
    let declaredWidth: UInt32?
    let declaredHeight: UInt32?
    let crop: Bool
  }

  fileprivate final class Source: NSObject {
    let url: String
    let headers: [String: String]

    init(url: String, headers: [String: String]) {
      self.url = url
      self.headers = headers
    }

    override var hash: Int {
      var hash = Hasher()
      hash.combine(url)
      hash.combine(headers)
      return hash.finalize()
    }

    override func isEqual(_ object: Any?) -> Bool {
      guard let other = object as? Source else { return false }
      return url == other.url && headers == other.headers
    }
  }

  fileprivate final class Input: NSObject {
    let source: Source
    let args: Args

    init(source: Source, args: Args) {
      self.source = source
      self.args = args
    }

    override var hash: Int {
      var hash = Hasher()
      hash.combine(source)
      hash.combine(args)
      return hash.finalize()
    }

    override func isEqual(_ object: Any?) -> Bool {
      guard let other = object as? Input else { return false }
      return source == other.source && args == other.args
    }
  }

  private final class Identity {
    weak var source: Source?
    let value: String

    init(source: Source, value: String) {
      self.source = source
      self.value = value
    }
  }

  final class Prepared: @unchecked Sendable {
    fileprivate let input: Input
    let identity: String
    let pixels: PixelSize?

    fileprivate init(input: Input, identity: String, pixels: PixelSize?) {
      self.input = input
      self.identity = identity
      self.pixels = pixels
    }
  }

  private static let capacity = 64
  private let compute: @Sendable (Double, Double, Double, PixelSize?, Bool) -> PixelSize?
  private let lock = NSLock()
  private var sizes: [Args: PixelSize?] = [:]
  private var order: [Args] = []
  private let requests = NSMapTable<Input, Prepared>(keyOptions: .weakMemory, valueOptions: .weakMemory)
  private let sources = NSMapTable<Source, Identity>(keyOptions: .weakMemory, valueOptions: .strongMemory)

  init(compute: @escaping @Sendable (Double, Double, Double, PixelSize?, Bool) -> PixelSize? = { imagePixelSize(width: $0, height: $1, scale: $2, declared: $3, crop: $4) }) {
    self.compute = compute
  }

  static func of(_ places: MediaPlaces) -> PixelSizeMemo {
    objc_sync_enter(places)
    defer { objc_sync_exit(places) }
    if let existing = objc_getAssociatedObject(places, &pixelSizesKey) as? PixelSizeMemo { return existing }
    let created = PixelSizeMemo()
    objc_setAssociatedObject(places, &pixelSizesKey, created, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
    return created
  }

  static func install(_ memo: PixelSizeMemo, on places: MediaPlaces) {
    objc_sync_enter(places)
    defer { objc_sync_exit(places) }
    objc_setAssociatedObject(places, &pixelSizesKey, memo, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
  }

  func cachedPreparation(url: String, headers: [String: String], width: Double, height: Double, scale: Double, declared: PixelSize?, crop: Bool) -> Prepared? {
    let input = Input(source: Source(url: url, headers: headers), args: Args(width: width, height: height, scale: scale, declaredWidth: declared?.width, declaredHeight: declared?.height, crop: crop))
    return lock.withLock { requests.object(forKey: input) }
  }

  func prepare(url: String, headers: [String: String], width: Double, height: Double, scale: Double, declared: PixelSize?, crop: Bool, readKey: () -> String) -> Prepared {
    if let hit = cachedPreparation(url: url, headers: headers, width: width, height: height, scale: scale, declared: declared, crop: crop) { return hit }
    let source = Source(url: url, headers: headers)
    let identity = lock.withLock { sources.object(forKey: source)?.value } ?? readKey()
    let pixels = size(width: width, height: height, scale: scale, declared: declared, crop: crop)
    return lock.withLock {
      let existingSource = sources.object(forKey: source)
      let input = Input(source: existingSource?.source ?? source, args: Args(width: width, height: height, scale: scale, declaredWidth: declared?.width, declaredHeight: declared?.height, crop: crop))
      if let existing = requests.object(forKey: input) { return existing }
      let value = Prepared(input: input, identity: existingSource?.value ?? identity, pixels: pixels)
      requests.setObject(value, forKey: input)
      sources.setObject(Identity(source: input.source, value: value.identity), forKey: input.source)
      return value
    }
  }

  func size(width: Double, height: Double, scale: Double, declared: PixelSize?, crop: Bool) -> PixelSize? {
    let args = Args(width: width, height: height, scale: scale, declaredWidth: declared?.width, declaredHeight: declared?.height, crop: crop)
    if let hit = lock.withLock({ sizes[args] }) { return hit }
    let size = compute(width, height, scale, declared, crop)
    lock.withLock {
      if sizes.updateValue(size, forKey: args) == nil {
        order.append(args)
        if order.count > Self.capacity { sizes[order.removeFirst()] = nil }
      }
    }
    return size
  }
}
