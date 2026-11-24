//! The pure part of `kf-semmarker-probe` (`docs/design/V3_BLACKWELL_USER_RING_ROUTE.md` §2.7): the
//! exact words each variant pushes, the host-dmesg Xid parser, and the verdict. Tested GPU-free; the binary only
//! runs these words on a native USER channel and reports.
//!
//! Variants (each one process, one fresh channel, its own VA space, owned memory only; no guest memory):
//! - **A** `SET_REFERENCE 0x4B460001`, then a WITNESS release. Measured once on GB203 / 595.91.07
//!   (`traces/semprobe_gb203_20261118/`): one Xid 32 with `intr 00200000` (METHOD), and the witness after it
//!   still landed, so the verdict does not look at the witness. Any other Xid, or a second one, is `Unexpected`.
//! - **B** the stage-1 semaphore marker cell (`kf_chan::user_semaphore::marker_cell`, `SemaphoreMarker`): the word
//!   must read the marker value, and no Xid.
//! - **C** `NON_STALL_INTERRUPT 0`, then a witness release: no Xid, the witness lands.

use kf_abi::submit::{fifo, method_header_inc};
use kf_chan::user_semaphore::{ProgressSource, marker_cell};

/// The marker value variant A references and variant B releases.
pub const MARK: u32 = 0x4B46_0001;
/// The witness value a variant's trailing release writes (`0x5717_00` | variant letter).
#[must_use]
pub const fn witness_value(v: Variant) -> u32 {
    0x5717_0000 | v.letter() as u32
}

/// One probe variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// `SET_REFERENCE` then a witness release.
    A,
    /// The stage-1 semaphore marker cell.
    B,
    /// `NON_STALL_INTERRUPT` then a witness release.
    C,
}

impl Variant {
    /// All variants, in run order.
    pub const ALL: [Self; 3] = [Self::A, Self::B, Self::C];

    /// The variant's letter.
    #[must_use]
    pub const fn letter(self) -> char {
        match self {
            Self::A => 'A',
            Self::B => 'B',
            Self::C => 'C',
        }
    }

    /// Parse `A` / `B` / `C`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| s == v.letter().to_string())
    }

    /// The value the probe expects in the owned word once the stream ran to its end.
    #[must_use]
    pub const fn expected_word(self) -> u32 {
        match self {
            Self::B => MARK,
            Self::A | Self::C => witness_value(self),
        }
    }
}

/// A plain 32-bit release of `value` to `va`: UVM's Hopper+ host release (`ogkm-595
/// kernel-open/nvidia-uvm/uvm_hopper_host.c:45-52`), no interrupt.
///
/// # Errors
/// A VA the semaphore cannot address (zero, unaligned, beyond 57 bits).
pub fn release(va: u64, value: u32) -> Result<Vec<u32>, &'static str> {
    let mut cell = marker_cell(ProgressSource::SemaphoreMarker, value, va)?;
    cell.truncate(6); // drop the trailing NON_STALL_INTERRUPT pair: a witness raises nothing
    Ok(cell)
}

/// The exact words variant `v` pushes, with `word_va` the probe's owned, GPU-writable 4-byte word.
///
/// # Errors
/// An unencodable method or VA.
pub fn words(v: Variant, word_va: u64) -> Result<Vec<u32>, &'static str> {
    match v {
        Variant::A => {
            let mut w = vec![
                method_header_inc(0, fifo::SET_REFERENCE_C56F, 1).ok_or("encode")?,
                MARK,
            ];
            w.extend(release(word_va, witness_value(v))?);
            Ok(w)
        }
        Variant::B => marker_cell(ProgressSource::SemaphoreMarker, MARK, word_va),
        Variant::C => {
            let mut w = vec![
                method_header_inc(0, fifo::NON_STALL_INTERRUPT, 1).ok_or("encode")?,
                0,
            ];
            w.extend(release(word_va, witness_value(v))?);
            Ok(w)
        }
    }
}

