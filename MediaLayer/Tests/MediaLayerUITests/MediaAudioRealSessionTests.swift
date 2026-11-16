@preconcurrency import AVFoundation
import Testing
@testable import MediaLayerUI

extension GlobalMedia {
  @Suite(.serialized)
  struct MediaAudioRealSessionTests {
    @Test
    func theRealSessionReceivesTheCategoryAndOptionsOfEachPolicy() async throws {
      let real = AVAudioSession.sharedInstance()
      for (focus, mix, duck) in [(MediaAudioFocus.exclusive, false, false), (.mix, true, false), (.duck, true, true)] {
        let session = MediaAudioSession(focus: focus)
        session.setAudible(slot: 1, true)
        #expect(await session.settled())

        #expect(real.category == .playback)
        #expect(real.mode == .moviePlayback)
        #expect(real.categoryOptions.contains(.mixWithOthers) == mix)
        #expect(real.categoryOptions.contains(.duckOthers) == duck)

        session.setAudible(slot: 1, false)
        #expect(await session.settled())
      }
    }

    @Test
    func anIdleSessionLeavesTheCategoryOfTheAppAlone() async throws {
      let real = AVAudioSession.sharedInstance()
      try real.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker])

      let session = MediaAudioSession(focus: .duck)
      session.setAudible(slot: 1, false)
      #expect(await session.settled())

      #expect(real.category == .playAndRecord)
      #expect(real.categoryOptions == [.defaultToSpeaker])
    }

    @Test
    func recordingSwitchesTheRealSessionToTheMicrophoneCategory() async throws {
      let real = AVAudioSession.sharedInstance()
      let session = MediaAudioSession(focus: .mix)

      let hold = await session.holdRecording()
      #expect(hold != nil)
      #expect(real.category == .playAndRecord)
      #expect(real.mode == .default)
      #expect(real.categoryOptions.contains(.defaultToSpeaker))
      #expect(!real.categoryOptions.contains(.mixWithOthers))

      hold?.release()
      session.setAudible(slot: 1, true)
      #expect(await session.settled())
      #expect(real.category == .playback)
      #expect(real.categoryOptions.contains(.mixWithOthers))
      session.setAudible(slot: 1, false)
      #expect(await session.settled())
    }
  }
}
