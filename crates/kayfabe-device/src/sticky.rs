//! ★★★ **The sticky answer: which of our replies the guest keeps FOREVER, and the one
//! seam that can stop it.**
//!
//! A reply we post is normally a one-shot: get it wrong and the next boot gets it right.
//! For some controls it is not. The guest's RM keeps a **control cache** keyed by
//! `(gpuInstance, cmd)` and consults it *before* the RPC, so a cached answer means the
//! command **never reaches us again** — not this boot, and not after we fix the code. A
//! wrong answer of that class cannot be corrected by editing this repository.
//!
//! ## ★★★ 1. The universe is DERIVED, not listed — three greps over the guest's source
//!
//! **(1a) Which call sites populate the cache from an RPC reply.** Grep the whole guest
//! driver for the two setters. `ogkm-580` has exactly three hits and `ogkm-610` exactly
//! three, and they are the same three:
//!
//! | site | reachable from a reply? |
//! |---|---|
//! | `rmapi/control.c:301` (`ogkm-610: :298`) | **no** — the LOCAL control path, cached under `RMCTRL_API_COPY_FLAGS_SET_CONTROL_CACHE` by CPU-RM answering itself. No RPC is involved and no byte of ours reaches it. |
//! | `vgpu/rpc.c:11097` (`ogkm-610: :10902`) | **yes** — branch (a) below |
//! | `vgpu/rpc.c:11102` (`ogkm-610: :10907`) | **yes** — branch (b) below |
//!
//! Both reply-side sites sit inside **`rpcRmApiControl_GSP`**. ⇒ ★★ **Only a
//! `GSP_RM_CONTROL` (fn 76) reply can ever become sticky.** Every policy that answers some
//! other function carries its argument by construction and needs no guard — and that is a
//! fact about the guest's source, not a reading of ours.
//!
//! **(1b) What decides each branch.** `ogkm-580: rpc.c:11093-11104`, `ogkm-610: :10898-10909`
//! — character-identical at both tags:
//!
//! ```text
//! if (rpc_params->status != NV_OK) status = rpc_params->status;
//! else {
//!     if (bCacheable)
//!         rmapiControlCacheSet(hClient, hObject, cmd, rpc_params->params, paramsSize);
//!     else if (IsGssLegacyCall(cmd) && !(resCtrlFlags & NVOS54_FLAGS_FINN_SERIALIZED) &&
//!          rmapiControlIsCacheable(rpc_params->rmctrlFlags, rpc_params->rmctrlAccessRight, NV_TRUE) &&
//!          !(rpc_params->rmctrlFlags & RMCTRL_FLAGS_CACHEABLE_BY_INPUT))
//!         rmapiControlCacheSetUnchecked(hClient, hObject, cmd, rpc_params->params,
//!                                       paramsSize, rpc_params->rmctrlFlags);
//! }
//! ```
//!
//! - **branch (a), `bCacheable`** — computed **before the RPC is sent**, from the guest's
//!   own NVOC export table: `rmapiutilGetControlInfo(cmd, &ctrlFlags, &ctrlAccessRight, NULL)`
//!   then `rmapiControlIsCacheable(ctrlFlags, ctrlAccessRight, NV_TRUE)`
//!   (`ogkm-580: rpc.c:10900-10913`, `ogkm-610: :10705-10718`). ⊘ **Nothing in our reply
//!   appears in that decision.** See §3.
//! - **branch (b), `IsGssLegacyCall`** — bit 15 of the command word
//!   (`ogkm-580: src/nvidia/interface/deprecated/rmapi_deprecated.h:41`, `IsGssLegacyCall`
//!   at `rmapi_deprecated_control.c:95-98`) — and then a predicate over
//!   **`rpc_params->rmctrlFlags` / `rpc_params->rmctrlAccessRight`**, which are fields of
//!   the **REPLY**. Those are ours. §2.
//!
//! **(1c) What `rmapiControlIsCacheable` actually tests** (`ogkm-580:
//! src/nvidia/src/kernel/rmapi/rmapi_cache.c:152-172`, `ogkm-610: :152-172` — identical):
//! it returns `NV_FALSE` unless `flags & RMCTRL_FLAGS_CACHEABLE_ANY`, `NV_FALSE` when
//! `accessRight != 0`, and `bAllowInternal` (`NV_TRUE` on this path) when
//! `flags & RMCTRL_FLAGS_INTERNAL`. See [`reply_would_be_cached`].
//!
//! ## ★★ 2. Branch (b) is the one we own, and the guard here closes it for EVERY policy
//!
//! ⚠ **CORRECTION, and it is the load-bearing kind.** [`crate::inittables`]'s
//! `RM_GSS_LEGACY_MASK` doc said *"Our replies **reflect** the request's `rmctrlFlags` …
//! so for a GSS-legacy control the guest would cache whatever we answered"*. Reading the
//! send path shows the opposite: `rpcRmApiControl_GSP` assigns
//! `rpc_params->rmctrlFlags = 0; rpc_params->rmctrlAccessRight = 0;` before every send
//! (`ogkm-580: rpc.c:10994-10995`, `ogkm-610: :10799-10800` — the offsets
//! `DriverAbiTable::decode_rpc_control` already cites). So a reflected header carries
//! **zero**, `rmapiControlIsCacheable(0, 0, …)` fails its very first test, and the
//! reflection is what SAVES an echo rather than what endangers it. The old sentence made a
//! true guard rest on a false mechanism, which is exactly the PC-D6 shape.
//!
//! ⊘ That does **not** make the exposure imaginary, and the difference is who is trusted.
//! `rmctrlFlags` is a field on a message **the guest wrote**, and this port answers a
//! *hostile* guest, not a stock one. A guest that pre-sets `RMCTRL_FLAGS_CACHEABLE` in the
//! request gets it echoed back by any policy that clones the request header — and the
//! guest's own RM then persists our answer for the rest of the driver's life against a
//! `(gpuInstance, cmd)` key it will never re-ask on. The old reasoning reached the right
//! guard through a mechanism that does not exist; the real one is a reflected
//! guest-controlled field.
//!
//! [`StickyAnswerGuard`] therefore does the smallest thing that is total: on **every**
//! accepted (`NV_OK`) fn-76 reply, whatever policy produced it, it writes `rmctrlFlags`
//! and `rmctrlAccessRight` back to **zero**.
//!
//! ★ Zero is not a neutralising trick, it is the **honest** value. Those two fields are a
//! real GSP's statement *"you may remember this answer"*. This GSP runs no firmware, its
//! rows are a build-time table, and it makes no promise that the answer it gave is the
//! answer it would give next time. `0` says exactly that, and it is byte-for-byte what a
//! stock guest already put there — so the guard is invisible to a stock boot and load
//! bearing against a crafted one.
//!
//! ## ★★★ 3. Branch (a) is NOT ours, it is LIVE TODAY, and it is written down here
//!
//! `bCacheable` is decided entirely inside the guest, before we see the command, from
//! flags NVOC generated into the guest's own module. No reply can influence it; the only
//! levers left are a non-`NV_OK` **envelope** (which skips the whole post-RPC block,
//! `ogkm-580: rpc.c:1994`) or a non-`NV_OK` control **`status`** (the `if
//! (rpc_params->status != NV_OK)` above). Both are refusals. ⇒ **any control we ANSWER
//! that the guest's table marks cacheable is cached, and there is no third option.**
//!
//! `[measured]` by reading `ogkm-580`'s generated export table on 2026-08-01 — the
//! `/*flags=*/` and `/*accessRight=*/` words beside each `methodId` in
//! `src/nvidia/generated/g_subdevice_nvoc.c` — **five of the twenty-five controls this port
//! already serves are in branch (a)**:
//!
//! | control | `flags` | site | `bCacheable` |
//! |---|---|---|---|
//! | `0x2080_1803` `BUS_GET_PCI_BAR_INFO` | `0x10518` | `g_subdevice_nvoc.c:6498` | **yes** |
//! | `0x2080_0a36` `INTERNAL_GPU_GET_CHIP_INFO` | `0x404c0` | `:2253` | **yes** |
//! | `0x2080_0a41` `INTERNAL_GPU_GET_USER_REGISTER_ACCESS_MAP` | `0x4c0` | `:2403` | **yes** |
//! | `0x2080_0a40` `INTERNAL_GET_DEVICE_INFO_TABLE` | `0x1c4c0` | `:2388` | **yes** |
//! | ★ `0x2080_0102` `GPU_GET_INFO_V2` | `0x30118` | `:151` | **yes**, and by a DIFFERENT bit |
//! | the rest | — | — | no (`CACHEABLE_ANY` clear) |
//!
//! ⚠ The fifth row, added at §14.28, is cacheable through `RMCTRL_FLAGS_CACHEABLE_BY_INPUT`
//! (`0x20000`) with plain `RMCTRL_FLAGS_CACHEABLE` (`0x400`) **clear** — the first row here
//! that reaches branch (a) by the input-keyed half of `CACHEABLE_ANY` rather than the
//! blanket one. ★ That is the *right* half for it, and it is the reason the exposure stays
//! tolerable even though this control's reply is not a constant: the guest's cache is keyed
//! on the request params, and this port's answer is a **pure function** of exactly those
//! params and a build-time [`crate::ChipProfile`] row. Two different index lists get two
//! different cache entries, which is what `BY_INPUT` means.
//!
//! ★★ **Two consequences worth more than the guard.**
//!
//! 1. **It is tolerable, and the reason is a property of those four and not of luck.**
//!    Each is a static per-GPU fact — a bar table, a chip id, a register-access bitmap, an
//!    engine table — and this port answers each from a build-time [`crate::ChipProfile`]
//!    row that cannot change while the device lives. Caching a constant is correct. ⊘ The
//!    day a served control's answer depends on device *state*, branch (a) makes it
//!    unserveable-as-answered and the choice becomes refuse-or-be-wrong-forever. That is
//!    the decision [`BRANCH_A_CACHEABLE`] exists to force someone to make.
//! 2. ⚠ **It blinds every recorder we own.** `rmapiControlCacheGet` runs *before* the send
//!    (`ogkm-580: rpc.c:10952-10961`), so the second and later asks for those four
//!    commands are answered inside the guest and **never appear on the wire**. A capture
//!    showing a control once is not evidence the guest asked once, and a differential that
//!    sees no repeat has observed nothing about repeats.
//!
//! ## ★ 4. What the `cap1b` capture says about bit 15 — asked, and answered: nothing
//!
//! `[measured]` run of `cargo run -p kayfabe-crec --example cap1b_report` at `rev 049ecae`
//! on 2026-08-01: `cap1b_coldboot_hermetic_d6` decodes **54** commands in reach, of which
//! 38 are fn 76, and **not one** control word has bit 15 set — they are `0x2080_0xxx`,
//! `0x2080_1xxx`, `0x2080_2xxx` and `0x2080_01b0`/`0x2080_017e`/`0x2080_0301`. So branch
//! (b) is **unexercised by the whole cold-boot prefix**, and no capture this repository
//! holds can bite a guard on it.
//!
//! ⊘ That is a reason to state the guard's reach in a test rather than to skip the guard.
//! The GSS-legacy traffic the C measured is the **cudart initialisation gate**
//! (`0x2080_9009`, `0x2080_9001`, `0x2080_9064` — `C: src/qemu/nvkvm_gpu_emul.c:3328-3395`,
//! pinned in `crates/kayfabe-rmrpc/tests/gss_legacy_answer.rs`), which arrives at CUDA
//! init, long past where `cap1b` ends. The exposure is real and simply later than every
//! trace we own.
//!
//! ## 5. The disposition ledger
//!
//! [`POLICY_DISPOSITIONS`] is one row per [`kayfabe_gsp::CommandPolicy`] implementation in
//! the workspace's library sources. It is not documentation: `tests/tests/sticky_answer.rs`
//! **derives** the same universe from `git ls-files` + the source text and fails if the two
//! disagree, then **exercises** each row's disposition against the policy it names. A row
//! that is wrong is red, and an implementation added without a row is red.

