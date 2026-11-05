//! **S5 — the replay projection**, and the ledger of things we deliberately do
//! differently.
//!
//! ## The differential is over a decoded projection, never over raw bytes
//!
//! `mode2_gsp_port_plan.md` §6.3. Byte-diffing against the C artifact would *enshrine*
//! three things we intend not to reproduce: its zero padding, its uninitialised element
//! tails, and `rpc.length = 36` for a 32-byte header. [`Observation`] therefore carries
//! decoded fields, so a divergence is expressible as a field-level exception with a
//! reason — and so class (3), "bytes no guest reads", is excluded *by construction*
//! rather than by an allowlist that rots.
//!
//! Every assertion falls in exactly one of three classes, declared rather than inferred:
//!
//! 1. **MUST-MATCH** — anything ogkm shows the guest reading or validating. The default.
//! 2. **MUST-DIFFER** — [`LEDGER`], and an entry is admissible only with all four of a C
//!    site, a guest-visible consequence, our behaviour, and an **independent oracle**.
//!    *"It's cleaner"* is not an oracle.
//! 3. **UNCONSTRAINED** — not comparable in the first place.
//!
//! ## ★ The recorded C traces do not exist yet, and this file does not pretend otherwise
//!
//! §6.2's capture patch is ~60 lines against `nvkvm_gpu_emul.c`, a file this work does not
//! own, and it is called out in the plan as its own task. So the harness here is proven
//! against a **guest model** — an independent implementation of the driver's own receive
//! path, which is a stronger oracle than a synthetic script for everything except
//! C-parity, and no oracle at all for C-parity. That gap is real and is stated rather
//! than papered over: until the capture lands, "byte-equivalence with the C" is untested.
//!
//! ## The non-vacuity problem this file is built around
//!
//! The classic vacuous protocol test is a replay that passes because the script never
//! reached the transition it claims to cover. Two mechanisms, not one comment:
//! [`crate::Transition`] is recorded on every step so a test asserts the transition
//! *fired*, and [`ObservationLog`] counts every observation kind so "this instrument read
//! zero because nothing happened" is distinguishable from "…because nothing was
//! measured".

use crate::element::ElementLayout;
use crate::fault::GspFault;
use crate::ram::{GuestRam, RegionMap};
use crate::ring::{MsgqGeometry, TxHeader};
use kayfabe_abi::view::RpcEnvelope;
use kayfabe_arch::GspReg;
use kayfabe_trace::{FaultTag, Faulted};

/// Which of the two queues an observation is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QueueId {
    /// CPU → GSP. The guest produces; we consume.
    Command,
    /// GSP → CPU. We produce; the guest consumes.
    Status,
}

/// One thing a guest could possibly notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// We published a queue's tx header — the thing `msgqRxLink` spins on.
    TxHeaderPublished {
        /// Which queue.
        queue: QueueId,
        /// The header, decoded.
        hdr: TxHeader,
    },
    /// We wrote one element into a ring slot.
    ElementPosted {
        /// The slot index.
        slot: u32,
        /// The element's per-message sequence number.
        seq_num: u32,
        /// `rpc.function`.
        function: u32,
        /// `rpc.sequence` — the transaction id.
        sequence: u32,
        /// `rpc.rpc_result`.
        rpc_result: u32,
        /// `rpc.length`, as declared. ★ Carried because it is exactly where the C and
        /// this port differ (GSP-D1), so the differential can name it.
        rpc_length: u32,
        /// A change detector over the body — **not** a cryptographic hash; it exists so
        /// a projection can compare payloads without carrying them.
        payload_digest: u64,
    },
    /// We published a consumption pointer.
    ReadPtrAcked {
        /// Which queue's consumption.
        queue: QueueId,
        /// The value published.
        value: u32,
    },
    /// We advanced a producer pointer.
    WritePtrAdvanced {
        /// Which queue's production.
        queue: QueueId,
        /// The value published.
        value: u32,
    },
    /// We served a register read.
    RegisterServed {
        /// Which register.
        reg: GspReg,
        /// The value served.
        value: u64,
    },
    /// We announced the status queue.
    IrqRaised,
    /// ★ Rust-only: the C has no such concept. This is the *anti*-expectation of the
    /// negative trace class — replaying the recorded stale-state bring-up must produce
    /// exactly one of these and **zero** [`Observation::ElementPosted`].
    Refused {
        /// The exact refusal variant.
        fault: FaultTag,
    },
}