/// The method list of a word stream, for the record (`(byte method, argument words)` per header).
#[must_use]
pub fn methods(words: &[u32]) -> Vec<(u32, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&w) = words.get(i) {
        let Some(h) = kf_abi::submit::method_header_decode(w) else {
            break;
        };
        out.push((h.method, h.arg_words));
        i += 1 + h.arg_words;
    }
    out
}

/// One host Xid from a dmesg line: `NVRM: Xid (PCI:...): <n>, pid=..., ... intr <hex>` (the `intr` part may be absent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Xid {
    /// The Xid number.
    pub code: u32,
    /// The `intr` word, when printed.
    pub intr: Option<u32>,
}

/// Parse a dmesg line; `None` when it is not an `NVRM: Xid` line.
#[must_use]
pub fn parse_xid(line: &str) -> Option<Xid> {
    let rest = line.split("NVRM: Xid (").nth(1)?;
    let after = rest.split_once("): ")?.1;
    let code = after
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    let intr = after.split_once(" intr ").and_then(|(_, t)| {
        let t = t.trim_start().trim_start_matches("0x");
        let end = t.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(t.len());
        u32::from_str_radix(&t[..end], 16).ok()
    });
    Some(Xid { code, intr })
}

/// What one attempt observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// The child process ended by itself (not killed by the watchdog).
    pub exited: bool,
    /// The owned word after the stream, if the child read it.
    pub word: Option<u32>,
    /// Host Xids that appeared during the attempt.
    pub xids: Vec<Xid>,
}

/// PBDMA `INTR_0` METHOD (bit 21; GA100 `dev_pbdma.ref` meaning, INFERRED for GB20x) as RM prints it after `intr`.
pub const INTR_METHOD: u32 = 0x0020_0000;

/// The verdict of one attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The stream ran to its end with no Xid.
    Clean,
    /// Variant A only: exactly one Xid 32 with PBDMA `INTR_0` METHOD (bit 21, [`INTR_METHOD`]): 0x50 is not a
    /// method here. Whether the witness after it lands does not matter: measured 2026-11-18 on GB203 / 595.91.07
    /// (probe `d360cd651`), the Xid was raised AND the later witness release landed.
    MethodRefused,
    /// Anything else: stop the run (fail closed) and keep the evidence.
    Unexpected,
}

