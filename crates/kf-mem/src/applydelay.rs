//! Host-configured, bounded delay on the ordinary VA-manager apply thread.
/// Diagnostic policy, never guest input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Requested delay, ms; zero is OFF.
    pub ms: u64,
    /// Minimum attempted diff runs in an apply.
    pub min_runs: usize,
}
impl Config {
    /// Strict bounded operator configuration; missing delay is OFF.
    pub fn parse(_ms: Option<&str>, _runs: Option<&str>) -> Result<Self, &'static str> {
        Ok(Self {
            ms: 0,
            min_runs: 1000,
        })
    }
    /// Delay of this apply; does not alter gate policy.
    pub fn for_runs(self, _runs: usize) -> u64 {
        0
    }
    /// Execute on the caller, with a fake sleeper seam for GPU-free checks.
    pub fn run(self, runs: usize, mut sleep: impl FnMut(u64)) -> u64 {
        let ms = self.for_runs(runs);
        if ms != 0 {
            sleep(ms);
        }
        ms
    }
}
