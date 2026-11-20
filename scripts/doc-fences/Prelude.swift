import Foundation
import SwiftUI
import UIKit
import AVFoundation
import Observation
import MediaLayer
import MediaLayerUI
import MediaLayerTesting

struct MomentVideo: MediaSource {
  let url: String
  let cover: String?
  let kind = Kind.video
  let headers: [String: String] = [:]
  init(url: String, cover: String? = nil) { self.url = url; self.cover = cover }
}
struct Avatar: MediaSource {
  let url: String
  let blurHash: String?
  let kind = Kind.image
  let headers: [String: String] = [:]
  init(url: String, blurHash: String? = nil) { self.url = url; self.blurHash = blurHash }
}
struct Poster: MediaSource {
  let url: String
  let kind = Kind.image
  let headers: [String: String] = [:]
}
typealias Upload = Poster
struct VoiceNote: MediaSource {
  let url: String
  let kind = Kind.audio
  let headers: [String: String] = [:]
}
typealias IntroClip = MomentVideo
typealias VoiceClip = MomentVideo
typealias MomentCircle = MomentVideo
struct Moment { var videoUrl: String; var coverUrl: String; var circleUrl: String; var posterUrl: String }
struct User { var avatarUrl: String; var blurHash: String? }
struct Message { var remoteUrl: String; var localPreview: String?; var audioUrl: String }
struct Clip { var url: String }
struct Recording { var fileUrl: URL }
struct Row: Sendable { var avatarUrl: String; var url: String; var video: MediaItem?; var photos: [MediaItem] }
@Observable final class Feed { var rows: [Row] = [] }
struct Bubble: View { let row: Row; init(_ row: Row) { self.row = row }; var body: some View { EmptyView() } }
final class RTCAudioSession {
  static func sharedInstance() -> RTCAudioSession { RTCAudioSession() }
  var isAudioEnabled = false
  func audioSessionDidActivate(_ session: AVAudioSession) {}
  func audioSessionDidDeactivate(_ session: AVAudioSession) {}
}
@MainActor var media: MediaLayer { fatalError() }
@MainActor var source: MediaSourceValue { fatalError() }
@MainActor var moment: Moment { fatalError() }
@MainActor var user: User { fatalError() }
@MainActor var message: Message { fatalError() }
@MainActor var clip: Clip { fatalError() }
@MainActor var recording: Recording { fatalError() }
@MainActor var entry: Row { fatalError() }
@MainActor var feed: Feed { fatalError() }
@MainActor var thread: Feed { fatalError() }
@MainActor var rows: [Row] { thread.rows }
@MainActor var rowIndex = 0
@MainActor var isActive: Bool { false }
@MainActor var isShareSheetOpen: Bool { false }
@MainActor var url: String { "" }
@MainActor var avatarUrl: String { "" }
@MainActor var avatarURL: URL { fatalError() }
@MainActor var introUrl: String { "" }
@MainActor var hash: String { "" }
@MainActor var blurHash: String { "" }
@MainActor var specJson: String { "" }
@MainActor var fileURL: URL { fatalError() }
@MainActor var clipUrl: URL { fatalError() }
@MainActor var cacheDirectory: URL { fatalError() }
@MainActor var webpBytes: Data { Data() }
@MainActor var bytes: Data { Data() }
@MainActor var uiImage: UIImage { fatalError() }
@MainActor var item: MediaItem { fatalError() }
@MainActor var introItem: MediaItem { fatalError() }
@MainActor var video: MomentVideo { fatalError() }
@MainActor var newItems: [MediaItem] { [] }
@MainActor var callUUID: UUID { UUID() }
@MainActor var fetcher: Fetcher { fatalError() }
@MainActor var settings: SettingsStore { fatalError() }
@MainActor var context: MediaContext { fatalError() }
@MainActor var audio: AudioPlayback { fatalError() }
@MainActor var call: MediaCallAudio { fatalError() }
@MainActor var preparer: MediaPreparer { fatalError() }
@MainActor var path: String { fatalError() }
@MainActor var output: String { fatalError() }
@MainActor var sourcePath: String { fatalError() }
@MainActor var destinationPath: String { fatalError() }
@MainActor var photo: PreparedMedia? = nil
@MainActor var ring: Double = 0
@MainActor var recordEnabled = false
@MainActor var cacheBytes: UInt64 = 0
@MainActor func dismiss() {}
@MainActor func draw<T>(_ value: T) {}
@MainActor func show<T>(_ value: T) {}
@MainActor func showPlaying(_ value: Bool) {}
@MainActor func hideSplash() {}
@MainActor func showHome() {}
struct RootView: View { var body: some View { EmptyView() } }
struct ChatScreen: View { var body: some View { EmptyView() } }
struct RetryButton: View { var body: some View { EmptyView() } }
enum Sentry { static func capture(message: String) {} }
