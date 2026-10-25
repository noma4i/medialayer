mod photo;
mod plan;

pub use photo::{jpeg_quality, photo_turn, PhotoTurn};
pub use plan::{default_video_limits, plan_video, BitrateStep, VideoLimits, VideoPlan, VideoSource};

#[cfg(test)]
mod send_tests;
