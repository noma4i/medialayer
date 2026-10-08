use super::controls::Controls;
use std::sync::Weak;
use std::time::Duration;
use tokio::runtime::Handle;
use super::guard::AbortOnDrop;

pub fn spawn_pulse(handle: &Handle, controls: Weak<Controls>, every: Duration) -> AbortOnDrop {
    AbortOnDrop::new(handle.spawn(async move {
        loop {
            tokio::time::sleep(every).await;
            let Some(controls) = controls.upgrade() else { return };
            controls.publish_stats().await;
        }
    }))
}
