//! The command-line grammar. Hand-parsed (no dependency), and strict: an odd value is **refused by
//! name**, never clamped, because a run that silently used a different number than the harness asked
//! for would be reported as the harness's number.
//!
//! ```text
//! kf-rawclient --list-arms
//! kf-rawclient --refresh-compare NATIVE.log GUEST.log
//! kf-rawclient [--gpu N] [--mode bare|native|guest] --<arm> [arm flags]
//! kf-rawclient --help
//! ```
//!
//! `--gpu N` is the RM device minor (default 0), matching the legacy client. Arm flags are the arm's own
//! (`--stress-*` for the refresh stress) and are parsed by the arm.

use crate::mode::Mode;
use crate::registry::{self, ArmSpec};
use std::fmt;

/// A refused command line: a stable name and the offending text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError {
    /// Stable kebab-case name of the refusal.
    pub name: &'static str,
    /// What was wrong.
    pub detail: String,
}

impl UsageError {
    /// Construct.
    #[must_use]
    pub fn new(name: &'static str, detail: impl Into<String>) -> Self {
        UsageError {
            name,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "refused {}: {}", self.name, self.detail)
    }
}

impl std::error::Error for UsageError {}

/// The longest argument accepted. Hostile arguments are refused before anything parses them.
pub const MAX_ARG_BYTES: usize = 256;
/// The most arguments accepted.
pub const MAX_ARGS: usize = 64;
/// The highest GPU minor accepted.
pub const MAX_GPU_MINOR: u32 = 63;

/// What the command line asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Print the registry.
    ListArms,
    /// Print usage.
    Help,
    /// Compare two result logs (`--refresh-compare`).
    Compare {
        /// The native log path.
        native: String,
        /// The guest log path.
        guest: String,
    },
    /// Run one arm.
    Run {
        /// The arm.
        arm: &'static ArmSpec,
        /// RM device minor.
        gpu: u32,
        /// Declared mode.
        mode: Mode,
        /// Arguments for the arm to parse.
        rest: Vec<String>,
    },
}

/// Parse `args` (without `argv[0]`).
///
/// # Errors
/// A [`UsageError`] naming the refusal.
pub fn parse(args: &[String]) -> Result<Command, UsageError> {
    if args.len() > MAX_ARGS {
        return Err(UsageError::new(
            "too-many-arguments",
            format!("{} > {MAX_ARGS}", args.len()),
        ));
    }
    if let Some(a) = args.iter().find(|a| a.len() > MAX_ARG_BYTES) {
        return Err(UsageError::new(
            "argument-too-long",
            format!("{} bytes > {MAX_ARG_BYTES}", a.len()),
        ));
    }
    let mut gpu: Option<u32> = None;
    let mut mode: Option<Mode> = None;
    let mut arm: Option<&'static ArmSpec> = None;
    let mut list = false;
    let mut help = false;
    let mut compare: Option<(String, String)> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--gpu" => {
                let v = it
                    .next()
                    .ok_or_else(|| UsageError::new("missing-value", "--gpu needs a number"))?;
                let n: u32 = v
                    .parse()
                    .map_err(|_| UsageError::new("bad-gpu", format!("{v:?} is not a number")))?;
                if n > MAX_GPU_MINOR {
                    return Err(UsageError::new(
                        "gpu-out-of-range",
                        format!("{n} > {MAX_GPU_MINOR}"),
                    ));
                }
                if gpu.replace(n).is_some() {
                    return Err(UsageError::new("duplicate-flag", "--gpu"));
                }
            }
            "--mode" => {
                let v = it
                    .next()
                    .ok_or_else(|| UsageError::new("missing-value", "--mode needs a word"))?;
                let m = Mode::parse(v).map_err(|e| UsageError::new("bad-mode", e))?;
                if mode.replace(m).is_some() {
                    return Err(UsageError::new("duplicate-flag", "--mode"));
                }
            }
            "--list-arms" => list = true,
            "--help" | "-h" => help = true,
            "--refresh-compare" => {
                let n = it.next().ok_or_else(|| {
                    UsageError::new("missing-value", "--refresh-compare needs two paths")
                })?;
                let g = it.next().ok_or_else(|| {
                    UsageError::new("missing-value", "--refresh-compare needs two paths")
                })?;
                if compare.replace((n.clone(), g.clone())).is_some() {
                    return Err(UsageError::new("duplicate-flag", "--refresh-compare"));
                }
            }
            other => {
                if let Some(spec) = other.strip_prefix("--").and_then(registry::find) {
                    if let Some(prev) = arm.replace(spec) {
                        return Err(UsageError::new(
                            "two-arms",
                            format!("--{} and --{}", prev.name, spec.name),
                        ));
                    }
                } else {
                    rest.push(other.to_owned());
                }
            }
        }
    }
    let modes = usize::from(list)
        + usize::from(help)
        + usize::from(compare.is_some())
        + usize::from(arm.is_some());
    if modes > 1 {
        return Err(UsageError::new(
            "conflicting-commands",
            "give exactly one of --list-arms, --refresh-compare, --help or an arm",
        ));
    }
    if help {
        return Ok(Command::Help);
    }
    if list {
        return reject_extras(&rest, gpu, mode).map(|()| Command::ListArms);
    }
    if let Some((native, guest)) = compare {
        return reject_extras(&rest, gpu, mode).map(|()| Command::Compare { native, guest });
    }
    match arm {
        Some(arm) => Ok(Command::Run {
            arm,
            gpu: gpu.unwrap_or(0),
            mode: mode.unwrap_or(Mode::Unknown),
            rest,
        }),
        None => Err(match rest.first() {
            Some(x) => UsageError::new("unknown-argument", x.clone()),
            None => UsageError::new(
                "no-command",
                "give an arm, --list-arms, --refresh-compare or --help",
            ),
        }),
    }
}

