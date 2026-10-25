import Foundation

/// How a `MediaImage` plays an animated image (GIF, WebP) and what it shows while it rests.
public struct ImageAnimation: Equatable, Sendable {
  public enum Repeats: Equatable, Sendable {
    case forever
    case times(Int)
  }

  public enum RestFrame: Equatable, Sendable {
    case first
    case last
  }

  public var playing: Bool
  public var repeats: Repeats
  public var rest: RestFrame
  public var ignoresReduceMotion: Bool

  public init(playing: Bool = true, repeats: Repeats = .forever, rest: RestFrame = .first, ignoresReduceMotion: Bool = false) {
    self.playing = playing
    self.repeats = repeats
    self.rest = rest
    self.ignoresReduceMotion = ignoresReduceMotion
  }

  /// The animation rests on its rest frame: the caller stopped it, or the system asks to reduce motion.
  func rests(reduceMotion: Bool) -> Bool {
    !playing || (reduceMotion && !ignoresReduceMotion)
  }
}
