import Foundation
import MediaLayer
import os

enum MediaTrace {
  static let subsystem = "com.noma4i.medialayer"

  /// `ML_TRACE` in the environment, or the `MLTrace` default (the launch argument `-MLTrace YES` sets it). Read once, at the first emit.
  static let isOn: Bool = {
    let on = enabled(environment: ProcessInfo.processInfo.environment, defaults: .standard)
    if on { setTraceSink(sink: OSLogSink()) }
    return on
  }()

  static func enabled(environment: [String: String], defaults: UserDefaults) -> Bool {
    environment["ML_TRACE"] != nil || defaults.bool(forKey: "MLTrace")
  }

  static func emit(_ source: String, _ message: @autoclosure () -> String) {
    emit(source, message(), on: isOn, write: write)
  }

  static func emit(_ source: String, _ message: @autoclosure () -> String, on: Bool, write: (String, String) -> Void) {
    guard on else { return }
    let milliseconds = Int(Date().timeIntervalSince1970 * 1000)
    let line = "ML \(milliseconds) \(source) \(message())"
    observer?(line)
    write(source, line)
  }

  /// Sees every emitted line when the trace is on. The device probe reads the library's own lines through it.
  nonisolated(unsafe) static var observer: ((String) -> Void)?

  /// A line to the unified log (device syslog, Console.app) and to stderr for a run under `ML_TRACE`.
  static func write(_ source: String, _ line: String) {
    Logger(subsystem: subsystem, category: source).notice("\(line, privacy: .public)")
    if ProcessInfo.processInfo.environment["ML_TRACE"] != nil {
      FileHandle.standardError.write(Data((line + "\n").utf8))
    }
  }

  static func tail(_ text: String) -> String {
    let path = text.prefix { $0 != "?" && $0 != "#" }
    return String(path.suffix(24))
  }

  /// Domain and code only: the description of an error can carry a url with its token.
  static func errorLabel(_ error: NSError?) -> String {
    error.map { "\($0.domain)#\($0.code)" } ?? "-"
  }

  /// The core prints its trace lines here, so a device without a stderr shows them next to the slot lines.
  private final class OSLogSink: TraceSink {
    private let logger = Logger(subsystem: MediaTrace.subsystem, category: "core")

    func line(text: String) {
      logger.notice("\(text, privacy: .public)")
    }
  }
}
