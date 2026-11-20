//! The state machine of an outgoing media item: one pure fold from the inputs the app forwards (upload state, remote url)
//! and the platform's fetch result to the phase the UI draws. Both platforms only draw what this module decides.

/// The slots for fetching the remote copies of outgoing items at once; the rest queue (an album of 10 tiles works).
pub const OUTGOING_FETCH_SLOTS: u32 = 2;
/// How soon to ask the remote again while it answered "not ready yet" and the item is on screen.
const REASK_AFTER_MS: u32 = 5_000;
const STALE_UNREADY_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const STALE_UNDECLARED_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OutgoingKind {
    Image,
    Video,
}

/// The upload as DBlayer reports it. Every pre-upload status (waiting, preparing, queued, uploading, sending) is `Sending`.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum OutgoingUpload {
    Sending { progress: f64 },
    Sent,
    Failed { reason: String },
}

/// What the platform reports about fetching the remote copy. `Complete` means the whole body is cached AND decoded.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum RemoteResult {
    NotReady,
    Fetching { progress: f64 },
    Complete,
    Failed { status: Option<u16>, detail: String },
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum OutgoingFailure {
    Upload { reason: String },
    Remote { status: Option<u16>, detail: String },
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum OutgoingPhase {
    Sending { progress: f64 },
    AwaitingRemote,
    FetchingRemote { progress: f64 },
    Ready,
    Failed { reason: OutgoingFailure },
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct OutgoingInputs {
    pub key: String,
    pub kind: OutgoingKind,
    pub has_local: bool,
    pub blur_hash: Option<String>,
    pub remote: Option<String>,
    pub upload: OutgoingUpload,
    pub remote_result: Option<RemoteResult>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct OutgoingState {
    pub inputs: OutgoingInputs,
    pub phase: OutgoingPhase,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum OutgoingEvent {
    Upload { upload: OutgoingUpload },
    /// The remote url of the item. `Some` also means the upload is sent, unless it failed (DBlayer drops the upload entry
    /// and fills the server field when the upload completes). `None` never takes a known url back.
    Remote { remote: Option<String> },
    RemoteResult { result: RemoteResult },
    /// Only from `Failed(Remote)`: fetch the same url again.
    Retry,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct OutgoingView {
    pub phase: OutgoingPhase,
    pub shows_local: bool,
    pub shows_loader: bool,
    pub can_open: bool,
    pub reask_after_ms: Option<u32>,
}

/// The first state of an item, from what the app declares.
#[uniffi::export]
pub fn outgoing_declare(key: String, kind: OutgoingKind, has_local: bool, blur_hash: Option<String>) -> OutgoingState {
    let inputs = OutgoingInputs {
        key,
        kind,
        has_local,
        blur_hash,
        remote: None,
        upload: OutgoingUpload::Sending { progress: 0.0 },
        remote_result: None,
    };
    OutgoingState { phase: phase_of(&inputs), inputs }
}

fn clamp(progress: f64) -> f64 {
    if progress.is_nan() { 0.0 } else { progress.clamp(0.0, 1.0) }
}

fn phase_of(inputs: &OutgoingInputs) -> OutgoingPhase {
    if let Some(result) = &inputs.remote_result {
        match result {
            RemoteResult::Complete => return OutgoingPhase::Ready,
            RemoteResult::Failed { status, detail } => {
                return OutgoingPhase::Failed { reason: OutgoingFailure::Remote { status: *status, detail: detail.clone() } };
            }
            _ => {}
        }
    }
    if let OutgoingUpload::Failed { reason } = &inputs.upload {
        return OutgoingPhase::Failed { reason: OutgoingFailure::Upload { reason: reason.clone() } };
    }
    if inputs.remote.is_some() {
        let progress = match &inputs.remote_result {
            Some(RemoteResult::Fetching { progress }) => *progress,
            _ => 0.0,
        };
        return OutgoingPhase::FetchingRemote { progress };
    }
    match &inputs.upload {
        OutgoingUpload::Sending { progress } => OutgoingPhase::Sending { progress: *progress },
        _ => OutgoingPhase::AwaitingRemote,
    }
}

fn is_ready(inputs: &OutgoingInputs) -> bool {
    matches!(inputs.remote_result, Some(RemoteResult::Complete))
}

fn apply_upload(inputs: &mut OutgoingInputs, upload: OutgoingUpload) {
    if is_ready(inputs) {
        return;
    }
    match upload {
        OutgoingUpload::Sending { progress } => {
            let progress = clamp(progress);
            match &inputs.upload {
                OutgoingUpload::Sent => {}
                OutgoingUpload::Sending { progress: seen } => {
                    inputs.upload = OutgoingUpload::Sending { progress: progress.max(*seen) };
                }
                OutgoingUpload::Failed { .. } => inputs.upload = OutgoingUpload::Sending { progress },
            }
        }
        OutgoingUpload::Sent => {
            if !matches!(inputs.upload, OutgoingUpload::Failed { .. }) || inputs.remote.is_none() {
                inputs.upload = OutgoingUpload::Sent;
            }
        }
        OutgoingUpload::Failed { reason } => {
            if !matches!(inputs.upload, OutgoingUpload::Sent) {
                inputs.upload = OutgoingUpload::Failed { reason };
            }
        }
    }
}

fn apply_remote(inputs: &mut OutgoingInputs, remote: Option<String>) {
    let Some(url) = remote else { return };
    if inputs.remote.as_deref() == Some(url.as_str()) {
        return;
    }
    inputs.remote = Some(url);
    inputs.remote_result = None;
    if !matches!(inputs.upload, OutgoingUpload::Failed { .. }) {
        inputs.upload = OutgoingUpload::Sent;
    }
}

fn apply_result(inputs: &mut OutgoingInputs, result: RemoteResult) {
    if inputs.remote.is_none() {
        return;
    }
    match (&inputs.remote_result, result) {
        (Some(RemoteResult::Complete | RemoteResult::Failed { .. }), _) => {}
        (Some(RemoteResult::Fetching { .. }), RemoteResult::NotReady) => {}
        (Some(RemoteResult::Fetching { progress: seen }), RemoteResult::Fetching { progress }) => {
            inputs.remote_result = Some(RemoteResult::Fetching { progress: clamp(progress).max(*seen) });
        }
        (_, RemoteResult::Fetching { progress }) => {
            inputs.remote_result = Some(RemoteResult::Fetching { progress: clamp(progress) });
        }
        (_, other) => inputs.remote_result = Some(other),
    }
}

/// The one fold: the next state from the previous one and an event. Pure; duplicates and out-of-order events are folded.
#[uniffi::export]
pub fn outgoing_next(previous: Option<OutgoingState>, event: OutgoingEvent) -> OutgoingState {
    let mut inputs = match previous {
        Some(state) => state.inputs,
        None => OutgoingInputs {
            key: String::new(),
            kind: OutgoingKind::Image,
            has_local: false,
            blur_hash: None,
            remote: None,
            upload: OutgoingUpload::Sending { progress: 0.0 },
            remote_result: None,
        },
    };
    match event {
        OutgoingEvent::Upload { upload } => apply_upload(&mut inputs, upload),
        OutgoingEvent::Remote { remote } => apply_remote(&mut inputs, remote),
        OutgoingEvent::RemoteResult { result } => apply_result(&mut inputs, result),
        OutgoingEvent::Retry => {
            if matches!(inputs.remote_result, Some(RemoteResult::Failed { .. })) {
                inputs.remote_result = None;
            }
        }
    }
    OutgoingState { phase: phase_of(&inputs), inputs }
}

/// What the UI draws and may do in this state.
#[uniffi::export]
pub fn outgoing_view(state: OutgoingState) -> OutgoingView {
    let busy = matches!(
        state.phase,
        OutgoingPhase::Sending { .. } | OutgoingPhase::AwaitingRemote | OutgoingPhase::FetchingRemote { .. }
    );
    let reask = busy
        && state.inputs.remote.is_some()
        && matches!(state.inputs.remote_result, Some(RemoteResult::NotReady));
    OutgoingView {
        shows_local: state.phase != OutgoingPhase::Ready,
        shows_loader: busy && state.inputs.has_local,
        can_open: state.phase == OutgoingPhase::Ready,
        reask_after_ms: reask.then_some(REASK_AFTER_MS),
        phase: state.phase,
    }
}

#[uniffi::export]
pub fn outgoing_fetch_slots() -> u32 {
    OUTGOING_FETCH_SLOTS
}

/// Whether the stored local of an item is garbage: never ready and older than 7 days, or not declared again for 24 hours
/// after a relaunch. A ready item has released its local already.
#[uniffi::export]
pub fn outgoing_stale(created_ms: u64, last_declared_ms: u64, now_ms: u64, ready: bool) -> bool {
    if ready {
        return false;
    }
    now_ms.saturating_sub(created_ms) > STALE_UNREADY_MS || now_ms.saturating_sub(last_declared_ms) > STALE_UNDECLARED_MS
}

#[cfg(test)]
mod outgoing_tests;
