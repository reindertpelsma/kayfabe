//! Replay a recorded C capture against [`kayfabe_gsp::GspFsm`], and project **both** sides
//! into the vocabulary a differential can compare.
//!
//! ## The unit of replay is a TRANSACTION, not a record
//!
//! The C device is single-threaded and every guest-RAM access it makes is *caused by* a
//! trapped MMIO access. So the stream decomposes exactly into
//!
//! > one MMIO record, followed by every guest-RAM read, guest-RAM write, IRQ raise and
//! > clock reading it caused, up to the next MMIO record.
//!
//! That is the natural replay unit, and it is also the natural **resync point**: the
//! guest's next access is an input we replay rather than something we produce, so a
//! divergence inside one transaction does not have to cascade through every later one.
//! `c_rust_trace_differential.md` §1 states the same idea from the other end — *"if the
//! guest's access sequence diverges at access N, our reply to N−1 differed"*.
//!
//! Two diffs are therefore reported, both through [`kayfabe_trace::diff`]:
//!
//! - the **global** one over the whole projection, positional and cascade-prone, which is
//!   what §6.3 literally specifies;
//! - the **per-transaction** one, which is what makes a *census* possible.
//!
//! ## What is projected, and what is deliberately not
//!
//! A transaction is projected **only if its driving MMIO access decodes to a
//! [`GspReg`]** — symmetrically, on both sides. That rule is declared rather than
//! discovered, and it has to be, because `cap1` contains three other planes:
//!
//! | driving register | records | plane |
//! |---|---|---|
//! | `NV_PGSP_QUEUE_HEAD(0)` `0x110c00`, `MAILBOX1` `0x110044`, GSP `CPUCTL` `0x110100` | 934 | **ours** |
//! | `NV_VIRTUAL_FUNCTION_DOORBELL` `0xbb0090` | 66 | the channel/pushbuffer plane — `kayfabe-fwd`'s, not this crate's |
//! | `PTIMER` `0xbb0080/84`, `INTR_*` `0xb816xx` | 66 | clock and the CPU interrupt tree |
//!
//! Excluding a plane this crate does not implement is not sampling: [`kayfabe_trace::diff`]
//! is positional and the same predicate is applied to both streams, so no index shifts.
//! What it *is* is a limit, and [`ReplayResult::unprojected`] reports it as a number.
//!
//! ## Never sampled, never capped
//!
//! Every record is walked. Guest-RAM reads from **unprojected** transactions are still
//! installed into the oracle, because they are still ground truth about guest memory.

use kayfabe_arch::Arch;
use kayfabe_arch::gsp::GspReg;
use kayfabe_arch::ids::Gpa;
use kayfabe_gsp::{
    BootPhase, EchoOk, GspAbi, GspFault, GspFsm, Observation, Projection, QueueState, RpcCommand,
    Transition,
};
use kayfabe_trace::{Bar, IrqSpec, TraceEvent, Width};

use crate::format::{CKind, CTrace};
use crate::ga10x::Ga10xArch;
use crate::oracle::{Answer, OracleRam, ReconKind, Reconstruction, Unobserved};

/// What one projected entry *is*, kept beside the [`TraceEvent`] the differential
/// compares so a divergence can be reported in decoded terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// A register read, with the register it named.
    Register(GspReg),
    /// A guest-RAM write the projection decoded.
    Decoded(Observation),
    /// A guest-RAM write the projection could **not** decode — bytes at an address that
    /// is none of the structures the guest reads from us. Never silently dropped.
    Undecoded {
        /// Where.
        gpa: u64,
        /// How many bytes.
        len: usize,
    },
    /// An interrupt raise.
    Irq,
}

/// One side's decoded projection: the events a differential compares, the note for each,
/// and which transaction each came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Projected {
    /// What [`kayfabe_trace::diff`] compares.
    pub events: Vec<TraceEvent>,
    /// Aligned with [`Projected::events`].
    pub notes: Vec<Note>,
    /// Aligned with [`Projected::events`]: the index into [`ReplayResult::txns`].
    pub txn: Vec<usize>,
}

