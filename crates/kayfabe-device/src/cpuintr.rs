//! ★★★ The **CPU interrupt tree** — `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_*`, the seven
//! register families a Turing-or-later driver drives to raise, find and clear an interrupt.
//!
//! # ★★★ Why this exists: `RmInitAdapter failed! (0x11:0x45:2134)`
//!
//! `[measured]` run `stateload2` at `7819839`
//! (`/workspace/bench/run_stateload2_dmesg.log:31,35`):
//!
//! ```text
//! NVRM: RmInitAdapter: osVerifySystemEnvironment failed, bailing!
//! NVRM: GPU 0000:00:03.0: RmInitAdapter failed! (0x11:0x45:2134)
//! ```
//!
//! `0x11` is `RM_INIT_SYS_ENVIRONMENT_FAILED` and `0x45` is `NV_ERR_IRQ_NOT_FIRING`
//! (`ogkm-580: src/common/sdk/nvidia/inc/nvstatuscodes.h:98`). ⚠ Note what that is **not**:
//! it is not in the `0x20` GPU block, so it is not a GPU-init failure at all. It is
//! `osVerifySystemEnvironment` → `_osVerifyInterrupts`
//! (`ogkm-580: src/nvidia/src/kernel/os/os_sanity.c:299-311`, called at
//! `arch/nvalloc/unix/src/osinit.c:2127`), which is a **loopback self-test**: the driver
//! asks the device to interrupt it, and fails the whole adapter if nothing arrives.
//!
//! ## The exact sequence, and every register it touches
//!
//! `_osVerifyInterrupts` (`os_sanity.c:117-291`) does this, on a GA10x
//! (`intr_swintr_tu102.c`, which is the Ampere HAL too):
//!
//! 1. disable every other tree, then `intrClearStallSWIntr_TU102` → `intrClearLeafVector`
//!    → write `NVBIT(bit)` to `CPU_INTR_LEAF(reg)` (`intr_tu102.c:648-663`);
//! 2. `intrEnableStallSWIntr_TU102` (`intr_swintr_tu102.c:72-90`) → `intrEnableLeaf` writes
//!    `CPU_INTR_LEAF_EN_SET(reg)`, then `CPU_INTR_TOP_EN_SET(topIdx) = NVBIT(topBit)`;
//! 3. `pGpu->testIntr = NV_TRUE`, so the ISR is diverted to `osSanityTestIsr`
//!    (`g_os_nvoc.h:1181-1200` → `unix_intr.c:850`);
//! 4. **`intrSetStallSWIntr_TU102`** (`intr_swintr_tu102.c:40-51`) —
//!    `GPU_VREG_WR32(NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF_TRIGGER, 129)`. ★ **This one
//!    write is the whole request.** Everything before it is set-up.
//! 5. spin on `interrupt_triggered` for `pGpu->timeoutData.defaultus`, ~4.3 s
//!    `[measured]`: `stateload2`'s log timestamps are `21.704` at the last state-load line
//!    and `26.016` at `osVerifySystemEnvironment failed`;
//! 6. the ISR — `osWaitForInterrupt` (`os_sanity.c:49-101`) — calls
//!    `intrGetStallInterruptMode_TU102` (`intr_swintr_tu102.c:118-135`), which reports
//!    `INTERRUPT_TYPE_SOFTWARE` unconditionally and `pending =
//!    intrIsVectorPending_TU102` → **reads `CPU_INTR_LEAF(reg)` and tests `NVBIT(bit)`**
//!    (`intr_tu102.c:729-744`). If clear it returns without setting the flag, and the test
//!    still fails even though a vector was delivered;
//! 7. on a hit it clears the leaf bit and sets `interrupt_triggered = 1`.
//!
//! ⇒ Three things must all be true, and building any two of them is worth nothing:
//! the **trigger write** must be seen, the **vector** must actually reach the guest, and
//! the **leaf register must read back pending** when the ISR looks.
//!
//! # The address arithmetic, and the one constant everything hangs off
//!
//! `GPU_VREG_WR32(pGpu, a)` is `pGpu->sriovState.virtualRegPhysOffset + a`, and on a
//! **physical** function that offset is `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` =
//! `0x00B8_0000` (`ogkm-580: kern_gpu_tu102.c:92-101`, base at
//! `src/common/inc/swref/published/turing/tu102/dev_vm.h:28`). ⚠ It is zero **only** for an
//! SR-IOV virtual function, and this device does not claim to be one — see
//! `kayfabe_device::ga10x`'s `VIRTUAL_FUNCTION_BASE`, which resolves the same choice the
//! same way.
//!
//! The vector is `NV_CTRL_CPU_DOORBELL_VECTORID_VALUE_CONSTANT` = **129**
//! (`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_ctrl.h:35`), and the three
//! index macros are `dev_ctrl_defines.h:70,71,77-78,46,47`:
//!
//! ```text
//! leaf reg = v / 32          = 129 / 32 = 4
//! leaf bit = v % 32          = 129 % 32 = 1
//! subtree  = leafReg / 2     =   4 /  2 = 2
//! top idx  = subtree / 32    =   2 / 32 = 0
//! top bit  = subtree % 32    =   2 % 32 = 2
//! ```
//!
//! # ★★★ The one place this deliberately differs from silicon, and why
//!
//! A real GIN gates delivery on the enable bits: a leaf whose `LEAF_EN` bit is clear, or
//! whose subtree is not in `TOP_EN`, sets the pending bit and sends nothing. **This model
//! raises unconditionally**, and records the disagreement instead of acting on it — see
//! [`TriggerOutcome::would_be_masked`].
//!
//! ⊘ That is a decision, not an oversight, and the reasoning is about which failure is
//! recoverable. If the enable bookkeeping here is subtly wrong and this gated on it, the
//! guest would see *exactly* `NV_ERR_IRQ_NOT_FIRING` — byte-identical to not having built
//! this at all — and the boot that told us so would not distinguish "we did not build it"
//! from "we built it and masked ourselves". Ungated, a bookkeeping error costs at worst a
//! spurious vector, which the guest's own ISR already handles: `osWaitForInterrupt` reads
//! the leaf, finds nothing pending, and returns without servicing
//! (`os_sanity.c:65-71` → `rm_isr` `NV_ERR_NO_INTR_PENDING` → `unix_intr.c:872`).
//!
//! ★ The C artifact makes the same choice at `C: src/qemu/nvkvm_gpu_emul.c:4376-4394` —
//! it raises on `LEAF_TRIGGER` with no reference to `intr_leaf_en` at all — and it is the
//! only implementation a real NVIDIA driver has ever accepted end to end. `[measured]` from
//! the oracle's own capture: `cap1` contains **exactly one** `IrqRaise` in 359 062 records
//! and it is a write of `0x81` (= 129) to `0xb81640`, i.e. this very self-test
//! (`crates/kayfabe-crec/tests/cap1_differential.rs:31`).
//!
//! ⊘ What that does NOT establish: the oracle's capture says nothing about whether gating
//! would have worked, because the oracle never gated. [`TriggerOutcome::would_be_masked`]
//! is what makes the question answerable in one boot rather than by argument.

