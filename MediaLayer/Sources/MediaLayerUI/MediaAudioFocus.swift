import MediaLayer

public enum MediaAudioFocus: Sendable, Equatable {
  case exclusive
  case mix
  case duck

  var core: AudioFocus {
    switch self {
    case .exclusive: .exclusive
    case .mix: .mix
    case .duck: .duck
    }
  }
}
