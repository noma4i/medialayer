const DEFAULT_LIMIT_MS: u64 = 600_000;
const LEAST_LIMIT_MS: u64 = 1_000;
const MOST_LIMIT_MS: u64 = 3_600_000;
const QUIET_DB: f32 = -50.0;

/// Why a voice recording did not start or gave nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VoiceRefusal {
    PermissionDenied,
    MicrophoneBusy,
    Empty,
    InCall,
}

/// How a voice recording is encoded, the same on both platforms: AAC in m4a.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct VoiceEncoderSettings {
    pub sample_rate_hz: u32,
    pub channels: u32,
    pub bit_rate: u32,
}

/// The one encoder setup of a voice recording: 44100 Hz, one channel, 128000 bit/s.
#[uniffi::export]
pub fn voice_encoder_settings() -> VoiceEncoderSettings {
    VoiceEncoderSettings { sample_rate_hz: 44_100, channels: 1, bit_rate: 128_000 }
}

/// The text of a refusal, the same on both platforms.
#[uniffi::export]
pub fn voice_refusal_text(reason: VoiceRefusal) -> String {
    match reason {
        VoiceRefusal::PermissionDenied => "the microphone permission is not granted",
        VoiceRefusal::MicrophoneBusy => "the microphone is used by another app or call",
        VoiceRefusal::Empty => "the recording holds no sound",
        VoiceRefusal::InCall => "a call is running, there is no recording",
    }
    .to_string()
}

/// The longest recording in milliseconds. An absent value gives 10 minutes, a value outside 1 second to 1 hour is brought into that range.
#[uniffi::export]
pub fn voice_limit_ms(requested: Option<u64>) -> u64 {
    requested.map_or(DEFAULT_LIMIT_MS, |value| value.clamp(LEAST_LIMIT_MS, MOST_LIMIT_MS))
}

/// The wave level 0.0 to 1.0 of a linear peak amplitude 0.0 to 1.0. Silence below -50 dB is 0.0, full scale is 1.0, a value that is not a number is 0.0.
#[uniffi::export]
pub fn voice_level(peak: f32) -> f32 {
    if peak.is_nan() || peak <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * peak.min(1.0).log10();
    ((db - QUIET_DB) / -QUIET_DB).clamp(0.0, 1.0)
}