impl Projected {
    fn push(&mut self, txn: usize, ev: TraceEvent, note: Note) {
        self.events.push(ev);
        self.notes.push(note);
        self.txn.push(txn);
    }

    /// How many entries of each note kind — the non-vacuity instrument. An assertion over
    /// a projection whose relevant kind counts zero is measuring nothing.
    #[must_use]
    pub fn census(&self) -> Vec<(&'static str, usize)> {
        let mut out: Vec<(&'static str, usize)> = Vec::new();
        for n in &self.notes {
            let k = match n {
                Note::Register(_) => "Register",
                Note::Decoded(o) => o.kind(),
                Note::Undecoded { .. } => "Undecoded",
                Note::Irq => "Irq",
            };
            match out.iter_mut().find(|(name, _)| *name == k) {
                Some((_, c)) => *c += 1,
                None => out.push((k, 1)),
            }
        }
        out.sort_unstable();
        out
    }
}

/// One replayed transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Txn {
    /// Index of the driving MMIO record in the capture, if it had one.
    pub driver: Option<usize>,
    /// The driving register, if our model decodes the offset.
    pub reg: Option<GspReg>,
    /// The driving offset.
    pub off: u64,
    /// The value read or written.
    pub val: u64,
    /// Was it a write?
    pub write: bool,
    /// Half-open span in the C projection.
    pub c: (usize, usize),
    /// Half-open span in the Rust projection.
    pub r: (usize, usize),
    /// Transitions the FSM reported.
    pub transitions: Vec<Transition>,
    /// The refusal, if the FSM refused this write. Carried whole, not as a tag: the
    /// payload is the diagnosis (which address, which checksum, which count).
    pub refusal: Option<GspFault>,
    /// Half-open span into [`ReplayResult::read_log`]: every guest-RAM read this
    /// transaction made, and how each was answered.
    pub reads: (usize, usize),
}

/// Everything one replay measured.
#[derive(Debug)]
pub struct ReplayResult {
    /// The C's projection.
    pub c: Projected,
    /// Ours.
    pub rust: Projected,
    /// Every transaction, in order.
    pub txns: Vec<Txn>,
    /// Transactions whose driving MMIO access our register model does not decode, and the
    /// guest-RAM writes and IRQ raises inside them — the plane this differential does not
    /// cover, as a number rather than a caveat.
    pub unprojected: Unprojected,
    /// How each guest-RAM read was answered.
    pub answers: Vec<(Answer, usize)>,
    /// Reads nothing could answer, each with the transaction that made it.
    pub unobserved: Vec<(usize, Unobserved)>,
    /// The furthest any lookahead reached, in records.
    pub max_lookahead: usize,
    /// ★ The first transaction the capture could not carry the replay through: the
    /// **closure limit**. `None` means the whole capture replayed.
    pub closure_limit: Option<usize>,
    /// The reconstructions this run needed.
    pub reconstructions: Vec<Reconstruction>,
    /// The phase the FSM finished in.
    pub final_phase: BootPhase,
    /// Every distinct transition that fired, for non-vacuity.
    pub transitions_seen: Vec<Transition>,
    /// Every guest-RAM read, in order: where, how big, and how it was answered. The
    /// hermeticity evidence in raw form.
    pub read_log: Vec<(u64, usize, Answer)>,
    /// ★★ Every command the guest sent that this run **decoded and acted on**, with the
    /// transaction it arrived in.
    ///
    /// Reported because a reply-plane assertion is otherwise unfalsifiable in the direction
    /// that matters: *"our fn-76 answer matched the C's"* means nothing without evidence
    /// that a fn-76 arrived at all, and which one. `docs/design/c_rust_trace_differential.md`
    /// §6.3's census compares what was *posted*; this is what provoked it.
    pub commands: Vec<(usize, RpcCommand)>,
}