fn reject_extras(rest: &[String], gpu: Option<u32>, mode: Option<Mode>) -> Result<(), UsageError> {
    if let Some(x) = rest.first() {
        return Err(UsageError::new("unknown-argument", x.clone()));
    }
    if gpu.is_some() || mode.is_some() {
        return Err(UsageError::new(
            "unused-flag",
            "--gpu and --mode apply only to running an arm",
        ));
    }
    Ok(())
}

/// The usage text.
#[must_use]
pub fn usage() -> String {
    let mut s = String::from(
        "usage: kf-rawclient --list-arms\n       kf-rawclient --refresh-compare NATIVE.log GUEST.log\n       kf-rawclient [--gpu N] [--mode bare|guest] --<arm> [arm flags]\narms:\n",
    );
    for a in registry::ARMS {
        s.push_str(&format!("  --{:<16} {}\n", a.name, a.doc));
    }
    s.push_str("exit codes: 0 pass, 1 fail, 2 refused command line, 3 not run (decides nothing), 124 watchdog\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> Result<Command, UsageError> {
        parse(&a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn the_legacy_shape_parses() {
        let c = p(&["--gpu", "1", "--refresh-stress"]).unwrap();
        match c {
            Command::Run {
                arm,
                gpu,
                mode,
                rest,
            } => {
                assert_eq!(arm.name, "refresh-stress");
                assert_eq!((gpu, mode), (1, Mode::Unknown));
                assert!(rest.is_empty());
            }
            other => panic!("{other:?}"),
        }
        let c = p(&[
            "--refresh-stress",
            "--mode",
            "guest",
            "--stress-batches",
            "4",
        ])
        .unwrap();
        match c {
            Command::Run {
                gpu, mode, rest, ..
            } => {
                assert_eq!((gpu, mode), (0, Mode::Guest));
                assert_eq!(rest, vec!["--stress-batches", "4"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn other_commands() {
        assert_eq!(p(&["--list-arms"]), Ok(Command::ListArms));
        assert_eq!(p(&["--help"]), Ok(Command::Help));
        assert_eq!(
            p(&["--refresh-compare", "a", "b"]),
            Ok(Command::Compare {
                native: "a".into(),
                guest: "b".into()
            })
        );
    }

    #[test]
    fn hostile_and_odd_command_lines_are_refused_by_name() {
        let name = |a: &[&str]| p(a).unwrap_err().name;
        assert_eq!(name(&[]), "no-command");
        assert_eq!(
            name(&["--timer"]),
            "unknown-argument",
            "legacy arms are not here"
        );
        assert_eq!(name(&["--gpu"]), "missing-value");
        assert_eq!(name(&["--gpu", "-1", "--refresh-stress"]), "bad-gpu");
        assert_eq!(name(&["--gpu", "x", "--refresh-stress"]), "bad-gpu");
        assert_eq!(
            name(&["--gpu", "64", "--refresh-stress"]),
            "gpu-out-of-range"
        );
        assert_eq!(
            name(&["--gpu", "99999999999", "--refresh-stress"]),
            "bad-gpu"
        );
        assert_eq!(
            name(&["--gpu", "1", "--gpu", "2", "--refresh-stress"]),
            "duplicate-flag"
        );
        assert_eq!(name(&["--mode", "metal", "--refresh-stress"]), "bad-mode");
        assert_eq!(
            name(&["--mode", "bare", "--mode", "guest", "--refresh-stress"]),
            "duplicate-flag"
        );
        assert_eq!(
            name(&["--list-arms", "--refresh-stress"]),
            "conflicting-commands"
        );
        assert_eq!(name(&["--list-arms", "--gpu", "1"]), "unused-flag");
        assert_eq!(name(&["--list-arms", "extra"]), "unknown-argument");
        assert_eq!(name(&["--refresh-compare", "a"]), "missing-value");
        assert_eq!(name(&["--refresh-stress", "--refresh-stress"]), "two-arms");
        let long = "x".repeat(MAX_ARG_BYTES + 1);
        assert_eq!(name(&["--refresh-stress", &long]), "argument-too-long");
        let many = vec!["--refresh-stress"; MAX_ARGS + 1];
        assert_eq!(name(&many), "too-many-arguments");
    }

    #[test]
    fn usage_lists_every_registered_arm_and_the_exit_codes() {
        let u = usage();
        for a in registry::ARMS {
            assert!(u.contains(&format!("--{}", a.name)));
        }
        assert!(u.contains("3 not run") && u.contains("124 watchdog"));
    }
}