use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

/// `RMCTRL_FLAGS_CACHEABLE` (`ogkm-580: src/nvidia/inc/kernel/rmapi/control.h:260`,
/// `ogkm-610: :257`).
pub const RMCTRL_FLAGS_CACHEABLE: u32 = 0x0000_0400;

/// `RMCTRL_FLAGS_CACHEABLE_BY_INPUT` (`ogkm-580: control.h:298`, `ogkm-610: :295`).
pub const RMCTRL_FLAGS_CACHEABLE_BY_INPUT: u32 = 0x0002_0000;

/// `RMCTRL_FLAGS_CACHEABLE_ANY` (`ogkm-580: control.h:311`, `ogkm-610: :308`).
pub const RMCTRL_FLAGS_CACHEABLE_ANY: u32 =
    RMCTRL_FLAGS_CACHEABLE | RMCTRL_FLAGS_CACHEABLE_BY_INPUT;

/// `RMCTRL_FLAGS_INTERNAL` (`ogkm-580: control.h:239`, `ogkm-610: :236`).
pub const RMCTRL_FLAGS_INTERNAL: u32 = 0x0000_0080;

/// Byte offset of `rmctrlFlags` in `rpc_gsp_rm_control_v03_00`.
///
/// ★ Not `RpcControlReq`'s business: that view decodes what a guest **sent** and
/// deliberately omits both fields because a stock sender writes zero into them
/// (`ogkm-580: rpc.c:10994-10995`). Here they are read and written on the **reply**, which
/// is the only place they carry information.
pub const CONTROL_RMCTRL_FLAGS_OFF: usize = 24;

