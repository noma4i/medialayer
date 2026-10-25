use tokio::runtime::Handle;

struct Once<Job: FnOnce()>(Option<Job>);

impl<Job: FnOnce()> Once<Job> {
    fn run(mut self) {
        if let Some(job) = self.0.take() {
            job();
        }
    }
}

impl<Job: FnOnce()> Drop for Once<Job> {
    fn drop(&mut self) {
        if let Some(job) = self.0.take() {
            job();
        }
    }
}

/// Runs a platform release off the calling thread; a runtime that drops the task still runs it once, on the dropping thread.
pub(crate) fn release_once(job: impl FnOnce() + Send + 'static) {
    let once = Once(Some(job));
    match Handle::try_current() {
        Ok(handle) => {
            handle.spawn_blocking(move || once.run());
        }
        Err(_) => once.run(),
    }
}
