import MediaLayer
import MediaLayerUI
import SwiftUI

/// The text of the last report, shown on the phone so a person can read it next to the circle camera.
@MainActor
final class ProbeScreen: ObservableObject {
  static let shared = ProbeScreen()
  @Published var camera: MediaCamera?
  @Published var report = "MediaLayerProbe: waiting for the tests"
}

@main
struct ProbeApp: App {
  /// The app opens the layer at launch, as docs/getting-started.md says; `openMs` is the time it took.
  nonisolated(unsafe) static var openMs = 0
  static let opening = Task { () -> MediaLayer in
    let started = Date()
    let layer = try await MediaLayer.open()
    openMs = Int(Date().timeIntervalSince(started) * 1000)
    return layer
  }

  var body: some Scene {
    WindowGroup { ProbeView().mediaLayer(opening: Self.opening) }
  }
}

struct ProbeView: View {
  @ObservedObject private var screen = ProbeScreen.shared

  var body: some View {
    VStack(spacing: 16) {
      Group {
        if let camera = screen.camera {
          MediaCameraPreview(camera: camera)
        } else {
          Color.gray.opacity(0.2)
        }
      }
      .frame(width: 240, height: 240)
      .clipShape(Circle())
      Text(screen.report)
        .font(.system(size: 11, design: .monospaced))
        .frame(maxWidth: .infinity, alignment: .leading)
    }
    .padding()
  }
}