impl Observation {
    /// A stable name for counting, so "which kinds were never seen" is answerable.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Observation::TxHeaderPublished { .. } => "TxHeaderPublished",
            Observation::ElementPosted { .. } => "ElementPosted",
            Observation::ReadPtrAcked { .. } => "ReadPtrAcked",
            Observation::WritePtrAdvanced { .. } => "WritePtrAdvanced",
            Observation::RegisterServed { .. } => "RegisterServed",
            Observation::IrqRaised => "IrqRaised",
            Observation::Refused { .. } => "Refused",
        }
    }
}

/// A recorded projection, with the counters that make it falsifiable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObservationLog {
    items: Vec<Observation>,
}

impl ObservationLog {
    /// An empty log.
    #[must_use]
    pub fn new() -> ObservationLog {
        ObservationLog::default()
    }

    /// Append.
    pub fn push(&mut self, obs: Observation) {
        self.items.push(obs);
    }

    /// Everything recorded, in order.
    #[must_use]
    pub fn items(&self) -> &[Observation] {
        &self.items
    }

    /// How many observations of one kind.
    #[must_use]
    pub fn count(&self, kind: &str) -> usize {
        self.items.iter().filter(|o| o.kind() == kind).count()
    }

    /// Every refusal, in order — the one-line form of the negative-trace assertion.
    #[must_use]
    pub fn refusals(&self) -> Vec<FaultTag> {
        self.items
            .iter()
            .filter_map(|o| match o {
                Observation::Refused { fault } => Some(*fault),
                _ => None,
            })
            .collect()
    }

    /// The kinds that were never observed. **The non-vacuity instrument**: an assertion
    /// over a log whose relevant kind is in this list is measuring nothing.
    #[must_use]
    pub fn unseen_kinds(&self) -> Vec<&'static str> {
        [
            "TxHeaderPublished",
            "ElementPosted",
            "ReadPtrAcked",
            "WritePtrAdvanced",
            "RegisterServed",
            "IrqRaised",
            "Refused",
        ]
        .into_iter()
        .filter(|k| self.count(k) == 0)
        .collect()
    }
}

/// A forward map from the structures we publish to their guest-physical addresses.
///
/// Built **forward** from a bound geometry — every address it knows is one we computed on
/// the way in, so a guest write we did not make cannot be mistaken for one we did. A
/// [`Projection`] deliberately has no reverse resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    layout: ElementLayout,
    element_size: u32,
    stat_hdr: Vec<u64>,
    stat_write_ptr: Vec<u64>,
    cmd_read_ptr_ack: Vec<u64>,
    stat_elements: Vec<(u32, u64)>,
}