/// Byte offset of `rmctrlAccessRight` in the same header.
pub const CONTROL_RMCTRL_ACCESS_RIGHT_OFF: usize = 28;

/// The header this port must be able to see before it can neutralise a reply.
const CONTROL_HEADER: usize = 40;

/// `NV_OK`.
const NV_OK: u32 = 0;

/// The five served controls the **guest's own** export table marks cacheable — branch (a),
/// which no reply of ours can influence. Module docs §3 carries the flag words and the
/// generated-source line for each.
///
/// ★ Stated as data so a test can quantify over it, and kept **here** rather than beside
/// [`crate::inittables::WantedTable::ALL`] on purpose: this is a fact about the *guest's*
/// table, not about what we serve. The two universes are related only by intersection, and
/// `tests/tests/sticky_answer.rs` asserts that relationship rather than assuming it.
pub const BRANCH_A_CACHEABLE: [u32; 6] = [
    0x2080_1803,
    0x2080_0a36,
    0x2080_0a41,
    0x2080_0a40,
    // ★ §14.28. Cacheable by INPUT (`flags = 0x30118`), so the guest keys its cache on the
    // request — and this port's reply is a pure function of that same request plus a
    // constant chip row. See the module docs' §3 table.
    0x2080_0102,
    // ★★★ §14.35 — `NV2080_CTRL_CMD_GSP_GET_FEATURES`, `flags = 0x40549`
    // (`ogkm-580: g_subdevice_nvoc.c:9466`), carrying plain `RMCTRL_FLAGS_CACHEABLE`
    // (`0x400`). It is the FIRST row here whose reply is not a projection of a build-time
    // `ChipProfile`: three fields are header constants, but `firmwareVersion` is latched
    // from the guest's own fn 1 (`crate::inittables::InitTablePolicy::guest_firmware`).
    //
    // ⇒ So the question this array exists to force gets its first real answer instead of
    // "it is a constant, therefore fine". It is still **correct to cache**, and the reason
    // is a lifetime argument rather than a constancy one: the latch is written once per
    // driver load, during the version handshake, and every `0x20803601` a guest can issue
    // arrives after it — the guest's cache and our latch are populated in that order and
    // neither outlives the driver load. ⊘ A guest that reloads its driver re-sends fn 1
    // and rebuilds both.
    0x2080_3601,
];