/// `DRF_BASE(NV_VIRTUAL_FUNCTION_FULL_PHYS_OFFSET)` — where the `dev_vm` register block
/// lands in BAR0 on a physical function (`ogkm-580: turing/tu102/dev_vm.h:28`, selected by
/// `kern_gpu_tu102.c:92-101`).
pub const VF_PRIV_BASE: u64 = 0x00B8_0000;

/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF(i)` = `0x1000 + i*4`, `__SIZE_1` = 8
/// (`ogkm-580: ampere/ga102/dev_vm.h:49-50`). Read: pending. Write: **write-1-to-clear**,
/// which is what `intrClearLeafVector_TU102` relies on (`intr_tu102.c:648-663`).
pub const LEAF0: u64 = VF_PRIV_BASE + 0x1000;
/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF_EN_SET(i)` = `0x1200 + i*4` (`ga102/dev_vm.h:53`).
pub const LEAF_EN_SET0: u64 = VF_PRIV_BASE + 0x1200;
/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF_EN_CLEAR(i)` = `0x1400 + i*4`
/// (`ga102/dev_vm.h:57`).
pub const LEAF_EN_CLEAR0: u64 = VF_PRIV_BASE + 0x1400;
/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_TOP(i)` = `0x1600 + i*4`, `__SIZE_1` = 1, and marked
/// `R--4A` — read-only (`ga102/dev_vm.h:26-27`).
pub const TOP0: u64 = VF_PRIV_BASE + 0x1600;
/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_TOP_EN_SET(i)` = `0x1608 + i*4` (`ga102/dev_vm.h:33`).
pub const TOP_EN_SET0: u64 = VF_PRIV_BASE + 0x1608;
/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_TOP_EN_CLEAR(i)` = `0x1610 + i*4`
/// (`ga102/dev_vm.h:41`).
pub const TOP_EN_CLEAR0: u64 = VF_PRIV_BASE + 0x1610;
/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF_TRIGGER` = `0x1640`, marked `-W-4R` — **write
/// only**, `VECTOR` in bits `11:0` (`ga102/dev_vm.h:61-62`).
pub const LEAF_TRIGGER: u64 = VF_PRIV_BASE + 0x1640;

/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF__SIZE_1` (`ga102/dev_vm.h:50`) — and therefore
/// the bound on every `LEAF*` family, since `LEAF_EN_SET`/`LEAF_EN_CLEAR` carry the same
/// `__SIZE_1` (`:54`, `:58`).
pub const N_LEAF: usize = 8;

