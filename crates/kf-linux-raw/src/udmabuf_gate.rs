// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **The `/dev/udmabuf` capability gate — skip LOUDLY, never silently.** The twin of
//! [`crate::kvm_gate`], for the display broker's dma-buf rung (`docs/design/V3_DISPLAY.md` §8).
//!
//! GitHub's runners have no `/dev/udmabuf`; a bench host has it as `root:kvm 0660`. A test that
//! needs it prints, on BOTH arms, straight to stderr (libtest's capture swallows the passing
//! arm otherwise):
//!
//! ```text
//!   UDMABUF-GATE: RAN <test>
//!   UDMABUF-GATE: SKIPPED <test> — <why /dev/udmabuf could not be opened>
//! ```
//!
//! and the CI step `UDMABUF-gate reached-count` counts both, so a skip never reads as a pass and
//! a gated test that silently vanished turns the build red. Never `#[ignore]`: an ignored test
//! is invisible in a summary and cannot be counted.

use std::sync::OnceLock;

/// The device this gate is about.
pub const UDMABUF_DEVICE: &str = "/dev/udmabuf";

/// Open the device read-write (what `UDMABUF_CREATE` needs).
///
/// # Errors
/// The `open` error, verbatim.
pub fn open_device() -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(UDMABUF_DEVICE)
}

/// ★ Is `/dev/udmabuf` present **and permitted to this process**? `Err` carries why not.
/// Resolved once per process, so half a binary cannot disagree about the policy.
pub fn udmabuf_available() -> Result<(), String> {
    static AVAILABLE: OnceLock<Result<(), String>> = OnceLock::new();
    AVAILABLE
        .get_or_init(|| {
            open_device()
                .map(|_| ())
                .map_err(|e| format!("{UDMABUF_DEVICE} cannot be opened read-write: {e}"))
        })
        .clone()
}

/// Emit this test's gate line (both arms). Public because the macro expands at the call site.
pub fn report(test: &str, available: &Result<(), String>) {
    use std::io::Write as _;
    let mut err = std::io::stderr();
    let _ = match available {
        Ok(()) => writeln!(err, "UDMABUF-GATE: RAN {test}"),
        Err(why) => writeln!(
            err,
            "UDMABUF-GATE: SKIPPED {test} — {why} (the test asserts nothing; this line is the \
             only record that it did not run)"
        ),
    };
}

/// Gate the enclosing `#[test]` on `/dev/udmabuf` — `require_udmabuf!("test_name")`. Prints
/// `UDMABUF-GATE: RAN <name>` and continues, or `UDMABUF-GATE: SKIPPED <name> — <why>` and
/// returns.
#[macro_export]
macro_rules! require_udmabuf {
    ($name:expr) => {
        let __udmabuf = $crate::udmabuf_gate::udmabuf_available();
        $crate::udmabuf_gate::report($name, &__udmabuf);
        if __udmabuf.is_err() {
            return;
        }
    };
}