/// `rmapiControlIsCacheable(flags, accessRight, NV_TRUE)` — a transcription, not a
/// paraphrase (`ogkm-580: src/nvidia/src/kernel/rmapi/rmapi_cache.c:152-172`,
/// `ogkm-610: :152-172`).
///
/// ⊘ `_cacheIsDisabled()` is deliberately **not** modelled. It reads the guest's own
/// runtime cache mode, which we cannot observe and must not assume is off — assuming it
/// off is the safe direction (it makes this return `true` more often, i.e. it makes the
/// guard fire more often), and assuming it on would be an unfalsifiable excuse to skip the
/// guard.
#[must_use]
pub fn rmapi_control_is_cacheable(flags: u32, access_right: u32, allow_internal: bool) -> bool {
    if flags & RMCTRL_FLAGS_CACHEABLE_ANY == 0 {
        return false;
    }
    if access_right != 0 {
        return false;
    }
    if flags & RMCTRL_FLAGS_INTERNAL != 0 {
        return allow_internal;
    }
    true
}

/// Whether a reply carrying `flags`/`access_right` for `cmd` would reach
/// `rmapiControlCacheSetUnchecked` — the **whole** branch-(b) conjunction of
/// `ogkm-580: rpc.c:11098-11103` / `ogkm-610: :10903-10908`, `IsGssLegacyCall` included.
///
/// ⊘ `!(resCtrlFlags & NVOS54_FLAGS_FINN_SERIALIZED)` is the one conjunct not modelled
/// here: `resCtrlFlags` is the guest's *local* call-context flags word and never crosses
/// the wire. Omitting it is again the safe direction — it can only make this answer `true`
/// where the guest would have answered `false`.
#[must_use]
pub fn reply_would_be_cached(cmd: u32, flags: u32, access_right: u32) -> bool {
    kayfabe_abi::capability::RM_GSS_LEGACY_MASK & cmd != 0
        && rmapi_control_is_cacheable(flags, access_right, true)
        && flags & RMCTRL_FLAGS_CACHEABLE_BY_INPUT == 0
}

