import Foundation
import MediaLayer

struct PlaceUpdate: Sendable {
  var input: PlaceInput?
  var seek: Double?
  var state: MediaState?
  var closes = false

  static func merged(_ held: PlaceUpdate?, _ next: PlaceUpdate) -> PlaceUpdate {
    PlaceUpdate(
      input: next.input ?? held?.input,
      seek: next.seek ?? held?.seek,
      state: next.state ?? held?.state,
      closes: (held?.closes ?? false) || next.closes
    )
  }
}

private final class OpenedPlace: @unchecked Sendable {
  var place: (any PlaceHandle)?
  var isClosed = false
}

func mediaError(_ error: Error) -> MediaError {
  (error as? MediaError) ?? .Unexpected(detail: String(describing: error))
}

/// One mounted place. Calls leave the caller thread through one latest-wins drain; a refused call reaches `failed`.
final class PlaceMount: @unchecked Sendable {
  private let drain: LatestDrain<PlaceUpdate>

  init(places: MediaPlaces, item: MediaItem, failed: @escaping @Sendable (MediaError) -> Void) {
    let opened = OpenedPlace()
    drain = LatestDrain(merge: { PlaceUpdate.merged($0, $1) }) { update in
      guard !opened.isClosed else { return }
      if update.closes {
        opened.isClosed = true
        if let fraction = update.seek {
          do { try opened.place?.seek(fraction: fraction) } catch { failed(mediaError(error)) }
        }
        opened.place?.release()
        opened.place = nil
        return
      }
      do {
        if opened.place == nil { opened.place = try places.mountPlace(item: item) }
        guard let place = opened.place else { return }
        if let input = update.input { try place.update(input: input) }
        if let fraction = update.seek { try place.seek(fraction: fraction) }
        if let state = update.state { try place.itemState(state: state) }
      } catch {
        failed(mediaError(error))
      }
    }
  }

  func update(_ input: PlaceInput) { drain.submit(PlaceUpdate(input: input)) }

  func seek(_ fraction: Double) { drain.submit(PlaceUpdate(seek: fraction)) }

  func report(_ state: MediaState) { drain.submit(PlaceUpdate(state: state)) }

  func close() { drain.submit(PlaceUpdate(closes: true)) }

  deinit { close() }
}

extension PlaceInput {
  static func make(
    visible: Bool = false, playing: Bool = false, group: String? = nil, peeking: Bool = false,
    muted: Bool? = nil, rate: Float? = nil, pinned: Bool = false, viewToken: UInt64? = nil
  ) -> PlaceInput {
    PlaceInput(visible: visible, playing: playing, group: group, peeking: peeking, muted: muted, rate: rate, pinned: pinned, viewToken: viewToken)
  }
}
