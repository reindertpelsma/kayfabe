// EXPECTED RED API scaffold: no native recorder exists yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    User,
    Translated,
    Private,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cause {
    OwnWalk,
    OtherWalk,
    Split,
    ScanBudget,
    PtLock,
    Storage,
    Act,
    Worker,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Wait {
    pub cause: Cause,
    pub start: u64,
    pub end: u64,
    pub owner: u64,
}
#[derive(Debug, Clone)]
pub(super) struct Sample {
    pub stream: u64,
    pub token: u32,
    pub kind: Kind,
    pub observed: u64,
    pub published: u64,
    pub marker: Option<u64>,
    pub wrap: bool,
    pub advanced: Option<u64>,
    pub gpu: Option<u64>,
    pub vector: Option<u32>,
    pub raised: Option<u64>,
    pub waits: Vec<Wait>,
    pub worker: u64,
}
impl Sample {
    pub fn stages(&self) -> Option<[u64; 3]> {
        None
    }
    pub fn observation_lag(&self) -> Option<u64> {
        None
    }
}
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Gauge {
    pub count: u64,
    pub max: u64,
}
#[derive(Debug, Default)]
pub(super) struct State {
    pub pending: std::collections::VecDeque<Sample>,
    pub slow: std::collections::VecDeque<Sample>,
    pub gauges: [Gauge; 4],
    pub overflow: u64,
    pub canceled: u64,
    pub early: u64,
    pub first_dump: bool,
    pub marker_unknown: u64,
}
impl State {
    pub fn publish(&mut self, _s: Sample) {}
    pub fn cover(&mut self, _stream: u64, _marker: u64, _wrap: bool) {}
    pub fn marker(&mut self, _stream: u64, _done: u64, _at: u64, _gpu: Option<u64>) {}
    pub fn notify(&mut self, _vector: u32, _at: u64, _success: bool) {}
    pub fn cancel(&mut self, _stream: u64) {}
    pub fn expire(&mut self, _now: u64) {}
}
