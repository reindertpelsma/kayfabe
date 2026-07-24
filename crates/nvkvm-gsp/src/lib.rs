//! # nvkvm-gsp — the faked GSP (SKELETON this milestone)
//!
//! Will own (arch doc §4.2, richest quirk density in the project — lesson L13's
//! ledger):
//!
//! - **The falcon boot FSM**: WPR2/FWSEC/SEC2-booter mailbox latches, GSP
//!   suspend/reload across contexts, `GSP_INIT_DONE`. **Resettable in-process**
//!   (lesson L12: WPR2 state is emulator state, not hardware — kills the C-era
//!   fresh-boot-per-run tax).
//! - **The message queues**: the single shared status queue with its strictly
//!   monotonic seqNum ring (the ⚠8 transport constraint — over-posting desyncs the
//!   RPC path). This is the *transport* half of completion delivery; the *policy*
//!   half is `nvkvm-completion` and stays per-proc.
//! - **RPC decode/encode** against `nvkvm-abi`'s generated message layouts, emitting
//!   abstract `RmEvent`s / control intents into the core.
//!
//! Ports second in the migration order (§4.5 step 2 — needs NO real GPU); its
//! oracle is trace replay: record BAR0+RPC traces from the C emulator booting a
//! real guest, replay into this crate as pure input, assert identical responses.

/// Placeholder for the resettable boot FSM state (shape lands with the port).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BootPhase {
    /// Power-on / post-reset.
    #[default]
    Cold,
    /// Boot handshake in progress (FWSEC/booter latch sequence).
    Booting,
    /// `GSP_INIT_DONE` posted; RPC plane live.
    Running,
}