/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_TOP__SIZE_1` (`ga102/dev_vm.h:27`).
///
/// ⚠ One, on this chip. `intrDisableStallSWIntr_TU102` indexes `TOP_EN_CLEAR(topIdx)` with
/// `subtree / 32`, so a subtree above 31 would need a second row — and there are 64 subtree
/// bits defined (`:30`). This port answers the one row the register block declares and
/// nothing beyond it, which is why [`decode`] returns `None` rather than clamping.
pub const N_TOP: usize = 1;

/// `NV_CTRL_CPU_DOORBELL_VECTORID_VALUE_CONSTANT`
/// (`ogkm-580: src/common/inc/swref/published/turing/tu102/dev_ctrl.h:35`) — the vector
/// `_osVerifyInterrupts` triggers, and the only one this port has ever seen written.
pub const DOORBELL_VECTOR: u32 = 129;

/// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF_TRIGGER_VECTOR` is `11:0` (`ga102/dev_vm.h:62`).
const TRIGGER_VECTOR_MASK: u64 = 0xFFF;

/// `NV_CTRL_INTR_GPU_VECTOR_TO_LEAF_REG(i)` (`ogkm-580: dev_ctrl_defines.h:70`).
#[must_use]
pub const fn vector_to_leaf_reg(vector: u32) -> usize {
    (vector / 32) as usize
}

/// `NV_CTRL_INTR_GPU_VECTOR_TO_LEAF_BIT(i)` (`ogkm-580: dev_ctrl_defines.h:71`).
#[must_use]
pub const fn vector_to_leaf_bit(vector: u32) -> u32 {
    vector % 32
}

/// `NV_CTRL_INTR_GPU_VECTOR_TO_SUBTREE(i)` — the *leaf register* index halved, not the
/// vector (`ogkm-580: dev_ctrl_defines.h:77-78`). ⚠ Two leaves per subtree; reading this as
/// `vector / 64` gives the same answer and is the wrong reason.
#[must_use]
pub const fn vector_to_subtree(vector: u32) -> u32 {
    (vector_to_leaf_reg(vector) as u32) / 2
}

/// `NV_CTRL_INTR_SUBTREE_TO_TOP_IDX(i)` (`ogkm-580: dev_ctrl_defines.h:46`).
#[must_use]
pub const fn subtree_to_top_idx(subtree: u32) -> usize {
    (subtree / 32) as usize
}

/// `NV_CTRL_INTR_SUBTREE_TO_TOP_BIT(i)` (`ogkm-580: dev_ctrl_defines.h:47`).
#[must_use]
pub const fn subtree_to_top_bit(subtree: u32) -> u32 {
    subtree % 32
}

