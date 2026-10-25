import Foundation
import MediaLayer
#if canImport(UIKit)
import AVFoundation
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

public struct SystemSignalNames {
  public var memoryWarning: Notification.Name
  public var background: Notification.Name
  public var foreground: Notification.Name
  public var outputLost: Notification.Name

  public init(memoryWarning: Notification.Name, background: Notification.Name, foreground: Notification.Name, outputLost: Notification.Name) {
    self.memoryWarning = memoryWarning
    self.background = background
    self.foreground = foreground
    self.outputLost = outputLost
  }

  #if canImport(UIKit)
  public static let system = SystemSignalNames(
    memoryWarning: UIApplication.didReceiveMemoryWarningNotification,
    background: UIApplication.didEnterBackgroundNotification,
    foreground: UIApplication.willEnterForegroundNotification,
    outputLost: AVAudioSession.routeChangeNotification
  )
  #else
  public static let system = SystemSignalNames(
    memoryWarning: Notification.Name("MediaLayerMemoryWarning"),
    background: NSApplication.didResignActiveNotification,
    foreground: NSApplication.didBecomeActiveNotification,
    outputLost: Notification.Name("MediaLayerOutputLost")
  )
  #endif
}

final class SystemSignals {
  private let center: NotificationCenter
  private var tokens: [NSObjectProtocol] = []
  private var onRelease: () -> Void

  init(events: PlayerEventsProtocol, center: NotificationCenter, names: SystemSignalNames, onOutputLost: @escaping () -> Void = { try? Media.setMuted(true) }, onRelease: @escaping () -> Void = {}) {
    self.center = center
    self.onRelease = onRelease
    tokens = [
      center.addObserver(forName: names.memoryWarning, object: nil, queue: nil) { _ in events.memoryWarning(level: .critical) },
      center.addObserver(forName: names.background, object: nil, queue: nil) { _ in events.background() },
      center.addObserver(forName: names.foreground, object: nil, queue: nil) { _ in events.foreground() },
      center.addObserver(forName: names.outputLost, object: nil, queue: nil) { note in if Self.isOutputLost(note) { onOutputLost() } }
    ]
  }

  deinit {
    stop()
  }

  static func isOutputLost(_ note: Notification) -> Bool {
    #if canImport(UIKit)
    let reason = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt
    return reason == AVAudioSession.RouteChangeReason.oldDeviceUnavailable.rawValue
    #else
    return true
    #endif
  }

  func stop() {
    tokens.forEach { center.removeObserver($0) }
    tokens = []
    let release = onRelease
    onRelease = {}
    release()
  }
}
