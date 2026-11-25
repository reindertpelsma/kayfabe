//! The pure part of `kf-debugger-cap-probe` (`traces/debugger_cap_20261125/PREREGISTRATION.md`): the command
//! line, the controller's commands, how sessions are spread over GR objects, the latency summary and the
//! names of refusals. Tested GPU-free; the binary only allocates host debugger sessions and reports.

use kf_host::RmError;

/// What the command line asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// The host GPU minor.
    pub gpu: u32,
    /// GR objects (GR channel plus compute object) this process builds.
    pub grs: usize,
    /// Sessions allowed per GR object; 0 = no limit of ours.
    pub per_gr: usize,
    /// Sessions this process stops at.
    pub max_sessions: usize,
    /// One `PROGRESS` line per this many sessions.
    pub progress_every: usize,
    /// Wait for the controller's `go` line; false = start at once (hand runs).
    pub wait_go: bool,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            gpu: 0,
            grs: 1,
            per_gr: 0,
            max_sessions: 20_000,
            progress_every: 100,
            wait_go: true,
        }
    }
}

/// Parse `--gpu N --grs N --per-gr N --max-sessions N --progress-every N [--no-wait]`.
///
/// # Errors
/// An unknown flag, a missing or non-numeric value, `--grs 0` or `--progress-every 0`.
pub fn parse_args<I: IntoIterator<Item = String>>(argv: I) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = argv.into_iter();
    while let Some(flag) = it.next() {
        if flag == "--no-wait" {
            a.wait_go = false;
            continue;
        }
        let mut num = |name: &str| -> Result<usize, String> {
            let v = it.next().ok_or_else(|| format!("{name} needs a value"))?;
            v.parse::<usize>()
                .map_err(|_| format!("{name}: `{v}` is not a number"))
        };
        match flag.as_str() {
            "--gpu" => a.gpu = u32::try_from(num("--gpu")?).map_err(|_| "--gpu too large")?,
            "--grs" => a.grs = num("--grs")?,
            "--per-gr" => a.per_gr = num("--per-gr")?,
            "--max-sessions" => a.max_sessions = num("--max-sessions")?,
            "--progress-every" => a.progress_every = num("--progress-every")?,
            other => return Err(format!("unknown flag `{other}`")),
        }
    }
    if a.grs == 0 {
        return Err("--grs must be at least 1".into());
    }
    if a.progress_every == 0 {
        return Err("--progress-every must be at least 1".into());
    }
    Ok(a)
}

/// A line of the controller on the worker's stdin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    /// Start allocating.
    Go,
    /// Stop allocating now (the ceiling, an Xid, FB growth or the wall limit was hit).
    Stop,
    /// Free the sessions.
    Release,
    /// Free the GR objects and exit.
    Exit,
}

/// One controller line. Unknown lines are `None`.
#[must_use]
pub fn parse_cmd(line: &str) -> Option<Cmd> {
    match line.trim() {
        "go" => Some(Cmd::Go),
        "stop" => Some(Cmd::Stop),
        "release" => Some(Cmd::Release),
        "exit" => Some(Cmd::Exit),
        _ => None,
    }
}

/// Which GR object gets the next session: round robin over the objects that are neither refused nor full.
#[derive(Debug)]
pub struct Spread {
    per_gr: usize,
    counts: Vec<usize>,
    dead: Vec<bool>,
    next: usize,
}

impl Spread {
    /// `grs` objects, at most `per_gr` sessions each (0 = no limit).
    #[must_use]
    pub fn new(grs: usize, per_gr: usize) -> Self {
        Spread {
            per_gr,
            counts: vec![0; grs],
            dead: vec![false; grs],
            next: 0,
        }
    }

    fn open(&self, j: usize) -> bool {
        !self.dead[j] && (self.per_gr == 0 || self.counts[j] < self.per_gr)
    }

    /// The object for the next session, or `None` when every object is refused or full.
    pub fn target(&mut self) -> Option<usize> {
        let n = self.counts.len();
        for step in 0..n {
            let j = (self.next + step) % n;
            if self.open(j) {
                self.next = (j + 1) % n;
                return Some(j);
            }
        }
        None
    }

    /// A session was made on `j`.
    pub fn made(&mut self, j: usize) {
        self.counts[j] += 1;
    }

    /// Host RM refused a session on `j`; the object takes no more.
    pub fn refused(&mut self, j: usize) {
        self.dead[j] = true;
    }

    /// Sessions made on `j`.
    #[must_use]
    pub fn on(&self, j: usize) -> usize {
        self.counts[j]
    }

    /// Objects host RM refused.
    #[must_use]
    pub fn refused_count(&self) -> usize {
        self.dead.iter().filter(|d| **d).count()
    }
}

/// Microsecond latency summary of a window of allocs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Latency {
    /// Samples.
    pub n: usize,
    /// Mean, microseconds.
    pub mean: u64,
    /// Median.
    pub p50: u32,
    /// 99th percentile (nearest rank).
    pub p99: u32,
    /// Slowest.
    pub max: u32,
}

/// Summarise `us` (sorted in place). `None` for an empty window.
#[must_use]
pub fn latency(us: &mut [u32]) -> Option<Latency> {
    if us.is_empty() {
        return None;
    }
    us.sort_unstable();
    let n = us.len();
    let rank = |p: usize| us[(n * p).div_ceil(100).saturating_sub(1).min(n - 1)];
    let sum: u64 = us.iter().map(|v| u64::from(*v)).sum();
    Some(Latency {
        n,
        mean: sum / n as u64,
        p50: rank(50),
        p99: rank(99),
        max: us[n - 1],
    })
}