impl Projection {
    /// Build the map for one binding.
    ///
    /// # Errors
    ///
    /// [`GspFault::RegionOutOfRange`] if the geometry does not fit its region — which
    /// [`MsgqGeometry::bind`] already refuses, so this cannot fail for a bound geometry.
    pub fn new(geom: &MsgqGeometry, layout: ElementLayout) -> Result<Projection, GspFault> {
        let starts = |off: u64, len: u64| -> Result<Vec<u64>, GspFault> {
            Ok(geom
                .region()
                .runs(off, len)?
                .into_iter()
                .map(|(g, _)| g)
                .collect())
        };
        let mut stat_elements = Vec::new();
        for i in 0..geom.msg_count().get() {
            let slot = geom.msg_count().slot(i);
            let off = geom.stat_element_off(slot);
            if let Some((gpa, _)) = geom.region().runs(off, 1)?.first() {
                stat_elements.push((i, *gpa));
            }
        }
        Ok(Projection {
            layout,
            element_size: geom.element_size(),
            stat_hdr: starts(geom.stat_write_ptr_off() - TxHeader::WRITE_PTR_OFFSET, 4)?,
            stat_write_ptr: starts(geom.stat_write_ptr_off(), 4)?,
            cmd_read_ptr_ack: starts(geom.cmd_read_ptr_ack_off(), 4)?,
            stat_elements,
        })
    }

    /// Decode one guest-RAM write into an observation, or `None` if it is not one of the
    /// structures the guest reads from us.
    #[must_use]
    pub fn classify(&self, gpa: u64, bytes: &[u8]) -> Option<Observation> {
        if self.stat_hdr.contains(&gpa) && bytes.len() >= TxHeader::BYTES {
            return TxHeader::decode(bytes)
                .ok()
                .map(|hdr| Observation::TxHeaderPublished {
                    queue: QueueId::Status,
                    hdr,
                });
        }
        if bytes.len() == 4 {
            let mut b = [0u8; 4];
            b.copy_from_slice(bytes);
            let value = u32::from_le_bytes(b);
            if self.stat_write_ptr.contains(&gpa) {
                return Some(Observation::WritePtrAdvanced {
                    queue: QueueId::Status,
                    value,
                });
            }
            if self.cmd_read_ptr_ack.contains(&gpa) {
                return Some(Observation::ReadPtrAcked {
                    queue: QueueId::Command,
                    value,
                });
            }
        }
        if bytes.len() == self.element_size as usize {
            let slot = self
                .stat_elements
                .iter()
                .find(|(_, g)| *g == gpa)
                .map(|(s, _)| *s)?;
            return decode_posted(&self.layout, slot, bytes);
        }
        None
    }
}

fn decode_posted(layout: &ElementLayout, slot: u32, bytes: &[u8]) -> Option<Observation> {
    let word = |off: usize| -> Option<u32> {
        let mut b = [0u8; 4];
        b.copy_from_slice(bytes.get(off..off + 4)?);
        Some(u32::from_le_bytes(b))
    };
    let hdr = layout.hdr_size();
    let seq_num = word(layout.seqnum_off())?;
    let rpc_length = word(hdr + 8)?;
    let function = word(hdr + 12)?;
    let rpc_result = word(hdr + 16)?;
    let sequence = word(hdr + 24)?;
    let body_end = (hdr + rpc_length as usize).min(bytes.len());
    let body_start = (hdr + RpcEnvelope::SIZE).min(body_end);
    Some(Observation::ElementPosted {
        slot,
        seq_num,
        function,
        sequence,
        rpc_result,
        rpc_length,
        payload_digest: digest(&bytes[body_start..body_end]),
    })
}

/// FNV-1a over a payload. A change detector, deliberately not a cryptographic hash: it
/// exists so a projection can compare bodies without carrying them, and nothing security-
/// relevant rests on it.
#[must_use]
pub fn digest(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A [`GuestRam`] that records every access into an [`ObservationLog`] and a
/// `kayfabe-trace` recorder's stream.
///
/// This is what makes a replay **hermetic** (§6.1): a stream that cannot answer a DMA
/// read is not a replay, so reads are recorded with the bytes they returned and writes
/// with the bytes written. It is also the instrument that proves E8 reads **zero** guest
/// RAM — a count, not an absence.
pub struct RecordingRam<'a> {
    inner: &'a mut dyn GuestRam,
    projection: Option<Projection>,
    /// Every observation this run produced.
    pub log: ObservationLog,
    /// How many reads were served.
    pub reads: usize,
    /// How many writes were served.
    pub writes: usize,
}