/// One decoded register in the tree.
///
/// ★ A decoded *place*, not an offset: the index is range-checked here, once, so no
/// consumer indexes an array with guest-supplied arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuIntrReg {
    /// `CPU_INTR_LEAF(i)` — pending, write-1-to-clear.
    Leaf(usize),
    /// `CPU_INTR_LEAF_EN_SET(i)`.
    LeafEnSet(usize),
    /// `CPU_INTR_LEAF_EN_CLEAR(i)`.
    LeafEnClear(usize),
    /// `CPU_INTR_TOP(i)` — read-only.
    Top(usize),
    /// `CPU_INTR_TOP_EN_SET(i)`.
    TopEnSet(usize),
    /// `CPU_INTR_TOP_EN_CLEAR(i)`.
    TopEnClear(usize),
    /// `CPU_INTR_LEAF_TRIGGER` — write-only.
    LeafTrigger,
}

/// Which BAR0 offset, if any, names a register in this tree.
///
/// ⊘ `None` for anything outside the declared `__SIZE_1` of each family, deliberately: an
/// out-of-range index is the guest addressing a row this chip does not have, and answering
/// it would be inventing a register block. It falls through to the plane's ordinary
/// unclaimed arm, which counts it and names it.
#[must_use]
pub fn decode(off: u64) -> Option<CpuIntrReg> {
    // ⚠ `LEAF_TRIGGER` is tested FIRST. It is `0xB81640`, and `TOP_EN_CLEAR0` is
    // `0xB81610`; a range test written as `off >= TOP_EN_CLEAR0 && off < TOP_EN_CLEAR0 +
    // N*4` with the wrong `N` would swallow it. Testing the singleton first makes that
    // class of arithmetic error impossible rather than merely absent today.
    if off == LEAF_TRIGGER {
        return Some(CpuIntrReg::LeafTrigger);
    }
    let row = |base: u64, n: usize| -> Option<usize> {
        if off >= base && off < base + (n as u64) * 4 && (off - base).is_multiple_of(4) {
            Some(((off - base) / 4) as usize)
        } else {
            None
        }
    };
    if let Some(i) = row(LEAF0, N_LEAF) {
        return Some(CpuIntrReg::Leaf(i));
    }
    if let Some(i) = row(LEAF_EN_SET0, N_LEAF) {
        return Some(CpuIntrReg::LeafEnSet(i));
    }
    if let Some(i) = row(LEAF_EN_CLEAR0, N_LEAF) {
        return Some(CpuIntrReg::LeafEnClear(i));
    }
    if let Some(i) = row(TOP0, N_TOP) {
        return Some(CpuIntrReg::Top(i));
    }
    if let Some(i) = row(TOP_EN_SET0, N_TOP) {
        return Some(CpuIntrReg::TopEnSet(i));
    }
    if let Some(i) = row(TOP_EN_CLEAR0, N_TOP) {
        return Some(CpuIntrReg::TopEnClear(i));
    }
    None
}

/// What a `LEAF_TRIGGER` write asked for, and what this model would have done with the
/// enables if it gated on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerOutcome {
    /// The vector, masked to `LEAF_TRIGGER_VECTOR` (`11:0`).
    pub vector: u32,
    /// The vector named a leaf row this chip does not have, so nothing was latched and
    /// nothing is raised. ⊘ Not an error to report upward: it is the guest writing a vector
    /// out of range, and the honest answer is to do nothing, loudly counted.
    pub out_of_range: bool,
    /// ★★★ **The disagreement, recorded rather than acted on.** True when the leaf-enable
    /// bit or the subtree's top-enable bit was clear at the instant of the trigger — i.e.
    /// when real silicon would have latched the pending bit and sent no message.
    ///
    /// A boot in which this is ever `true` for the doorbell vector is a boot in which
    /// gating would have cost us the adapter, and it is the only evidence that settles
    /// whether the enable bookkeeping here is right. See this module's header.
    pub would_be_masked: bool,
}

/// The `CPU_INTR` tree's state — pending and enables, nothing else.
///
/// ★ No interrupt *line* state. Whether a vector is in flight is the shell's fact and the
/// hypervisor's; this owns only what the guest can read back out of these registers, which
/// is what makes it testable without a VMM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuIntrTree {
    leaf: [u32; N_LEAF],
    leaf_en: [u32; N_LEAF],
    top: [u32; N_TOP],
    top_en: [u32; N_TOP],
}