/// The part of the capture this differential does not cover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Unprojected {
    /// Transactions whose driving register our model does not decode.
    pub txns: usize,
    /// Guest-RAM writes inside them.
    pub guest_writes: usize,
    /// IRQ raises inside them.
    pub irqs: usize,
    /// Guest-RAM reads inside them — still installed into the oracle.
    pub guest_reads: usize,
}

/// How aggressively the oracle may answer a read the capture does not contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    /// Only what the C observed. A read outside that set is refused — this is the
    /// **hermeticity measurement**, and it is the mode that establishes the finding.
    Observed,
    /// …plus a later observation of the same `(gpa, len)` in the same capture.
    Lookahead,
    /// …plus reconstructions, discovered by re-running: each unanswerable read is filled
    /// under a named assumption and the run repeated, up to [`Replay::MAX_ROUNDS`] times.
    /// The reconstruction list is a **finding**, not a configuration.
    Reconstructed,
}

/// How a run builds the command policy the FSM answers with.
///
/// ★★ A **factory**, not a value, and that is forced rather than stylistic:
/// [`Fill::Reconstructed`] re-runs the whole capture up to [`Replay::MAX_ROUNDS`] times, and
/// a policy carried across rounds would answer round *n* with state accumulated in round
/// *n − 1*. A `fn` pointer keeps [`Replay`] `Send`/`Sync` and keeps every round identical.
pub type PolicyFactory = fn() -> Box<dyn kayfabe_gsp::CommandPolicy>;

/// The replay.
pub struct Replay<'a> {
    trace: &'a CTrace,
    abi: GspAbi,
    arch: Ga10xArch,
    policy: PolicyFactory,
}

impl<'a> Replay<'a> {
    /// How many times [`Fill::Reconstructed`] may add a reconstruction and re-run. Small
    /// and fixed: a run that needs more than this is not converging and the harness must
    /// say so rather than iterate forever.
    pub const MAX_ROUNDS: usize = 8;

    /// A replay of `trace` against a GSP built from `abi`.
    ///
    /// Axis A is the caller's (`kayfabe_abi::versions`), Axis B is [`Ga10xArch`] — neither
    /// is hard-coded into the replay itself.
    #[must_use]
    pub fn new(trace: &'a CTrace, abi: GspAbi) -> Replay<'a> {
        Replay {
            trace,
            abi,
            arch: Ga10xArch::new(),
            policy: || Box::new(EchoOk),
        }
    }

    /// Replace the C-baseline echo with the policy a real guest is answered by.
    ///
    /// ★★★ **This is the difference between exercising the reply plane and not.** With the
    /// default [`EchoOk`] the replay reproduces the C's own *"acknowledge everything"*
    /// baseline, which is the right null model for measuring the transport — and it means
    /// no served control's *body* is ever produced, so no served control's body is ever
    /// differenced. `kayfabe_device::served_policy` is the chain a guest gets; handing it
    /// here is what makes a regression in one of those replies turn a differential red.
    #[must_use]
    pub fn with_policy(self, policy: PolicyFactory) -> Replay<'a> {
        Replay { policy, ..self }
    }

    /// Run it.
    #[must_use]
    pub fn run(&self, fill: Fill) -> ReplayResult {
        let mut recon: Vec<Reconstruction> = Vec::new();
        let mut rounds = 0;
        loop {
            let out = self.run_once(fill != Fill::Observed, &recon);
            if fill != Fill::Reconstructed || out.unobserved.is_empty() {
                return out;
            }
            let (at, first) = out.unobserved[0];
            let Some(next) = Replay::propose(out.txns.get(at), first) else {
                return out;
            };
            if recon.contains(&next) || rounds >= Replay::MAX_ROUNDS {
                return out;
            }
            recon.push(next);
            rounds += 1;
        }
    }

