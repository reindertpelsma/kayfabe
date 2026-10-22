//! Bench-only registration/wait seam; the backend owns one event source.
pub(super) trait Source {
    fn register(&mut self) -> Result<(), String>;
    fn wait(&mut self, ms: u32) -> Result<bool, String>;
}
pub(super) struct Probe<S> {
    source: S,
}
impl<S: Source> Probe<S> {
    pub(super) fn prepare(source: S) -> Result<Self, String> {
        Ok(Self { source })
    }
    pub(super) fn ready_within(&mut self, ms: u32) -> Result<bool, String> {
        self.source.register()?;
        self.source.wait(ms)
    }
}