impl Default for CpuIntrTree {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuIntrTree {
    /// Reset state — every pending bit and every enable clear.
    ///
    /// `NV_VIRTUAL_FUNCTION_PRIV_CPU_INTR_LEAF_VALUE_INIT` and both `_EN_*_VALUE_INIT` are
    /// `0x00000000` (`ogkm-580: ampere/ga102/dev_vm.h:52,56,60`), so this is the register
    /// block's own stated reset rather than a convenience.
    #[must_use]
    pub const fn new() -> CpuIntrTree {
        CpuIntrTree {
            leaf: [0; N_LEAF],
            leaf_en: [0; N_LEAF],
            top: [0; N_TOP],
            top_en: [0; N_TOP],
        }
    }

    /// What the guest reads out of one register in the tree.
    ///
    /// ⚠ `LeafTrigger` reads as zero and that is the register's own declaration — it is
    /// `-W-4R`, write-only (`ga102/dev_vm.h:61`). ⊘ Answering it with the last vector
    /// written would be inventing a readable field on a write-only register, which is the
    /// *plausible wrong answer* this port's plane docs argue is worse than a defaulted one.
    #[must_use]
    pub fn read(&self, reg: CpuIntrReg) -> u32 {
        match reg {
            CpuIntrReg::Leaf(i) => self.leaf[i],
            // ★ Both the SET and the CLEAR alias read back the *enable mask*. They are two
            // write ports onto one state, which is what `RW-4A` with a shared `_VALUE_INIT`
            // says, and it is what `intrReadRegLeafEnSet_TU102` relies on when
            // `intrGetNonStallEnable_TU102` tests `maskLo & intrReadRegTopEnSet(...)`
            // (`ogkm-580: intr_nonstall_tu102.c:65-69`).
            CpuIntrReg::LeafEnSet(i) | CpuIntrReg::LeafEnClear(i) => self.leaf_en[i],
            CpuIntrReg::Top(i) => self.top[i],
            CpuIntrReg::TopEnSet(i) | CpuIntrReg::TopEnClear(i) => self.top_en[i],
            CpuIntrReg::LeafTrigger => 0,
        }
    }

    /// Apply one write. Returns `Some` only for `LEAF_TRIGGER`, which is the one write that
    /// asks for anything of the outside world.
    pub fn write(&mut self, reg: CpuIntrReg, val: u32) -> Option<TriggerOutcome> {
        match reg {
            // ★★★ WRITE-1-TO-CLEAR, and it is the half of this module the ISR depends on.
            // `intrClearLeafVector_TU102` writes `NVBIT(bit)` to clear exactly that bit
            // (`ogkm-580: intr_tu102.c:648-663`). ⊘ A plain store here would clear every
            // *other* pending bit as a side effect of clearing one, and the symptom would
            // be a lost interrupt in a subsystem nobody was looking at.
            CpuIntrReg::Leaf(i) => {
                self.leaf[i] &= !val;
                self.recompute_top();
                None
            }
            CpuIntrReg::LeafEnSet(i) => {
                self.leaf_en[i] |= val;
                None
            }
            CpuIntrReg::LeafEnClear(i) => {
                self.leaf_en[i] &= !val;
                None
            }
            // ⊘ Read-only per `R--4A` (`ga102/dev_vm.h:26`). The pending subtree bits are
            // a *function* of the leaves — see `recompute_top` — so accepting a write here
            // would let the guest desynchronise TOP from LEAF, and the ISR reads both.
            CpuIntrReg::Top(_) => None,
            CpuIntrReg::TopEnSet(i) => {
                self.top_en[i] |= val;
                None
            }
            CpuIntrReg::TopEnClear(i) => {
                self.top_en[i] &= !val;
                None
            }
            CpuIntrReg::LeafTrigger => {
                Some(self.latch((u64::from(val) & TRIGGER_VECTOR_MASK) as u32))
            }
        }
    }

