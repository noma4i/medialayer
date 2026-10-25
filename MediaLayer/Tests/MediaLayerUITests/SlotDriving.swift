import Foundation
import MediaLayer
@testable import MediaLayerUI

extension AVQueuePlayerSlotFactory {
  func makeSlot(_ id: UInt32) async -> AVQueuePlayerSlot {
    await create(slot: id).player as! AVQueuePlayerSlot
  }
}

extension AVQueuePlayerSlot {
  func load(key: String, route: PlaybackRoute, generation: UInt64 = 1) async {
    await apply(command: .load(key: key, route: route, generation: generation))
  }
  func unload() async { await apply(command: .unload) }
  func play() async { await apply(command: .play) }
  func pause() async { await apply(command: .pause) }
  func mute() async { await apply(command: .mute) }
  func unmute() async { await apply(command: .unmute) }
  func detach() async { await apply(command: .detach) }
  func attach(viewToken: UInt64) async { await apply(command: .attach(viewToken: viewToken)) }
  func seek(positionMs: UInt64) async { await apply(command: .seek(positionMs: positionMs)) }
  func setRate(rate: Float) async { await apply(command: .rate(rate: rate)) }
  func setReadahead(ms: UInt64) async { await apply(command: .readahead(ms: ms)) }
}