/// The refusal as host RM answered it. kf-host folds `NV_ERR_NO_MEMORY` (0x51) and
/// `NV_ERR_INSUFFICIENT_RESOURCES` (0x1a) into one variant, so that one names both.
#[must_use]
pub fn refusal_name(e: &RmError) -> String {
    match e {
        RmError::InsufficientPermissions => "NV_ERR_INSUFFICIENT_PERMISSIONS(0x1b)".into(),
        RmError::NoMemory => "NV_ERR_NO_MEMORY(0x51)_or_NV_ERR_INSUFFICIENT_RESOURCES(0x1a)".into(),
        RmError::Interrupted => "EINTR".into(),
        RmError::Other(code) => match code {
            0x1f => "NV_ERR_INVALID_ARGUMENT(0x1f)".into(),
            0x22 => "NV_ERR_INVALID_CLASS(0x22)".into(),
            0x31 => "NV_ERR_INVALID_OBJECT(0x31)".into(),
            0x40 => "NV_ERR_INVALID_STATE(0x40)".into(),
            0x56 => "NV_ERR_NOT_SUPPORTED(0x56)".into(),
            c => format!("status({c:#x})"),
        },
        RmError::PlacementRefused { want, got } => format!("PLACEMENT({want:#x},{got:#x})"),
    }
}

/// The worker's resident set in KiB from `/proc/self/status` text (`VmRSS:` line).
#[must_use]
pub fn parse_rss_kib(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| (*x).to_string()).collect()
    }

    #[test]
    fn args_defaults_and_flags() {
        assert_eq!(parse_args(v(&[])).unwrap(), Args::default());
        let a = parse_args(v(&[
            "--gpu",
            "1",
            "--grs",
            "16",
            "--per-gr",
            "4",
            "--max-sessions",
            "500",
            "--progress-every",
            "10",
            "--no-wait",
        ]))
        .unwrap();
        assert_eq!((a.gpu, a.grs, a.per_gr, a.max_sessions), (1, 16, 4, 500));
        assert_eq!(a.progress_every, 10);
        assert!(!a.wait_go);
    }

    #[test]
    fn args_refuse_bad_input() {
        assert!(parse_args(v(&["--grs", "0"])).is_err());
        assert!(parse_args(v(&["--grs"])).is_err());
        assert!(parse_args(v(&["--grs", "x"])).is_err());
        assert!(parse_args(v(&["--progress-every", "0"])).is_err());
        assert!(parse_args(v(&["--nope"])).is_err());
    }

    #[test]
    fn commands() {
        assert_eq!(parse_cmd("go\n"), Some(Cmd::Go));
        assert_eq!(parse_cmd(" stop "), Some(Cmd::Stop));
        assert_eq!(parse_cmd("release"), Some(Cmd::Release));
        assert_eq!(parse_cmd("exit"), Some(Cmd::Exit));
        assert_eq!(parse_cmd("GO"), None);
        assert_eq!(parse_cmd(""), None);
    }

    #[test]
    fn spread_round_robins_and_stops_at_the_per_object_limit() {
        let mut s = Spread::new(3, 2);
        let mut order = Vec::new();
        while let Some(j) = s.target() {
            s.made(j);
            order.push(j);
        }
        assert_eq!(order, vec![0, 1, 2, 0, 1, 2]);
        assert_eq!((s.on(0), s.on(1), s.on(2)), (2, 2, 2));
    }

    #[test]
    fn spread_skips_a_refused_object_and_ends_when_all_are() {
        let mut s = Spread::new(2, 0);
        let j = s.target().unwrap();
        s.made(j);
        s.refused(1);
        assert_eq!(s.refused_count(), 1);
        for _ in 0..5 {
            assert_eq!(s.target(), Some(0));
        }
        s.refused(0);
        assert_eq!(s.target(), None);
    }

    #[test]
    fn spread_unlimited_never_fills() {
        let mut s = Spread::new(1, 0);
        for _ in 0..1000 {
            let j = s.target().unwrap();
            s.made(j);
        }
        assert_eq!(s.on(0), 1000);
    }

    #[test]
    fn latency_ranks() {
        assert_eq!(latency(&mut []), None);
        let mut w: Vec<u32> = (1..=100).rev().collect();
        let l = latency(&mut w).unwrap();
        assert_eq!((l.n, l.p50, l.p99, l.max, l.mean), (100, 50, 99, 100, 50));
        let mut one = [7u32];
        let l = latency(&mut one).unwrap();
        assert_eq!((l.p50, l.p99, l.max, l.mean), (7, 7, 7, 7));
    }

    #[test]
    fn refusals_are_named() {
        assert!(refusal_name(&RmError::NoMemory).contains("0x51"));
        assert!(refusal_name(&RmError::NoMemory).contains("0x1a"));
        assert_eq!(
            refusal_name(&RmError::Other(0x40)),
            "NV_ERR_INVALID_STATE(0x40)"
        );
        assert_eq!(refusal_name(&RmError::Other(0x1234)), "status(0x1234)");
        assert!(refusal_name(&RmError::InsufficientPermissions).contains("0x1b"));
    }

    #[test]
    fn rss_parses() {
        let s = "Name:\tx\nVmRSS:\t   12345 kB\nThreads:\t3\n";
        assert_eq!(parse_rss_kib(s), Some(12345));
        assert_eq!(parse_rss_kib("Name:\tx\n"), None);
    }
}