/// Classify one attempt of `v`.
#[must_use]
pub fn verdict(v: Variant, a: &Attempt) -> Verdict {
    if !a.exited {
        return Verdict::Unexpected;
    }
    let landed = a.word == Some(v.expected_word());
    match (v, a.xids.as_slice(), landed) {
        (_, [], true) => Verdict::Clean,
        (
            Variant::A,
            [
                Xid {
                    code: 32,
                    intr: Some(INTR_METHOD),
                },
            ],
            _,
        ) => Verdict::MethodRefused,
        _ => Verdict::Unexpected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VA: u64 = 0x0000_0010_0000_0000;

    #[test]
    fn variant_a_is_set_reference_then_a_witness_release() {
        let w = words(Variant::A, VA).unwrap();
        assert_eq!(
            methods(&w),
            vec![(fifo::SET_REFERENCE_C56F, 1), (fifo::SEM_ADDR_LO, 5)]
        );
        assert_eq!(w[1], MARK);
        assert_eq!(w[5], witness_value(Variant::A));
        assert_eq!(w.len(), 8);
    }

    #[test]
    fn variant_b_is_exactly_the_stage1_cell() {
        let w = words(Variant::B, VA).unwrap();
        assert_eq!(
            w,
            marker_cell(ProgressSource::SemaphoreMarker, MARK, VA).unwrap()
        );
        assert_eq!(
            methods(&w),
            vec![(fifo::SEM_ADDR_LO, 5), (fifo::NON_STALL_INTERRUPT, 1)]
        );
    }

    #[test]
    fn variant_c_is_an_interrupt_then_a_witness_release() {
        let w = words(Variant::C, VA).unwrap();
        assert_eq!(
            methods(&w),
            vec![(fifo::NON_STALL_INTERRUPT, 1), (fifo::SEM_ADDR_LO, 5)]
        );
        assert_eq!(w[5], witness_value(Variant::C));
    }

    #[test]
    fn a_witness_release_has_no_interrupt_and_no_reference() {
        let w = release(VA, 7).unwrap();
        assert_eq!(methods(&w), vec![(fifo::SEM_ADDR_LO, 5)]);
        assert_eq!(&w[1..], &[0, 0x10, 7, 0, 1]);
        assert_eq!(release(3, 7), Err("USER_SEM_VA"));
    }

    #[test]
    fn parses_the_creo04_xid_line() {
        let line = "[ 8468.111405] NVRM: Xid (PCI:0000:00:07): 32, pid=12345, name=qemu-system-x86, channel 0x00000016 intr 00800000";
        assert_eq!(
            parse_xid(line),
            Some(Xid {
                code: 32,
                intr: Some(0x0080_0000)
            })
        );
        assert_eq!(
            parse_xid(
                "NVRM: Xid (PCI:0000:01:00): 31, pid=1, Ch 00000010, intr 10000000. MMU Fault"
            ),
            Some(Xid {
                code: 31,
                intr: Some(0x1000_0000)
            })
        );
        assert_eq!(
            parse_xid("NVRM: Xid (PCI:0000:01:00): 79, GPU has fallen off the bus."),
            Some(Xid {
                code: 79,
                intr: None
            })
        );
        assert_eq!(
            parse_xid("x86/PAT: kf3-vamgr freeing invalid memtype"),
            None
        );
    }

    fn att(exited: bool, word: Option<u32>, xids: &[Xid]) -> Attempt {
        Attempt {
            exited,
            word,
            xids: xids.to_vec(),
        }
    }

    #[test]
    fn verdicts() {
        let x32 = Xid {
            code: 32,
            intr: Some(0x0020_0000),
        };
        let x31 = Xid {
            code: 31,
            intr: None,
        };
        for v in Variant::ALL {
            assert_eq!(
                verdict(v, &att(true, Some(v.expected_word()), &[])),
                Verdict::Clean
            );
            assert_eq!(
                verdict(v, &att(false, Some(v.expected_word()), &[])),
                Verdict::Unexpected
            );
            if v == Variant::A {
                assert_eq!(
                    verdict(v, &att(true, Some(v.expected_word()), &[x32])),
                    Verdict::MethodRefused,
                    "the witness may land after the METHOD Xid"
                );
            } else {
                assert_eq!(
                    verdict(v, &att(true, Some(v.expected_word()), &[x32])),
                    Verdict::Unexpected
                );
            }
            assert_eq!(verdict(v, &att(true, None, &[x31])), Verdict::Unexpected);
            assert_eq!(verdict(v, &att(true, Some(0), &[])), Verdict::Unexpected);
        }
        assert_eq!(
            verdict(Variant::A, &att(true, Some(0), &[x32])),
            Verdict::MethodRefused
        );
        assert_eq!(
            verdict(Variant::B, &att(true, Some(0), &[x32])),
            Verdict::Unexpected
        );
        assert_eq!(
            verdict(Variant::C, &att(true, Some(0), &[x32])),
            Verdict::Unexpected
        );
        assert_eq!(
            verdict(Variant::A, &att(true, Some(0), &[x32, x32])),
            Verdict::Unexpected
        );
    }

    /// Any Xid with a landed word, a killed child, or a missing word stops the run, for every variant.
    #[test]
    fn anything_but_a_clean_run_stops_the_run() {
        let x = |code| Xid { code, intr: None };
        for v in Variant::ALL {
            assert_eq!(
                verdict(v, &att(true, Some(v.expected_word()), &[x(32)])),
                Verdict::Unexpected
            );
            assert_eq!(
                verdict(v, &att(true, Some(v.expected_word()), &[x(31)])),
                Verdict::Unexpected
            );
            assert_eq!(
                verdict(v, &att(false, Some(v.expected_word()), &[])),
                Verdict::Unexpected
            );
            assert_eq!(verdict(v, &att(true, None, &[])), Verdict::Unexpected);
        }
        // Only an A attempt with exactly one Xid 32 whose intr is METHOD is MethodRefused.
        assert_eq!(
            verdict(Variant::A, &att(true, None, &[x(31)])),
            Verdict::Unexpected
        );
        assert_eq!(
            verdict(Variant::A, &att(true, None, &[x(32)])),
            Verdict::Unexpected,
            "no intr printed"
        );
        assert_eq!(
            verdict(Variant::B, &att(true, None, &[x(32)])),
            Verdict::Unexpected
        );
    }

    /// The GB203 A attempt as recorded in `traces/semprobe_gb203_20261118/`: one Xid 32 `intr 00200000`, AND the witness landed.
    #[test]
    fn the_measured_gb203_a_attempt_is_method_refused() {
        let line = "[18942.341911] NVRM: Xid (PCI:0000:00:07): 32, pid=118554, name=kf-semmarker-pr, channel 0x00000002 intr 00200000";
        let x = parse_xid(line).unwrap();
        assert_eq!(
            x,
            Xid {
                code: 32,
                intr: Some(INTR_METHOD)
            }
        );
        let landed = att(true, Some(witness_value(Variant::A)), &[x]);
        assert_eq!(verdict(Variant::A, &landed), Verdict::MethodRefused);
        assert_eq!(
            verdict(Variant::A, &att(true, Some(0), &[x])),
            Verdict::MethodRefused
        );
        // A DEVICE Xid 32 (the creo-04 signature) is not the METHOD answer.
        let device = Xid {
            code: 32,
            intr: Some(0x0080_0000),
        };
        assert_eq!(
            verdict(Variant::A, &landed_with(device)),
            Verdict::Unexpected
        );
    }

    fn landed_with(x: Xid) -> Attempt {
        att(true, Some(witness_value(Variant::A)), &[x])
    }

    /// A's answer is Xid 32 specifically: another Xid number with the same METHOD `intr` bits is not it.
    /// (A MMU fault or a fallen-off-the-bus GPU can print the same `intr` word; code 20 is how the first run's
    /// log printed 32, in hexadecimal.)
    #[test]
    fn a_method_refusal_is_xid_32_and_no_other_number() {
        for code in [0, 13, 20, 31, 33, 43, 45, 48, 61, 62, 79, 119] {
            let x = Xid {
                code,
                intr: Some(INTR_METHOD),
            };
            for word in [None, Some(Variant::A.expected_word()), Some(0)] {
                assert_eq!(
                    verdict(Variant::A, &att(true, word, &[x])),
                    Verdict::Unexpected,
                    "Xid {code} with the METHOD intr bits is not variant A's answer"
                );
            }
        }
        let x32 = Xid {
            code: 32,
            intr: Some(INTR_METHOD),
        };
        assert_eq!(
            verdict(Variant::A, &att(true, None, &[x32])),
            Verdict::MethodRefused
        );
    }

    /// A second Xid of any kind next to A's own stops the run, in either order.
    #[test]
    fn an_extra_xid_next_to_a_methods_refusal_stops_the_run() {
        let m = Xid {
            code: 32,
            intr: Some(INTR_METHOD),
        };
        for extra in [
            m,
            Xid {
                code: 31,
                intr: None,
            },
            Xid {
                code: 79,
                intr: Some(INTR_METHOD),
            },
        ] {
            for order in [[m, extra], [extra, m]] {
                assert_eq!(
                    verdict(
                        Variant::A,
                        &att(true, Some(Variant::A.expected_word()), &order)
                    ),
                    Verdict::Unexpected,
                    "{order:?}"
                );
            }
        }
    }

    #[test]
    fn variants_parse_by_letter() {
        for v in Variant::ALL {
            assert_eq!(Variant::parse(&v.letter().to_string()), Some(v));
        }
        assert_eq!(Variant::parse("D"), None);
    }
}