    /// ★★★ **Latch a vector this DEVICE raises** — the entry point that is not a guest
    /// `LEAF_TRIGGER` write (`execution_plane_increments.md` §14.18, step 1).
    ///
    /// # Why it had to exist, and why it is the *same* code as the trigger
    ///
    /// Until §14.18 the only producer of a pending bit was the guest asking the device to
    /// interrupt the guest — the `_osVerifyInterrupts` loopback. A **completion** vector is
    /// the opposite direction: the device announcing that an engine finished the guest's
    /// work, which is the whole of what
    /// [`crate::DoorbellReport::ServedLocally`] now owes its notifier arming
    /// (`kayfabe_abi::eventnotify`'s index-35 argument).
    ///
    /// ★ It shares [`CpuIntrTree::write`]'s body rather than duplicating it, because the
    /// guest's ISR cannot tell the two apart and neither may the pending state: the leaf
    /// bit, the derived TOP summary and the mask reading are one description of the tree,
    /// and a second latch that maintained them slightly differently is the two-places-must-
    /// agree defect [`CpuIntrTree::recompute_top`] already argues against.
    ///
    /// ⊘ **The caller must have witnessed the work.** This function cannot check that and
    /// does not try; what it guarantees is only that a latch is honest *about the tree*.
    /// The one caller is [`crate::RegPlane::ring_doorbell`], and it latches only on a
    /// `ServedLocally` report — after the bytes moved and the finishPayload advanced.
    ///
    /// ⚠ `vector` is a **GPU interrupt-tree vector**, e.g. `vectorNonStall` out of the
    /// chip's interrupt table — not an MSI-X index. It is masked to
    /// `LEAF_TRIGGER_VECTOR`'s `11:0` here as well, so the two entry points cannot
    /// disagree about the field width the register block declares.
    pub fn latch(&mut self, vector: u32) -> TriggerOutcome {
        let vector = (u64::from(vector) & TRIGGER_VECTOR_MASK) as u32;
        let leaf = vector_to_leaf_reg(vector);
        let bit = vector_to_leaf_bit(vector);
        let subtree = vector_to_subtree(vector);
        let top_idx = subtree_to_top_idx(subtree);
        if leaf >= N_LEAF || top_idx >= N_TOP {
            return TriggerOutcome {
                vector,
                out_of_range: true,
                would_be_masked: false,
            };
        }
        let top_bit = subtree_to_top_bit(subtree);
        let masked = (self.leaf_en[leaf] & (1u32 << bit)) == 0
            || (self.top_en[top_idx] & (1u32 << top_bit)) == 0;
        self.leaf[leaf] |= 1u32 << bit;
        self.top[top_idx] |= 1u32 << top_bit;
        TriggerOutcome {
            vector,
            out_of_range: false,
            would_be_masked: masked,
        }
    }

    /// TOP is a summary of LEAF, recomputed rather than tracked.
    ///
    /// ★★★ **Derived, not maintained**, and that is the whole reason this is a function.
    /// A subtree bit cleared by hand when "the last leaf went quiet" is a
    /// two-places-must-agree invariant, and the failure mode is a TOP bit that stays set
    /// over an empty subtree — which makes the guest's ISR walk leaves forever finding
    /// nothing. `C: src/qemu/nvkvm_gpu_emul.c:4398-4411` maintains it by hand and has to
    /// bounds-check the odd leaf of the pair inline; deriving it cannot get that wrong.
    fn recompute_top(&mut self) {
        for (t, slot) in self.top.iter_mut().enumerate() {
            let mut bits = 0u32;
            for sub_bit in 0..32u32 {
                let subtree = (t as u32) * 32 + sub_bit;
                let lo = (subtree * 2) as usize;
                let hi = lo + 1;
                let pending = self.leaf.get(lo).copied().unwrap_or(0)
                    | self.leaf.get(hi).copied().unwrap_or(0);
                if pending != 0 {
                    bits |= 1u32 << sub_bit;
                }
            }
            *slot = bits;
        }
    }

    /// Whether any vector is pending anywhere — what a level-triggered line would follow.
    ///
    /// ⊘ Nothing in this port uses it to *deassert*: the only delivery path this device has
    /// is message-signalled, and an MSI-X message is an edge with nothing to lower. It
    /// exists so a shell that one day advertises a legacy pin has the fact available rather
    /// than having to recompute it from the arrays. See `kayfabe_vmm::IrqSpec::IntxLevel`,
    /// which the QEMU adapter refuses today.
    #[must_use]
    pub fn any_pending(&self) -> bool {
        self.top.iter().any(|t| *t != 0)
    }
}
