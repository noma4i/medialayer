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
  let audio: AudioLink?
  private let center: NotificationCenter
  private var tokens: [NSObjectProtocol] = []
  private var onRelease: () -> Void

  init(events: PlayerEventsProtocol, center: NotificationCenter, names: SystemSignalNames, audio: AudioLink? = nil, onRelease: @escaping () -> Void = {}) {
    self.center = center
    self.audio = audio
    self.onRelease = onRelease
    tokens = [
      center.addObserver(forName: names.memoryWarning, object: nil, queue: nil) { _ in events.memoryWarning(level: .critical) },
      center.addObserver(forName: names.background, object: nil, queue: nil) { _ in events.background(); audio?.foreground(false) },
      center.addObserver(forName: names.foreground, object: nil, queue: nil) { _ in events.foreground(); audio?.foreground(true) }
    ]
  }

  deinit {
    stop()
  }

  #if canImport(UIKit)
  static func isOutputLost(reason: UInt?, previous: [AVAudioSession.Port], current: [AVAudioSession.Port]) -> Bool {
    guard reason == AVAudioSession.RouteChangeReason.oldDeviceUnavailable.rawValue else { return false }
    let privateOutputs: Set<AVAudioSession.Port> = [
      .headphones, .bluetoothA2DP, .bluetoothHFP, .bluetoothLE, .usbAudio,
      .carAudio, .airPlay, .lineOut, .HDMI
    ]
    return previous.contains { privateOutputs.contains($0) && !current.contains($0) }
  }
  #endif

  func stop() {
    tokens.forEach { center.removeObserver($0) }
    tokens = []
    let release = onRelease
    onRelease = {}
    release()
  }
}