impl<'a> RecordingRam<'a> {
    /// Wrap a RAM, with no projection: reads and writes are counted but not decoded.
    pub fn new(inner: &'a mut dyn GuestRam) -> RecordingRam<'a> {
        RecordingRam {
            inner,
            projection: None,
            log: ObservationLog::new(),
            reads: 0,
            writes: 0,
        }
    }

    /// Attach a projection, so writes decode into observations.
    #[must_use]
    pub fn with_projection(mut self, p: Projection) -> RecordingRam<'a> {
        self.projection = Some(p);
        self
    }

    /// Record a refusal into the log — the class-(2) assertion target.
    pub fn refused(&mut self, fault: &GspFault) {
        self.log.push(Observation::Refused {
            fault: fault.fault_tag(),
        });
    }
}

impl GuestRam for RecordingRam<'_> {
    fn read(&mut self, gpa: u64, buf: &mut [u8]) -> Result<(), crate::fault::RamRefused> {
        self.reads += 1;
        self.inner.read(gpa, buf)
    }

    fn write(&mut self, gpa: u64, bytes: &[u8]) -> Result<(), crate::fault::RamRefused> {
        self.writes += 1;
        self.inner.write(gpa, bytes)?;
        if let Some(p) = &self.projection
            && let Some(obs) = p.classify(gpa, bytes)
        {
            self.log.push(obs);
        }
        Ok(())
    }
}

/// One admissible MUST-DIFFER entry.
///
/// All six fields are required. `guest_visible_consequence` of "none" would make the row
/// class (3) — not comparable — and `independent_oracle` is what stops a preference from
/// being filed as a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divergence {
    /// Stable id, e.g. `"GSP-D1"`.
    pub id: &'static str,
    /// The C artifact's site.
    pub c_site: &'static str,
    /// What the C does there.
    pub c_behaviour: &'static str,
    /// What this crate does instead.
    pub our_behaviour: &'static str,
    /// What a guest would notice. Never "none".
    pub guest_visible_consequence: &'static str,
    /// ogkm/nouveau `file:line`, or a measured trace. Never an aesthetic argument.
    pub independent_oracle: &'static str,
}

