#if os(iOS)
import Testing
@testable import MediaLayerUI

/// The session of `AVCaptureBackend` needs a camera, which the simulator lacks, so the decision that keeps its graph still during a take is tested on its own.
struct FrameOutputDecisionTests {
  @Test func inv_capture_81_video_camera_carries_the_frame_output_without_a_tap() {
    #expect(frameOutputNeeded(videoMode: true, hasTap: false))
    #expect(frameOutputNeeded(videoMode: true, hasTap: true))
  }

  @Test func inv_capture_81_photo_camera_carries_it_only_for_a_tap() {
    #expect(!frameOutputNeeded(videoMode: false, hasTap: false))
    #expect(frameOutputNeeded(videoMode: false, hasTap: true))
  }
}
#endif