/// ★★★ **The one seam between every command policy and the guest's control cache.**
///
/// Wraps a [`CommandPolicy`] — in production, the whole of [`crate::served_policy`]'s
/// chain — and makes one statement true of *everything* behind it: **no answer of ours can
/// be remembered by the guest through branch (b)**. Module docs carry the derivation of why
/// that is the only branch a reply can reach, and why fn 76 is the only function.
///
/// ## Why a decorator and not a check in each policy
///
/// A per-policy check is a list, and this repository's most-repeated defect is a list that
/// silently shrinks (memory: `gates_quantified_over_a_list`). A decorator has no list: a
/// policy added to the chain tomorrow is behind it tomorrow, and a policy that *forgets*
/// the rule is corrected rather than trusted. The universe and the guarded set are the same
/// object rather than two that agree today — the shape that fixed
/// [`crate::inittables::WantedTable::from_cmd`].
///
/// ⊘ It does not replace [`crate::inittables::InitTablePolicy`]'s own refusal, which is a
/// **stronger and different** statement: *that* policy declines to serve a GSS-legacy
/// control at all. This guard says only that whatever anyone does serve cannot stick.
pub struct StickyAnswerGuard {
    driver: DriverAbiTable,
    inner: Box<dyn CommandPolicy>,
    inspected: u64,
    rewritten: u64,
    neutralised: u64,
    malformed: u64,
}

impl StickyAnswerGuard {
    /// Wrap `inner`, decoding control headers with `driver`.
    #[must_use]
    pub fn new(driver: DriverAbiTable, inner: Box<dyn CommandPolicy>) -> StickyAnswerGuard {
        StickyAnswerGuard {
            driver,
            inner,
            inspected: 0,
            rewritten: 0,
            neutralised: 0,
            malformed: 0,
        }
    }

    /// How many accepted fn-76 replies this guard looked at.
    ///
    /// ★ The non-vacuity instrument for every "nothing was neutralised" assertion: zero
    /// neutralisations over a run that inspected nothing is a guard that was never on the
    /// path, which is the green-instrument-on-an-unexercised-path failure the doctrine is
    /// about.
    #[must_use]
    pub fn inspected(&self) -> u64 {
        self.inspected
    }

    /// How many accepted fn-76 replies carried a **non-zero** `rmctrlFlags` or
    /// `rmctrlAccessRight` that this guard had to zero.
    ///
    /// ⊘ Expected to be **zero against a stock guest** — RM zeroes both fields before every
    /// send (`ogkm-580: rpc.c:10994-10995`) — so a non-zero count means either a policy
    /// authored them or the guest asked for its own answer to be persisted. Both are worth
    /// a number, and neither is worth a refusal.
    #[must_use]
    pub fn rewritten(&self) -> u64 {
        self.rewritten
    }

    /// How many of [`Self::rewritten`] would have reached
    /// `rmapiControlCacheSetUnchecked` — i.e. satisfied the **whole** branch-(b)
    /// conjunction, GSS-legacy bit included ([`reply_would_be_cached`]).
    ///
    /// ★ Strictly nested inside [`Self::rewritten`], and separate on purpose: *"the guest
    /// set a flag we do not honour"* and *"the guest set a flag that would have made this
    /// answer permanent"* are different observations, and a single total would let the
    /// second hide inside the first. This is the number that says branch (b) was actually
    /// live.
    #[must_use]
    pub fn neutralised(&self) -> u64 {
        self.neutralised
    }

    /// How many accepted fn-76 replies were **refused** because their body could not hold
    /// the control header the guest will read out of it.
    #[must_use]
    pub fn malformed(&self) -> u64 {
        self.malformed
    }
}

impl core::fmt::Debug for StickyAnswerGuard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StickyAnswerGuard")
            .field("inspected", &self.inspected)
            .field("rewritten", &self.rewritten)
            .field("neutralised", &self.neutralised)
            .field("malformed", &self.malformed)
            .finish_non_exhaustive()
    }
}

