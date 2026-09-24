//! v3 harness support: CE pushbuffer encoding and a verdict ledger.
//!
//! ★ A verdict is computed from named checks, never from reaching the end of a program (the old
//! tree's *"the last line is not the verdict"*). Every check prints `CHECK <name> PASS|FAIL <why>`.

use kf_abi::submit::{SET_OBJECT, ce, method_header_inc};

/// The CE subchannel every harness push uses.
pub const CE_SUBCHANNEL: u32 = 4;

/// One named check's outcome.
#[derive(Debug)]
pub struct Check {
    /// The check's name.
    pub name: &'static str,
    /// Whether it passed.
    pub pass: bool,
    /// What was measured.
    pub why: String,
}

/// The run's checks, and the verdict they imply.
#[derive(Debug, Default)]
pub struct Ledger {
    checks: Vec<Check>,
}

impl Ledger {
    /// Record and print one check.
    pub fn check(&mut self, name: &'static str, pass: bool, why: impl Into<String>) {
        let why = why.into();
        println!("CHECK {name} {} {why}", if pass { "PASS" } else { "FAIL" });
        self.checks.push(Check { name, pass, why });
    }

    /// Record a measurement that is NOT a pass/fail (an unknown being measured).
    pub fn measure(&mut self, name: &'static str, value: impl Into<String>) {
        println!("MEASURE {name} {}", value.into());
    }

    /// PASS only if at least one check ran and every check passed. ⊘ Zero checks is a FAIL.
    #[must_use]
    pub fn verdict(&self) -> bool {
        !self.checks.is_empty() && self.checks.iter().all(|c| c.pass)
    }
}

/// A virtual-addressed CE copy of `len` bytes `src → dst`, releasing `payload` at `sem_va`,
/// optionally raising a NON-STALL interrupt when done.
#[must_use]
pub fn ce_copy_push(
    ce_class: u32,
    src: u64,
    dst: u64,
    len: u32,
    sem_va: u64,
    payload: u32,
    interrupt: bool,
) -> Option<Vec<u32>> {
    let sub = CE_SUBCHANNEL;
    let mut flags = ce::LAUNCH_TRANSFER_NON_PIPELINED
        | ce::LAUNCH_FLUSH_ENABLE
        | ce::LAUNCH_SEMAPHORE_RELEASE_ONE_WORD
        | ce::LAUNCH_SRC_PITCH
        | ce::LAUNCH_DST_PITCH
        | ce::LAUNCH_MULTI_LINE_DISABLE
        | ce::LAUNCH_SRC_VIRTUAL
        | ce::LAUNCH_DST_VIRTUAL;
    if interrupt {
        flags |= ce::LAUNCH_INTERRUPT_NON_BLOCKING;
    }
    Some(vec![
        method_header_inc(sub, SET_OBJECT, 1)?,
        ce_class,
        method_header_inc(sub, ce::OFFSET_IN_UPPER, 4)?,
        (src >> 32) as u32,
        (src & 0xFFFF_FFFF) as u32,
        (dst >> 32) as u32,
        (dst & 0xFFFF_FFFF) as u32,
        method_header_inc(sub, ce::LINE_LENGTH_IN, 2)?,
        len,
        1,
        method_header_inc(sub, ce::SET_SEMAPHORE_A, 3)?,
        (sem_va >> 32) as u32,
        (sem_va & 0xFFFF_FFFF) as u32,
        payload,
        method_header_inc(sub, ce::LAUNCH_DMA, 1)?,
        flags,
    ])
}
