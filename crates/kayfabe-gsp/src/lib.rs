//! # kayfabe-gsp — the faked GSP
//!
//! The guest runs the **stock** NVIDIA driver against an emulated GPU whose GSP does not
//! exist. This crate is that GSP's side of the conversation: the falcon boot FSM, the
//! CPU↔GSP message-queue transport, and RPC decode/encode. Its specification is
//! `docs/design/mode2_gsp_port_plan.md`; its governing directive is that file's last
//! section, from the owner:
//!
//! > *"use regular traps. build the thing C proved work if it's not bugged."*
//!
//! So the default is to **reproduce** the C artifact — a working implementation on real
//! hardware, which is evidence no amount of reasoning replaces — and every deviation
//! names the defect it corrects with a citation. The five authorised ones are the reset/
//! latch chain, RPC element parsing, msgq linear addressing, the version key, and
//! `rpc.length = 36`; each is called out at the site that implements it.
//!
//! ## What is here
//!
//! | module | stage | owns |
//! |---|---|---|
//! | [`ram`] | S1 | the guest-RAM port, and the region's **self-describing page table** |
//! | [`ring`] | S1 | `msgq` geometry, the `msgqRxLink` acceptance predicate, free/available algebra |
//! | [`element`] | S2 | element framing, the checksum, multi-element split/join |
//! | [`boot`] | S3/S4 | `BootPhase` × `QueueState`, transitions E1–E11, the transport, `device_reset` |
//! | [`rpc`] | S4 | function classification, dispositions, replies |
//! | [`replay`] | S5 | the projection a trace differential compares, and the ledger |
//!
//! ## ★★ Five things this crate is deliberately not tied to
//!
//! Each is a *seam*, and each has a test that pushes on it — a seam with one
//! implementation is decoration.
//!
//! 1. **A GPU architecture.** No offset, no bit position, no generation name lives here;
//!    they are behind `kayfabe_arch::GspModel`, reached through `Arch::gsp()`. The suite
//!    drives the same FSM through two deliberately-different models.
//! 2. **A driver version.** The element layout genuinely changed in **(595.84, 610.43.02]**
//!    — 48 bytes with `elemCount@40` versus 16 with MCTP/NVDM transport headers — so the
//!    shape is an [`element::ElementLayout`] *value* supplied by
//!    `kayfabe_abi::versions::GspElementWire`, and the wire tables are keyed on the full
//!    `major.minor.patch` with a loud [`kayfabe_abi::wire::AbiError::NoTableForVersion`]
//!    below the floor. ★ Version skew and chip variation are **different axes** and are
//!    never folded into one enum.
//! 3. **A hypervisor.** The only way in is an abstract register access and the
//!    [`ram::GuestRam`] port; no descriptor, syscall, memslot or VMM type is nameable
//!    here, and CI greps for all of them. Every stage runs with no VMM present at all.
//! 4. **A single GPU lifetime.** [`boot::GspFsm::device_reset`] rebuilds the value, and
//!    the teardown transition drops the queue binding **by value**, so idle-release →
//!    re-acquire and driver-restart both work *in-process*, repeatedly. This is the C's
//!    measured failure (`docs/reference/mode2_bench_lifecycle.md` §3) and it is also the
//!    fresh-boot-per-run bench tax.
//! 5. **A CPU architecture.** Every wire access is explicitly little-endian
//!    (`from_le_bytes`/`to_le_bytes`; there is no `_ne_` or `_be_` conversion in the
//!    crate), no type's layout is assumed, and the one page size that appears is the
//!    *driver's* `RM_PAGE_SIZE` carried in [`ring::MsgqAbi`] — never the host's, which on
//!    an arm64 box is a different number.
//!
//! ## ★★ The fourth axis: the guest **OS** is not the guest **driver version**
//!
//! Three axes were already named — GPU architecture (`kayfabe-arch`), *guest* driver
//! version (`kayfabe-abi::DriverVersion`), and *host* driver version (abstract by
//! construction, since `VerbPlan` is named intents the adapter lowers). The fourth is the
//! guest **operating system**, and today it lives nowhere: `DriverVersion` silently means
//! "Linux, ogkm-shaped". Collapsing it into the version key would be the C's major-only
//! mistake one level up, so this crate states which of its assumptions belong to which
//! axis instead. Nothing here implements a second OS; the point is not to foreclose one.
//!
//! **Protocol-universal — true of any GSP-era guest, Windows included.** The `msgq` ring
//! algebra, the tx/rx header shape and the `MSGQ_FLAGS_SWAP_RX` topology, element framing,
//! the checksum, the per-message `seqNum` discipline, `bytesToElements`, the RPC envelope
//! and its `(function, sequence)` matching, the flow-control bound, `GSP_INIT_DONE`, and
//! fn-47's synchronousness. The reason is structural rather than optimistic: all of it
//! lives below the per-OS layer, in NVIDIA's **OS-independent RM core** — the per-OS
//! layer (ioctl versus escape) sits *above* it, and the GSP firmware on the other side is
//! the same binary either way.
//!
//! The *directory* that core occupies is itself version-dependent, so it is named per
//! tag: `ogkm-610: src/nvidia/` holds all of it, including `src/nvidia/src/libraries/msgq/`;
//! at `ogkm-580:` the RM core is `src/nvidia/` **but the `msgq` library is not in it** —
//! it is `src/common/shared/msgq/` (headers under `src/common/shared/msgq/inc/msgq/`).
//! Same code, different tree position — the two `msgq.c` files are byte-identical apart
//! from their `#include` preamble, with a uniform 580 = 610 + 1 line offset through the
//! body — so the argument from placement is unaffected, and if anything strengthened:
//! `src/common/shared/` is *more* OS-independent than `src/nvidia/`, not less.
//!
//! **RM-shaped, so it varies with the driver *version* and not with the OS.**
//! `MESSAGE_QUEUE_INIT_ARGUMENTS` (three shapes across r535/r570/610), the LibOS region
//! array, the boot-args mailbox pair, the FWSEC → Booter-Load ordering, the bootup-window
//! event allowlist, and the async set `{72, 73}`. These are already on the version axis,
//! which is where they belong.
//!
//! **Genuinely ogkm-shaped — the future Windows seam, such as it is.** Exactly three:
//!
//! 1. that the guest publishes **one** command/status queue pair described by one
//!    init-args struct (r535 also declares a *lockless* pair, which this port ignores);
//! 2. that the init-args region is found by the `RMARGS` id8 — a driver that named it
//!    differently is not found, and says so ([`GspFault::RmargsRegionAbsent`]);
//! 3. that the boot-args pointer arrives through the falcon mailbox pair — which is
//!    already the *architecture* axis (Hopper+ uses an entirely different path).
//!
//! **Out of scope, by design and loudly.** Drift is supported across *ogkm-like bootstrap
//! sequences* only. A guest whose bootstrap is something else terminates in a named
//! refusal rather than a best-effort attempt: a pre-GSP driver never writes the boot-args
//! mailboxes, so nothing ever binds and every doorbell is [`GspFault::QueueNotBound`]; a
//! different queue handshake fails the acceptance predicate and is
//! [`GspFault::GeometryRejected`] carrying the guest's own rejection code.
//!
//! ★ And the rule that keeps this true: **every transition fires on what the guest DID** —
//! wrote this register, completed this mailbox pair, published this header, posted this
//! element — and never on driver identity. There is no `if version == …` in this crate,
//! and the boot path is exactly where one would be most tempting.
//!
//! ## Concurrency
//!
//! Every public type is `Send + Sync` (decision #17), asserted at the bottom of each
//! module. The FSM is a plain value: it holds no lock, spawns nothing, and reads no
//! clock, so a shell that owns it decides its own serialisation.

