import Foundation

enum MediaTrace {
  static let isOn = ProcessInfo.processInfo.environment["ML_TRACE"] != nil

  static func emit(_ source: String, _ message: @autoclosure () -> String) {
    guard isOn else { return }
    let milliseconds = Int(Date().timeIntervalSince1970 * 1000)
    FileHandle.standardError.write(Data("ML \(milliseconds) \(source) \(message())\n".utf8))
  }

  static func tail(_ text: String) -> String {
    String(text.suffix(24))
  }
}