impl CommandPolicy for StickyAnswerGuard {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        let mut reply = self.inner.respond(cmd)?;
        // ⊘ A declined command is still declined: the FSM's named refusal is what it gets,
        // and a refusal cannot be cached (`ogkm-580: rpc.c:1994` short-circuits ahead of
        // the whole post-RPC block).
        if cmd.function != RpcFunction::RmControl || reply.rpc_result != NV_OK {
            return Some(reply);
        }
        self.inspected = self.inspected.saturating_add(1);

        // An empty body is `RpcCommand::reply`'s zero-fill, which writes zeros into both
        // fields — already the answer this guard wants, and nothing to rewrite.
        if reply.body.is_empty() {
            return Some(reply);
        }
        // ★ An accepted control reply whose body cannot hold the header is refused rather
        // than passed: the guest copies `paramsSize` bytes out of `rpc_params->params` and
        // reads `rpc_params->status` from inside that header, so a short body under `NV_OK`
        // is a body the guest will read past. A refusal is this port's house answer to
        // "we cannot state this correctly".
        if reply.body.len() < CONTROL_HEADER {
            self.malformed = self.malformed.saturating_add(1);
            return Some(Reply {
                rpc_result: kayfabe_abi::NV_ERR_NOT_SUPPORTED,
                body: Vec::new(),
            });
        }

        let control = self.driver.decode_rpc_control(&cmd.payload).ok();
        let flags = u32::from_le_bytes([
            reply.body[CONTROL_RMCTRL_FLAGS_OFF],
            reply.body[CONTROL_RMCTRL_FLAGS_OFF + 1],
            reply.body[CONTROL_RMCTRL_FLAGS_OFF + 2],
            reply.body[CONTROL_RMCTRL_FLAGS_OFF + 3],
        ]);
        let access_right = u32::from_le_bytes([
            reply.body[CONTROL_RMCTRL_ACCESS_RIGHT_OFF],
            reply.body[CONTROL_RMCTRL_ACCESS_RIGHT_OFF + 1],
            reply.body[CONTROL_RMCTRL_ACCESS_RIGHT_OFF + 2],
            reply.body[CONTROL_RMCTRL_ACCESS_RIGHT_OFF + 3],
        ]);
        // ★★ The rewrite is UNCONDITIONAL on the flag words and does not consult
        // `is_gss_legacy` to decide whether to act. Deliberate: `0` is the value a real
        // sender puts there and the value this GSP can honestly stand behind, so writing it
        // for every accepted control needs no classification to be correct — and a guard
        // that classified first would be one `cmd`-decode away from being wrong. The
        // classification is used only for the COUNTER, which is a question about what the
        // guest tried, not about what we owe it.
        reply.body[CONTROL_RMCTRL_FLAGS_OFF..CONTROL_RMCTRL_FLAGS_OFF + 4]
            .copy_from_slice(&0u32.to_le_bytes());
        reply.body[CONTROL_RMCTRL_ACCESS_RIGHT_OFF..CONTROL_RMCTRL_ACCESS_RIGHT_OFF + 4]
            .copy_from_slice(&0u32.to_le_bytes());
        if flags != 0 || access_right != 0 {
            self.rewritten = self.rewritten.saturating_add(1);
            if let Some(c) = control
                && reply_would_be_cached(c.cmd, flags, access_right)
            {
                self.neutralised = self.neutralised.saturating_add(1);
            }
        }
        Some(reply)
    }
}

// =====================================================================================
// The disposition ledger — one row per `CommandPolicy` implementation
// =====================================================================================

/// Why a given [`CommandPolicy`] implementation cannot hand the guest a sticky answer.
///
/// ★ Each variant is a **claim a test can execute**, not a label.
/// `tests/tests/sticky_answer.rs` runs the matching check for every row of
/// [`POLICY_DISPOSITIONS`], so a row carrying the wrong variant is red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StickyDisposition {
    /// Behind [`StickyAnswerGuard`] in [`crate::served_policy`], so branch (b) is closed
    /// for it by construction whatever it answers.
    Guarded,
    /// `respond` **never** returns `Some` — a recorder. Recording is not answering, so
    /// there is no reply to cache.
    NeverAnswers,
    /// Answers only functions that are not `GSP_RM_CONTROL`. Both cache-populating call
    /// sites live inside `rpcRmApiControl_GSP` (§1a), so no reply of this policy's can
    /// reach either.
    NotAControl,
    /// Pure delegation — it returns some inner policy's reply unchanged and authors none.
    Delegates,
    /// Not installed in the port. Reachable only from tests and the `kayfabe-crec` replay,
    /// and it carries its argument about the **accepted** path in its own rustdoc.
    FixtureOnly,
}