pub mod boot;
pub mod element;
pub mod fault;
pub mod ram;
pub mod replay;
pub mod ring;
pub mod rpc;
pub mod seq;

pub use boot::{
    CommandObserver, CommandPolicy, EchoOk, GspAbi, GspFsm, InitArgsLayout, Observing, PolicyChain,
    QueueBinding, QueueState, Reply, ServiceReport, Transition, Unserviced,
};
// ★ Re-exported, not defined here (task #121): `BootPhase` is read by the register model
// and by the boot sequence, neither of which may depend on this crate. Every existing
// `kayfabe_gsp::BootPhase` path still resolves.
pub use element::{
    ElementLayout, IncomingRpc, MsgLen, OutgoingRpc, TransportHdr, bytes_to_elements, checksum32,
    decode_message, encode_message, max_elements, peek_elem_count, peek_len,
};
pub use fault::{GspFault, LayoutError, RamRefused, RegionError, RxLinkCode};
pub use kayfabe_arch::gsp::BootPhase;
pub use ram::{GuestRam, RegionMap};
pub use replay::{
    Divergence, LEDGER, Observation, ObservationLog, Projection, QueueId, RecordingRam, digest,
};
pub use ring::{
    MsgCount, MsgqAbi, MsgqGeometry, RX_HEADER_BYTES, RxCursor, Slot, TxCursor, TxHeader,
    available_elements, free_elements, rx_link_check,
};
pub use rpc::{Disposition, FunctionCodes, RpcAbi, RpcCommand, RpcFunction};
pub use seq::FalconSecureBooterBoot;
