/// The smallest rise of the system volume, on a 0..1 scale, that counts as a press of volume up.
const VOLUME_RAISE_THRESHOLD: f32 = 0.001;

/// Whether the volume went up from one reading to the next by more than the threshold. Both platforms ask this one rule.
#[uniffi::export]
pub fn is_volume_raise(previous: f32, current: f32) -> bool {
    current - previous > VOLUME_RAISE_THRESHOLD
}

#[cfg(test)]
mod raise_tests;
