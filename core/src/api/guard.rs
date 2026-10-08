use tokio::task::JoinHandle;

pub struct AbortOnDrop(JoinHandle<()>);

impl AbortOnDrop {
    pub fn new(task: JoinHandle<()>) -> AbortOnDrop {
        AbortOnDrop(task)
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