/// One implementation, and how it is discharged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyDisposition {
    /// The implementing type, spelled exactly as `impl CommandPolicy for <name>` spells it.
    pub name: &'static str,
    /// Where it lives, relative to the workspace root.
    pub path: &'static str,
    /// The discharge.
    pub disposition: StickyDisposition,
}

/// ★★★ **Every [`CommandPolicy`] implementation in the workspace's library sources, and
/// what stops each from handing the guest a permanent answer.**
///
/// ⊘ This array is **not** the universe — it is a claim *about* the universe.
/// `tests/tests/sticky_answer.rs::the_universe_of_answering_policies_is_derived_from_the_source`
/// derives the real one from `git ls-files` and the `impl CommandPolicy for` occurrences in
/// it, and fails on any disagreement in either direction. An implementation added without a
/// row here is red; a row here naming a type that no longer implements the trait is red.
/// That is the only shape that survives this repository's most-repeated defect — a
/// hand-written list quietly shrinking with zero red tests.
///
/// ⊘ Test-only implementations are out of scope by the same derivation: the test filters
/// `git ls-files` to `crates/*/src/**`, so a `CommandPolicy` written inside a `#[test]`
/// module or a `tests/` target is neither required here nor forbidden there.
pub const POLICY_DISPOSITIONS: [PolicyDisposition; 15] = [
    PolicyDisposition {
        name: "InitTablePolicy",
        path: "crates/kayfabe-device/src/inittables.rs",
        disposition: StickyDisposition::Guarded,
    },
    // ★ The control census (`#`03a5c31). `Delegates` is the unconditional statement: it
    // forwards to the wrapped policy and returns that reply UNCHANGED — byte-for-byte,
    // pinned by `tests/control_census.rs::the_census_changes_no_byte_of_any_reply` — so
    // it authors nothing a cache could keep. ⊘ This row was missing for one commit: the
    // impl landed in 03a5c31 without it, and the derive-from-source test was red on
    // master while that commit's message claimed a green suite. The ledger row is the
    // fix; the incident is the reason the test derives the universe instead of trusting
    // this array.
    PolicyDisposition {
        name: "ControlCensus<P>",
        path: "crates/kayfabe-device/src/census.rs",
        disposition: StickyDisposition::Delegates,
    },
    PolicyDisposition {
        name: "StaticInfoPolicy",
        path: "crates/kayfabe-device/src/staticinfo.rs",
        disposition: StickyDisposition::NotAControl,
    },
    PolicyDisposition {
        name: "GuestSystemInfoPolicy",
        path: "crates/kayfabe-device/src/guestsysinfo.rs",
        disposition: StickyDisposition::NotAControl,
    },
    PolicyDisposition {
        name: "InertPolicy",
        path: "crates/kayfabe-device/src/inert.rs",
        disposition: StickyDisposition::NotAControl,
    },
    PolicyDisposition {
        name: "FaultBufferRecorder",
        path: "crates/kayfabe-device/src/faultbuffer.rs",
        disposition: StickyDisposition::NeverAnswers,
    },
    // ★★★ The VA-space page-directory recorder (`crate::gvaspub`), and its row is the
    // stronger claim of the two available. It sits **first** in `served_chain` — ahead of
    // every answering link, which is the only seat from which it can see a control
    // `InitTablePolicy` terminates the chain for — so `Guarded` would be true and would
    // say nothing about the seat. `NeverAnswers` is a claim about the TYPE: `respond`
    // returns `None` on every path without exception, so there is no reply of its own for
    // a cache to keep and none for `find_map` to short-circuit on, however it is composed.
    // ⊘ That is also the property that makes seating an observer ahead of the answerer
    // legal at all, and `the_never_answers_rows_answer_nothing` executes it here rather
    // than leaving it to `gvas_publication.rs` alone.
    PolicyDisposition {
        name: "GvasPubRecorder",
        path: "crates/kayfabe-device/src/gvaspub.rs",
        disposition: StickyDisposition::NeverAnswers,
    },
    PolicyDisposition {
        name: "UnservicedLedger",
        path: "crates/kayfabe-device/src/unserviced.rs",
        disposition: StickyDisposition::NeverAnswers,
    },
    PolicyDisposition {
        name: "StickyAnswerGuard",
        path: "crates/kayfabe-device/src/sticky.rs",
        disposition: StickyDisposition::Delegates,
    },
    PolicyDisposition {
        name: "PolicyChain",
        path: "crates/kayfabe-gsp/src/boot.rs",
        disposition: StickyDisposition::Delegates,
    },
    PolicyDisposition {
        name: "EchoOk",
        path: "crates/kayfabe-gsp/src/boot.rs",
        disposition: StickyDisposition::FixtureOnly,
    },
    PolicyDisposition {
        name: "GraphPolicy<'_>",
        path: "crates/kayfabe-rmrpc/src/policy.rs",
        disposition: StickyDisposition::FixtureOnly,
    },
    // ★★★ The first row added since a policy was actually INSTALLED in the port
    // (2026-08-01, the `GspRmAlloc` rung). `GraphPolicy`'s own rustdoc predicted this
    // moment — *"the day this policy is installed, it must go inside that wrapper"* — and
    // this is not that day: what got installed is `ObjectPolicy`, a narrower type, and it
    // IS inside the wrapper (`served_policy` wraps the whole chain including the object
    // link, which is why the link is a `served_chain` argument and not a second policy the
    // plane holds beside the guard).
    //
    // ⊘⊘ **CORRECTED 2026-08-08, and the correction is the interesting part.** This row
    // read `NotAControl`, argued as *"this policy claims exactly
    // `kayfabe_rmrpc::OBJECT_VERBS`, which contains no `GSP_RM_CONTROL`"* — and that
    // stopped being true when `#177` gave `ObjectPolicy` a second list,
    // `kayfabe_rmrpc::OBJECT_CONTROLS`, so it now answers `NVA06F_CTRL_CMD_GPFIFO_SCHEDULE`
    // and `NVA06F_CTRL_CMD_BIND` with `NV_OK` and a body.
    //
    // ★★★ The row's executor could not notice, and the reason is this repository's own
    // named trap: `the_not_a_control_rows_decline_every_control_command` sweeps
    // `WantedTable::ALL` — **`InitTablePolicy`'s** 25 ids — so it quantified over a
    // universe that contains neither control this policy answers, and passed for a year of
    // commits by asking about the wrong list (`gates_quantified_over_a_list`). A false
    // claim, an executing test, and a green suite.
    //
    // ⇒ `Guarded` is the true statement: `served_policy` wraps the WHOLE chain, this link
    // included, so both reply fields are zeroed on the way out. ⚠ It is weaker on purpose —
    // it is a claim about the installer, and `served_chain` exists precisely so a test can
    // build the chain without the guard. Anything installing this policy outside
    // `served_policy` owes the guard itself.
    PolicyDisposition {
        name: "ObjectPolicy",
        path: "crates/kayfabe-rmrpc/src/policy.rs",
        disposition: StickyDisposition::Guarded,
    },
    // ★★★ §14.23 — the observer ADAPTER (`kayfabe_gsp::Observing`), and its row is the
    // strongest of all of them because it is not a property of the code inside `respond`:
    // the type's whole body is `{ self.0.observe(cmd); None }`, and the wrapped
    // `CommandObserver` **has no return value to give it**. So `NeverAnswers` here is not
    // "this implementation happens to return `None` on every path" but "no implementation
    // of the wrapped trait could make it return anything else".
    //
    // ⊘ That is exactly what makes the chain's FRONT seat legal: a link seated ahead of the
    // answerer must not be able to answer, and `served_chain`'s front seat takes a
    // `CommandObserver` rather than a `CommandPolicy` for that reason.
    PolicyDisposition {
        name: "Observing",
        path: "crates/kayfabe-gsp/src/boot.rs",
        disposition: StickyDisposition::NeverAnswers,
    },
    // ★★ `#149`. `BarPdePolicy` answers exactly `UPDATE_BAR_PDE` (fn 70) and declines
    // everything else, so `NotAControl` is the unconditional statement — it is a claim
    // about the TYPE and not about who installs it, and both cache-populating call sites
    // live inside `rpcRmApiControl_GSP` (§1a), which fn 70 never reaches.
    //
    // ⊘ Worth saying why the question is not vacuous even so: this policy answers `NV_OK`
    // to a command whose sender **discards the status**, which is the shape that most
    // invites "then it does not matter what we reply". It matters for a different reason —
    // the reply is what makes the FSM stop refusing — and the sticky question is separate
    // and is answered here.
    PolicyDisposition {
        name: "BarPdePolicy",
        path: "crates/kayfabe-device/src/bar2.rs",
        disposition: StickyDisposition::NotAControl,
    },
];

kayfabe_util::assert_send!(StickyAnswerGuard);
kayfabe_util::assert_send_sync!(PolicyDisposition, StickyDisposition);
