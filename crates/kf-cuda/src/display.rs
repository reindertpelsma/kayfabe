//! ★ The display plane's own GPU context — how the emulated display engine reaches the guest's
//! framebuffer (the store) WITHOUT the CPU (`THE_CONSTRAINTS.md` §38: *"the CPU never reads guest
//! vidmem"*; `docs/design/V3_DISPLAY.md` §4.3–§4.6).
//!
//! The display engine is an emulated device, and the store is its memory: the context-DMA table in
//! the guest's display instance memory (FB) is read here, a video-memory notifier or semaphore is
//! written here, and (M2) a flipped surface is copied out of the store here by the GPU. Every access
//! is bounds-checked against the imported store before a byte moves.
//!
//! ★ `v3-sec-rawaddr` (2026-10-04, audit S1-03/S1-04): this file is DATA — the compose program
//! ([`ComposeLayer`]) and the committed PTX. The context, the store, the staging frame, the console
//! frames and every check that guards them are the perimeter's
//! (`driver_unsafe/display_gpu_unsafe.rs`, re-exported below): [`DisplayGpu`], [`Composer`],
//! [`ConsoleFrame`]. No address is held here; a console frame leaves as an opaque
//! `kf_linux_raw::StaticSpan`, never as a number (`THE_CONSTRAINTS.md` §13).
//!
//! ⊘ A SEPARATE context from the walker's (`walk.rs`): the walker is owned by the VA thread and its
//! synchronous accesses order behind walks; the display worker must never wait on a walk, and a
//! walk must never wait on a scanout copy.

pub use crate::driver_unsafe::display_gpu_unsafe::{
    Composer, ConsoleFrame, DisplayGpu, compose_layer_fits,
};

/// ★ The display plane's kernels, hand-written PTX (`cuda/display/kf_scanout.ptx`), JIT-compiled at
/// the plane's bring-up; the block-linear address function is `kf_disp::scanout::bl_offset`.
pub static SCANOUT_PTX: &[u8] = include_bytes!("../../../cuda/display/kf_scanout.ptx");

/// The compose kernel's entry point: one window into the head's staging frame.
pub const COMPOSE_ENTRY: &str = "kf_compose";

/// ★ One window's compose-kernel program (the mirror of `kf_disp::scanout::LayerPlan`). Plain
/// data: [`Composer::layer`] checks it ([`compose_layer_fits`], V10) — every read bounded by
/// `extent` inside the store and every write by the composition — before it is queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposeLayer {
    /// Store offset of the first byte the kernel addresses.
    pub src: u64,
    /// Bytes from `src` the kernel may read.
    pub extent: u64,
    /// Block-linear (else pitch).
    pub block_linear: bool,
    /// Pitch in bytes, or GOBs per row.
    pub pitch: u32,
    /// log2 GOBs per block.
    pub block_height_log2: u32,
    /// The rectangle's first byte column (block-linear).
    pub x0_bytes: u32,
    /// Its first row (block-linear).
    pub y0: u32,
    /// Pixels per row.
    pub width: u32,
    /// Rows.
    pub rows: u32,
    /// Output column in the frame.
    pub ox: u32,
    /// Output row.
    pub oy: u32,
    /// `bit0` alpha, `bit1` swap red/blue, `bit2` opaque.
    pub flags: u32,
    /// Blend coefficients (see `kf_compose`).
    pub a_s: i32,
    /// See `a_s`.
    pub b_s: i32,
    /// See `a_s`.
    pub a_d: i32,
    /// See `a_s`.
    pub b_d: i32,
}