    /// The reconstruction an unanswerable read calls for, or `None` if none of the named
    /// assumptions applies — in which case the harness reports the read and stops, rather
    /// than inventing a category.
    /// ★★ **A reconstruction is only admissible for a read the BIND makes.** That is a
    /// structural rule, not a size heuristic: E6 is the one place our implementation reads
    /// addresses the C's addressing scheme elided — the region's page table (GSP-D8) and,
    /// at bind time, the peer's status-queue read pointer (GSP-D2). Every other
    /// unanswerable read is ring traffic, no named assumption covers it, and the honest
    /// answer is the closure limit rather than an invention.
    ///
    /// Without this rule the harness would happily "reconstruct" a skipped continuation
    /// element as a contiguous page table — which is not a value, it is noise with a
    /// justification attached.
    fn propose(txn: Option<&Txn>, u: Unobserved) -> Option<Reconstruction> {
        let bind = txn.is_some_and(|t| {
            matches!(
                t.reg,
                Some(GspReg::GspFalconMailbox0 | GspReg::GspFalconMailbox1)
            )
        });
        if !bind {
            return None;
        }
        let kind = match u.len {
            // The `pageTableEntryCount` × 8-byte table `RegionMap::load` reads at
            // `sharedMemPhysAddr`. GSP-D8: the C computes offsets from that address
            // instead of reading it, so no capture can contain it.
            n if n >= 16 && n.is_multiple_of(8) => ReconKind::RegionPageTable,
            // The peer's status-queue read pointer. GSP-D2: the C has no flow control and
            // never reads it.
            4 => ReconKind::PeerStatusReadPtr,
            _ => return None,
        };
        Some(Reconstruction {
            gpa: u.gpa,
            len: u.len,
            kind,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn run_once(&self, lookahead: bool, recon: &[Reconstruction]) -> ReplayResult {
        let recs = self.trace.records();
        let reads: Vec<(usize, u64, Vec<u8>)> = recs
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == CKind::GuestRead)
            .map(|(at, r)| (at, r.a, r.payload.clone()))
            .collect();
        let mut ram = OracleRam::new(reads, lookahead);
        for r in recon {
            ram.reconstruct(*r);
        }

        let mut fsm = GspFsm::new(self.abi);
        let mut policy = (self.policy)();
        let mut projection: Option<Projection> = None;

        let mut c = Projected::default();
        let mut rust = Projected::default();
        let mut txns: Vec<Txn> = Vec::new();
        let mut unprojected = Unprojected::default();
        let mut transitions_seen: Vec<Transition> = Vec::new();
        let mut unobserved: Vec<(usize, Unobserved)> = Vec::new();
        let mut commands: Vec<(usize, RpcCommand)> = Vec::new();

        let is_mmio = |k: CKind| matches!(k, CKind::MmioRead | CKind::MmioWrite);

        let mut i = 0usize;
        while i < recs.len() {
            let head = &recs[i];
            let driver = is_mmio(head.kind).then_some(i);
            let body_start = if driver.is_some() { i + 1 } else { i };
            let mut j = body_start;
            while j < recs.len() && !is_mmio(recs[j].kind) {
                j += 1;
            }

            // ── the oracle sees the C's reads for this transaction, before we run ──
            for r in &recs[body_start..j] {
                if r.kind == CKind::GuestRead {
                    ram.observe(r.a, &r.payload);
                }
            }

            let Some(d) = driver else {
                // A driverless prologue: nothing produced it, so there is nothing to
                // compare. Its reads are already installed.
                i = j;
                continue;
            };

            let reg = self.arch.gsp().and_then(|m| m.decode_reg(head.bar, head.a));
            let Some(reg) = reg else {
                unprojected.txns += 1;
                for r in &recs[body_start..j] {
                    match r.kind {
                        CKind::GuestWrite => unprojected.guest_writes += 1,
                        CKind::GuestRead => unprojected.guest_reads += 1,
                        CKind::Irq => unprojected.irqs += 1,
                        _ => {}
                    }
                }
                i = j;
                continue;
            };

            let txn = txns.len();
            let c0 = c.events.len();
            let r0 = rust.events.len();
            let write = head.kind == CKind::MmioWrite;
            let size = Width::from_bytes(head.width).unwrap_or(Width::B4);
            let mut transitions = Vec::new();
            let mut refusal = None;
            let writes_before = ram.writes.len();
            let reads_before = ram.answers.len();
            let unobserved_before = ram.unobserved.len();
            ram.seek(d);

            // ── our side ──
            if write {
                let res = fsm.mmio_write(
                    &mut ram,
                    &self.arch,
                    policy.as_mut(),
                    head.bar,
                    head.a,
                    head.b,
                );
                let mut raise_irq = false;
                match res {
                    Ok(report) => {
                        transitions.clone_from(&report.transitions);
                        for t in &report.transitions {
                            if !transitions_seen.contains(t) {
                                transitions_seen.push(*t);
                            }
                        }
                        raise_irq = report.raise_status_irq;
                        commands.extend(report.commands.iter().map(|c| (txn, c.clone())));
                    }
                    Err(f) => refusal = Some(f),
                }
                // Rebuild the forward map whenever a bind could have happened, and tell
                // the oracle where our published write pointer lives.
                if let QueueState::Bound(b) = fsm.queue() {
                    let geom = b.geometry();
                    projection = Projection::new(geom, self.abi.element).ok();
                    if let Ok(runs) = geom.region().runs(geom.stat_write_ptr_off(), 4)
                        && let Some((gpa, _)) = runs.first()
                    {
                        ram.bind_pointers(*gpa);
                    }
                }
                // Our guest-RAM writes, projected through the same forward map the C's are.
                let ours: Vec<(u64, Vec<u8>)> = ram.writes[writes_before..].to_vec();
                for (gpa, bytes) in &ours {
                    let (ev, note) = project_write(projection.as_ref(), *gpa, bytes);
                    rust.push(txn, ev, note);
                }
                // The IRQ comes *after* the writes it announces — the port plan's
                // "the tx header was written before the interrupt" ordering fact.
                if raise_irq {
                    rust.push(
                        txn,
                        TraceEvent::IrqRaise {
                            spec: IrqSpec::Msix(0),
                        },
                        Note::Irq,
                    );
                }
            } else {
                match fsm.mmio_read(&self.arch, head.bar, head.a) {
                    Some(Ok(val)) => rust.push(
                        txn,
                        TraceEvent::MmioRead {
                            bar: Bar(head.bar),
                            off: head.a,
                            size,
                            val,
                        },
                        Note::Register(reg),
                    ),
                    Some(Err(f)) => refusal = Some(f),
                    // Decoded above, so `None` here is unreachable; if it ever happens the
                    // model is inconsistent with itself and it must not be silently
                    // skipped — `NoGspModel` is the loudest thing sayable here.
                    None => refusal = Some(GspFault::NoGspModel),
                }
            }

            // ── the C's side, same rule, same order ──
            if !write {
                c.push(
                    txn,
                    TraceEvent::MmioRead {
                        bar: Bar(head.bar),
                        off: head.a,
                        size,
                        val: head.b,
                    },
                    Note::Register(reg),
                );
            }
            for r in &recs[body_start..j] {
                match r.kind {
                    CKind::GuestWrite => {
                        let (ev, note) = project_write(projection.as_ref(), r.a, &r.payload);
                        c.push(txn, ev, note);
                    }
                    CKind::Irq => c.push(
                        txn,
                        TraceEvent::IrqRaise {
                            spec: if r.a == 0 {
                                IrqSpec::Msix(r.b as u16)
                            } else {
                                IrqSpec::IntxLevel(r.b != 0)
                            },
                        },
                        Note::Irq,
                    ),
                    _ => {}
                }
            }

            txns.push(Txn {
                driver: Some(d),
                reg: Some(reg),
                off: head.a,
                val: head.b,
                write,
                c: (c0, c.events.len()),
                r: (r0, rust.events.len()),
                transitions,
                refusal,
                reads: (reads_before, ram.answers.len()),
            });
            for u in &ram.unobserved[unobserved_before..] {
                unobserved.push((txn, *u));
            }
            i = j;
        }

        let closure_limit = txns
            .iter()
            .position(|t| t.refusal.is_some())
            .or_else(|| unobserved.first().map(|(t, _)| *t));
        ReplayResult {
            c,
            rust,
            txns,
            unprojected,
            answers: ram.answer_census(),
            closure_limit,
            unobserved,
            reconstructions: ram.reconstructions().to_vec(),
            final_phase: fsm.phase(),
            transitions_seen,
            max_lookahead: ram.max_lookahead(),
            read_log: ram.answers.clone(),
            commands,
        }
    }
}

/// Project one guest-RAM write into the decoded form the differential compares.
///
/// ★ §6.3: **never raw bytes.** A write the forward map recognises is replaced by a
/// canonical encoding of its *decoded fields*, so the C's zero padding, its uninitialised
/// element tails and its `rpc.length = 36` cannot be enshrined as byte equality — and so
/// `rpc.length` is still *carried*, because GSP-D1 is exactly the field a differential
/// must be able to name. A write the map does not recognise keeps its bytes and says so.
fn project_write(projection: Option<&Projection>, gpa: u64, bytes: &[u8]) -> (TraceEvent, Note) {
    match projection.and_then(|p| p.classify(gpa, bytes)) {
        Some(obs) => (
            TraceEvent::GuestWrite {
                gpa: Gpa(gpa),
                bytes: canon(&obs),
            },
            Note::Decoded(obs),
        ),
        None => (
            TraceEvent::GuestWrite {
                gpa: Gpa(gpa),
                bytes: bytes.to_vec(),
            },
            Note::Undecoded {
                gpa,
                len: bytes.len(),
            },
        ),
    }
}

/// A deterministic encoding of an [`Observation`]'s decoded fields.
///
/// It exists so the one comparison in this harness is [`kayfabe_trace::diff`] — there is
/// no second differential written here. The tag byte keeps two observation kinds from
/// ever colliding.
fn canon(obs: &Observation) -> Vec<u8> {
    let mut v = Vec::with_capacity(48);
    match obs {
        Observation::TxHeaderPublished { queue, hdr } => {
            v.push(1);
            v.push(*queue as u8);
            v.extend_from_slice(&hdr.encode());
        }
        Observation::ElementPosted {
            slot,
            seq_num,
            function,
            sequence,
            rpc_result,
            rpc_length,
            payload_digest,
        } => {
            v.push(2);
            for w in [slot, seq_num, function, sequence, rpc_result, rpc_length] {
                v.extend_from_slice(&w.to_le_bytes());
            }
            v.extend_from_slice(&payload_digest.to_le_bytes());
        }
        Observation::ReadPtrAcked { queue, value } => {
            v.push(3);
            v.push(*queue as u8);
            v.extend_from_slice(&value.to_le_bytes());
        }
        Observation::WritePtrAdvanced { queue, value } => {
            v.push(4);
            v.push(*queue as u8);
            v.extend_from_slice(&value.to_le_bytes());
        }
        Observation::RegisterServed { value, .. } => {
            v.push(5);
            v.extend_from_slice(&value.to_le_bytes());
        }
        Observation::IrqRaised => v.push(6),
        Observation::Refused { fault } => {
            v.push(7);
            v.extend_from_slice(fault.0.as_bytes());
        }
    }
    v
}