/// The MUST-DIFFER ledger, as data rather than as prose.
///
/// A differential harness reads this; a reviewer can check every row against its oracle;
/// and a row without a guest-visible consequence cannot be added without lying in a field
/// that is read.
///
/// # ⚠ `GSP-Dn` is the **C's** defects. Ours are `PC-Dn`.
///
/// Every row below says *"the C does X and we deliberately do not"*. It is a ledger of the
/// artifact's bugs, and `Verdict::Expected(row)` asserts the C is the thing that is wrong at
/// that site.
///
/// A **second** numbering exists and it is not this one: `PC-D1` … `PC-D7`, the
/// 2026-07-31 **protocol-compliance audit** of *our* GSP command path. Those are defects in
/// this port, each closed by a commit and a biting test:
///
/// | id | ours | where it was closed |
/// |---|---|---|
/// | `PC-D1` | a service pass consumed a command **before** answering it, so a `QueueFull` lost it | [`GspFsm::service_command_queue`] / `tests/tests/service_pass_atomicity.rs` |
/// | `PC-D2` | the consumption acknowledgement was not published when a pass faulted | the same |
/// | `PC-D3` | the `NoReply` set was short by one (`ECC_NOTIFIER_WRITE_ACK`) | [`RpcFunction::disposition`] / `tests/tests/rpc_async_set_oracle.rs` |
/// | `PC-D4` | `GET_GSP_STATIC_INFO` compared no size at all | `kayfabe_device::staticinfo` |
/// | `PC-D5` | fn 47's refusal escaped `kgspUnloadRm` and failed a successful `rmmod` | `kayfabe_device::inert` |
/// | `PC-D6` | a **false rationale** on `CONTROL_PARAMS_SIZE_OFF`, corrected | `kayfabe_device::inittables` |
/// | `PC-D7` | `sharedMemPhysAddr` as an instance discriminator — a **named trade-off**, not fixed | [`GspFsm::publish`] |
///
/// ★ The two namespaces collided once, on the day the audit landed: the audit numbered its
/// findings `D1…D7` and the first attempt at labelling them wrote `GSP-D4` for what this
/// table calls *"the C parses guest RAM on a doorbell while `q_ready` is stale"*. A reader
/// following the id would have found an unrelated row. Kept apart by name from here on.
pub const LEDGER: &[Divergence] = &[
    Divergence {
        id: "GSP-D1",
        c_site: "C:1586",
        c_behaviour: "rpc.length = 36 for a bare 32-byte header",
        our_behaviour: "32",
        guest_visible_consequence: "the declared length the peer checksums over, and the element count it derives",
        independent_oracle: "ogkm-610: src/nvidia/generated/g_rpc-message-header.h:41-52 \
                             (sizeof = 32); ogkm-580: same path, same lines, byte-identical \
                             struct — no version seam; the C artifact itself uses 32 at \
                             C:1637 and C:1655",
    },
    Divergence {
        id: "GSP-D2",
        c_site: "C:1561-1620",
        c_behaviour: "posts without ever reading the guest's status-queue readPtr",
        our_behaviour: "flow-controlled; QueueFull is a retryable refusal",
        guest_visible_consequence: "an over-posted ring overwrites unconsumed elements; the guest then reads a \
             seqNum GREATER than its rxSeqNum, for which there is no recovery branch",
        independent_oracle: "ogkm-610: msgq.c:490-496 (free space) = ogkm-580: \
                             src/common/shared/msgq/msgq.c:491-497; and ogkm-610: \
                             message_queue_cpu.c:768-782 = ogkm-580: :699-713 \
                             (recovery handles only seqNum < rxSeqNum, identically at both tags)",
    },
    Divergence {
        id: "GSP-D3",
        c_site: "C:1615",
        c_behaviour: "% s->q_msgcount with no zero guard",
        our_behaviour: "MsgCount(NonZeroU32): the divisor is a type",
        guest_visible_consequence: "SIGFPE in the device process — a guest-reachable VMM crash",
        independent_oracle: "docs/reference/mode2_bench_lifecycle.md §4 [measured]",
    },
    Divergence {
        id: "GSP-D4",
        c_site: "C:2380-2410",
        c_behaviour: "parses guest RAM as RPC elements on a doorbell while q_ready is stale, \
                      and answers NV_OK",
        our_behaviour: "QueueNotBound refusal with ZERO guest-RAM reads",
        guest_visible_consequence: "arbitrary guest memory answered as RPC — reachable by reloading the guest driver",
        independent_oracle: "docs/reference/mode2_bench_lifecycle.md §4 [measured]: 508 log \
                             lines of cmd fn=1959520414 seq=4055862830 -> echo NV_OK",
    },
    Divergence {
        id: "GSP-D5",
        c_site: "C:4255-4283",
        c_behaviour: "misclassifies the teardown STARTCPU as a re-acquire, re-latching \
                      bootargs_dumped/q_ready",
        our_behaviour: "E2: Suspending -> Halted, binding dropped by value",
        guest_visible_consequence: "the next driver life's msgqRxLink spins forever on -7 (71 064 retries observed)",
        independent_oracle: "ogkm-610: msgq.c:386-389 (-7 has exactly one cause) = \
                             ogkm-580: src/common/shared/msgq/msgq.c:387-390 + \
                             docs/reference/mode2_bench_lifecycle.md §3 [measured]",
    },
    Divergence {
        id: "GSP-D6",
        c_site: "C:3341-3350",
        c_behaviour: "advances past continuation elements without reading them",
        our_behaviour: "reads nElements, bounded by queueElementSizeMax before the read",
        guest_visible_consequence: "a multi-element command's payload is truncated",
        independent_oracle: "★ VERSION SEAM in HOW nElements is obtained, not in whether \
                             it must be. ogkm-610: message_queue_cpu.c:684-705 derives it \
                             from the declared message length (gspMsgQueueBytesToElements \
                             over queueElementHdrSize + rpc.length). ogkm-580: \
                             message_queue_cpu.c:652-658 instead READS THE FIELD, \
                             nElements = pCmdQueueElement->elemCount, and \
                             msgqRxMarkConsumed advances by that (ogkm-580: :774). Both \
                             are honoured here: the field where the layout has one, the \
                             derivation where it does not, with a refusal when the two \
                             disagree (GspFault::ElementCountMismatch)",
    },
    Divergence {
        id: "GSP-D7",
        c_site: "C:1697-1703",
        c_behaviour: "one SWGEN0 batch outstanding, globally, as a proxy for flow control",
        our_behaviour: "a correctly flow-controlled transport; per-Proc queuing above it is \
                        kayfabe-completion's, not this crate's",
        guest_visible_consequence: "completion delivery serialises across every process",
        independent_oracle: "mode2_rust_rewrite_architecture.md §4.3.2 (the ⚠8 constraint)",
    },
    Divergence {
        id: "GSP-D8",
        c_site: "C:3437, C:2387, C:1610",
        c_behaviour: "addresses the shared region as sharedMemPhysAddr + offset",
        our_behaviour: "resolves every access through the region's own page table",
        guest_visible_consequence: "a fragmented region has its queues read and written at the wrong pages",
        independent_oracle: "ogkm-610: message_queue_cpu.c:250-256 (NV_MEMORY_NONCONTIGUOUS), \
                             :295-316 (the table), :328-329 (sharedMemPA = pPageTbl[0]); \
                             ogkm-580: :228-234, :273-294, :306-307 — identical code, no seam",
    },
    Divergence {
        id: "GSP-D9",
        c_site: "C:3388-3407",
        c_behaviour: "scans at most 16 LibOS regions and stops at the first zero entry",
        our_behaviour: "bounds by the descriptor's declared maximum; a zero entry is skipped, \
                        not treated as a terminator",
        guest_visible_consequence: "a guest with more regions before RMARGS is silently not found, and the queue \
             never binds",
        independent_oracle: "src/common/uproc/os/common/include/libos_init_args.h:31-56 \
                             (MAX = 4096; no sentinel is declared) — same path and same lines at \
                             ogkm-610 and ogkm-580; the two files are byte-identical, no seam",
    },
    Divergence {
        id: "GSP-D10",
        c_site: "C:3459-3483",
        c_behaviour: "preserves the status/command sequence numbers across EVERY rebind",
        our_behaviour: "preserves them when the queue region is the same instance, and \
                        restarts at 0 when the guest allocated a new one",
        guest_visible_consequence: "a driver that reloaded its module starts at rxSeqNum 0; a preserved N > 0 \
             then arrives as a sequence number GREATER than the guest expects, which its \
             receive path has no recovery branch for at all",
        independent_oracle: "rxSeqNum is only ++'d, never assigned, at BOTH tags — \
                             ogkm-610: message_queue_cpu.c:836, ogkm-580: :782 (grep of both \
                             whole trees finds no other write). Its storage is GspMsgQueuesInit's \
                             portMemSet-to-0 (ogkm-610: message_queue_cpu.c:235, ogkm-580: :216), \
                             reached from kgspConstructEngine via \
                             _kgspInitRpcInfrastructure (ogkm-610: kernel_gsp.c:4355, \
                             ogkm-580: :3624) and torn down in kgspDestruct \
                             (ogkm-610: :5323, ogkm-580: :4384), i.e. module load/unload. \
                             The recovery asymmetry is ogkm-610: message_queue_cpu.c:768-782 = \
                             ogkm-580: :699-713. ★ ONE SEAM, and it does not change the row: at \
                             610 the ++ is unconditional at the exit: label; at 580 it is in the \
                             else arm of msgqRxMarkConsumed, so a failed mark-consumed leaves it \
                             put. ★ UNMEASURED: the C never reached a second driver life \
                             (mode2_bench_lifecycle.md §1), so this is the plan's open item O3 \
                             and S7 is its falsifier",
    },
];

