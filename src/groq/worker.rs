use std::process::{Child, Command, Stdio};

/// A recording owns its warm helper. Discard, cancel and shutdown reclaim it.
pub struct Worker(Option<Child>);

impl Worker {
    pub fn start() -> std::io::Result<Self> {
        let child = crate::platform::spawn_hidden(
            Command::new(std::env::current_exe()?)
                .args(["_worker", "--warm"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit()),
        )?;
        Ok(Self(Some(child)))
    }
    pub fn take(mut self) -> Child {
        self.0.take().unwrap()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
