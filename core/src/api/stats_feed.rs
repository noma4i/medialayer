use super::controls::Controls;
use super::guard::AbortOnDrop;
use crate::change::Change;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::runtime::Handle;

const MERGE_WINDOW: Duration = Duration::from_millis(250);

pub fn follow_stats(handle: &Handle, controls: Weak<Controls>, change: Arc<Change>) -> AbortOnDrop {
    AbortOnDrop::new(handle.spawn(async move {
        loop {
            change.changed().await;
            let Some(controls) = controls.upgrade() else { return };
            controls.count_wake();
            controls.publish_stats().await;
            if !controls.has_stats_listener() {
                change.release();
                if controls.has_stats_listener() {
                    change.want();
                }
            }
            drop(controls);
            tokio::time::sleep(MERGE_WINDOW).await;
        }
    }))
}