impl Faulted for GspFault {
    /// Exhaustive by construction: a new [`GspFault`] variant fails this build until it is
    /// named, which is what keeps the trace vocabulary from drifting away from the code it
    /// describes (`testing_doctrine.md` §6.1, *"prefer a mechanism to a sentence"*).
    fn fault_tag(&self) -> FaultTag {
        match self {
            GspFault::NoGspModel => FaultTag("GspFault::NoGspModel"),
            GspFault::RegisterUnserviceable { .. } => FaultTag("GspFault::RegisterUnserviceable"),
            GspFault::BootStepOverflow => FaultTag("GspFault::BootStepOverflow"),
            GspFault::GuestRam(_) => FaultTag("GspFault::GuestRam"),
            GspFault::QueueNotBound => FaultTag("GspFault::QueueNotBound"),
            GspFault::GeometryRejected(_) => FaultTag("GspFault::GeometryRejected"),
            GspFault::MsgCountZero => FaultTag("GspFault::MsgCountZero"),
            GspFault::SwapRxNotAgreed { .. } => FaultTag("GspFault::SwapRxNotAgreed"),
            GspFault::PeerReadPtrOutOfRange { .. } => FaultTag("GspFault::PeerReadPtrOutOfRange"),
            GspFault::PeerWritePtrOutOfRange { .. } => FaultTag("GspFault::PeerWritePtrOutOfRange"),
            GspFault::QueueFull { .. } => FaultTag("GspFault::QueueFull"),
            GspFault::MsgLenOutOfRange { .. } => FaultTag("GspFault::MsgLenOutOfRange"),
            GspFault::ElementCountOutOfRange { .. } => FaultTag("GspFault::ElementCountOutOfRange"),
            GspFault::ElementCountMismatch { .. } => FaultTag("GspFault::ElementCountMismatch"),
            GspFault::ChecksumMismatch { .. } => FaultTag("GspFault::ChecksumMismatch"),
            GspFault::SeqNumGap { .. } => FaultTag("GspFault::SeqNumGap"),
            GspFault::TransportHeaderInvalid { .. } => FaultTag("GspFault::TransportHeaderInvalid"),
            GspFault::Truncated { .. } => FaultTag("GspFault::Truncated"),
            GspFault::RegionMalformed(_) => FaultTag("GspFault::RegionMalformed"),
            GspFault::RegionOutOfRange { .. } => FaultTag("GspFault::RegionOutOfRange"),
            GspFault::Envelope(_) => FaultTag("GspFault::Envelope"),
            GspFault::NotRunning => FaultTag("GspFault::NotRunning"),
            GspFault::Layout(_) => FaultTag("GspFault::Layout"),
            GspFault::DuplicateFunctionCode { .. } => FaultTag("GspFault::DuplicateFunctionCode"),
            GspFault::RmargsRegionAbsent { .. } => FaultTag("GspFault::RmargsRegionAbsent"),
            GspFault::DeclaredHeaderSizeMismatch { .. } => {
                FaultTag("GspFault::DeclaredHeaderSizeMismatch")
            }
        }
    }
}

/// Compile-time witness that a [`RegionMap`] is what the projection is built over — the
/// forward map, never a reverse resolver.
const _: fn(&RegionMap) -> u64 = RegionMap::len;

kayfabe_util::assert_send_sync!(Observation, ObservationLog, Projection, Divergence, QueueId);
