//! # The RM capability tables — Axis A's **default-deny** control/class allowlist
//!
//! A port of the C research artifact's nvproxy-derived allowlists
//! (`C: src/qemu/nvkvm_ctrl_allowlist.h`, `C: src/qemu/nvkvm_fe_alloc_allowlist.h`),
//! which were themselves extracted from gVisor's `nvproxy` and then validated in Mode 1
//! against 22 real GPU applications at host parity. It is a **port, not a derivation**:
//! no entry here was invented, and the provenance of every row is [`Origin`].
//!
//! ## The gap this closes
//!
//! nvproxy default-**denies**: `rmControl` looks the command up in a map and answers
//! `NV_ERR_NOT_SUPPORTED` when the handler is nil
//! (`gvisor/pkg/sentry/devices/nvproxy/frontend.go:805-813`). This port default-
//! **allowed**: `kayfabe_fwd::classify_control` answered `Forwarded` for everything that
//! was not Case-2, and any class at all could be alloc'd. The asymmetry was recorded as a
//! finding (`docs/design/eight_blockers_resolved.md` §6). After this module, a control or
//! a class this project has never seen is refused **by name** before anything decodes it.
//!
//! ## Where the gate is, and why there is exactly one
//!
//! At the **guest ingress** — `kayfabe_rmrpc::translate`, before the params decoder. That
//! is where the C put it too (in QEMU, ahead of the isolate, *"because the guest kernel
//! module is untrusted"*), and one gate is the governing directive: port the C, do not
//! redesign it. A second gate at the host egress (`kayfabe-fwd`) would be a second source
//! of truth for the same question, and that crate cannot see this one anyway — it has no
//! `kayfabe-abi` dependency, deliberately.
//!
//! ★★ **What that means for the transport, and it is the port's main finding.** The C
//! gated *ioctls*; Mode 2's guest sends *GSP RPCs*. The command space is the same but the
//! traffic is not, and the difference is measurable rather than theoretical: **not one**
//! of the six controls this port already names is on the C's list — the four
//! page-directory commands ([`crate::versions::ControlParams`]) and the two canonical
//! Case-2 commands (`NV2080_CTRL_CMD_GPU_PROMOTE_CTX`,
//! `NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_INFO`; `docs/design/execution_plane.md` §2.5). In
//! Mode 1 the guest's own kernel driver issues all six to *its* GSP and none of them
//! crosses a userspace ioctl boundary, so a list validated against 22 real applications
//! could not have contained them. They are carried as [`Origin::Mode2Rpc`], which is the
//! honest label for *"the C could not have known about this row"* — and the six are a
//! floor, not a census: the rest of the Mode-2 delta is unknown until a GSP boots and the
//! refusals are read off.
//!
//! ★★★ **"Default-deny" is true of half the command space, not all of it.** The
//! GSS-legacy rule below passes every command with bit 15 set — that is
//! 2³¹ values, with no table row and no review. It is nvproxy's rule and the C's, ported
//! verbatim because this is a port; but the sentence *"an unknown control is refused"*
//! is only true where that bit is clear, and
//! `the_gss_legacy_rule_passes_half_the_command_space` pins the fact so nobody has to
//! rediscover it. Narrowing it is a design decision on evidence nobody has yet.
//!
//! ## Ordering, which is a COHERENCE property of this table
//!
//! ⊘ **Not the security property, and F5 corrected an earlier heading that said it was.**
//! All six controls this table denies are `NON_PRIVILEGED` in RM, so **the denylist does
//! not carry P** (the blast-radius property). What carries P is that RM re-derives
//! privilege on *every* ioctl (`ogkm-580: escape.c:304`) and its control dispatch is
//! kernel-privileged by default, closing **613 of 1359** exported entries (45.1%) to any
//! userspace caller regardless of anything decided here — see
//! `docs/design/guest_blast_radius.md` §3.4. This table is **defence in depth over a
//! decision the driver makes again anyway**; the ordering below is what keeps the *table*
//! coherent, which is worth having and is a smaller claim.
//!
//! [`CapabilityTable::control`] answers in this order, and the order is what the
//! `deny_beats_the_rule_based_passthrough` test pins:
//!
//! 1. an **explicit denial** ([`DeniedEntry`]) — named, with a reason;
//! 2. the two **rule-based passthroughs** the C implements in code rather than in a
//!    table: the GSS-legacy mask ([`RM_GSS_LEGACY_MASK`]) and the binary-API class
//!    ([`NV2081_BINAPI_CLASS`]). Both are GSP-routed with no app pointers
//!    (`gvisor/pkg/sentry/devices/nvproxy/frontend.go:756-816`);
//! 3. the **allowlist** — this boundary's **own** blocks first, then the shared base;
//! 4. otherwise [`Denial::NotOnAllowlist`].
//!
//! Denial first is stricter than nvproxy, which checks its two rules *before* the map. It
//! costs nothing today — the two sets are provably disjoint, which is its own test — and
//! it means a future "this is dangerous" row cannot be silently outvoted by a bit.
//!
//! ## The version seam — **shared base + per-boundary declaration**, depth two, no chain
//!
//! ★★★ **A boundary can REMOVE, not only add** (task #122). The shape that made removal
//! inexpressible was *inherit-then-add*: [`CapabilityTable`] carried an `inherits`
//! pointer, a lookup walked it, and a row placed at the bottom leaked into every
//! boundary above whether or not the vendor still had it. That is not a missing field —
//! it is the wrong shape, because *inheritance is exactly the thing that makes a removal
//! unsayable*.
//!
//! What is here instead: **one shared base holding only what every declared boundary
//! shares**, and each boundary naming its **own** blocks explicitly. Nothing is inherited
//! from a neighbour, so there is no delta chain to replay and no ordering to reason
//! about:
//!
//! ```text
//! resolved(boundary) = SHARED_CAPS  ∪  own_blocks(boundary)
//! ```
//!
//! A **removal** is then just a boundary not naming a block. 575.51.02 does not name
//! [`CONTROLS_DRAM_ENCRYPTION_570`], and that one absent word *is* the deletion nvproxy
//! spells with two `delete()` calls (`gvisor nvproxy: version.go:1039-1040`). A
//! **replacement** is a boundary not naming the old block and naming a new one — which
//! is why the same command word can carry two different NVIDIA names at two boundaries,
//! the thing an additive table could not say at all.
//!
//! ★★ **Why (b) and not inherit-then-{add, subtract}.** Both express *replace*. The
//! difference is what a mistake costs. In a delta chain an early subtract silently
//! shrinks every later boundary's set, and this repo has been bitten by that exact shape
//! before — shortening a list weakened a gate with zero red tests
//! (`docs/design/testing_doctrine.md`, the *gates quantified over a list* incidents).
//! Here every boundary's content is one line of block names, readable without holding
//! the chain in your head, and a wrong block name changes **one** boundary. The cost is
//! that a block shared by four boundaries is named four times — paid deliberately, and
//! `each_boundarys_resolved_delta_is_materialised` prints the resolved set anyway so the
//! effect of an edit is visible per-boundary rather than implied.
//!
//! ★ **One axis: DRIVER VERSION.** The owner's phrasing said *arch*; the data is
//! version-keyed, because the only source for it — nvproxy's registry — is a chain of
//! driver versions and there is no arch-keyed capability source to port. So a
//! [`CapabilityTable`] is reached through [`crate::versions::DriverAbiTable`] and through
//! nothing else. The shape is variant-agnostic (a shared base plus per-variant blocks
//! composes over any axis, and intersection is associative), so an arch axis later is
//! more variants over the same struct — but building it now would be rows no traffic can
//! reach, which the module already refuses to do elsewhere.
//!
//! Eight boundaries are wired, all read out of nvproxy's own chain
//! (`gvisor nvproxy: version.go`):
//!
//! | boundary | what changes | nvproxy |
//! |---|---|---|
//! | 550.54.04 | the shared base + `NVC36F_CTRL_GET_CLASS_ENGINEID` | `version.go:360` |
//! | 550.90.07 | +`NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE` | `version.go:906` |
//! | 555.42.02 | **−**`NVC36F_CTRL_GET_CLASS_ENGINEID` | `version.go:933` |
//! | 560.28.03 | +8 alloc classes, +`NV_SEMAPHORE_SURFACE_CTRL_CMD_UNBIND_CHANNEL` | `version.go:945-977` |
//! | 570.86.15 | +6 alloc classes, +2 DRAM-encryption controls | `version.go:990-1027` |
//! | 575.51.02 | **−**those 2, +2 renumbered, +`THERMAL_SYSTEM_EXECUTE_V2` | `version.go:1036-1053` |
//! | 580.65.06 | +2 alloc classes, `NVCEB7`/`NVD1B7` | `version.go:1057-1078` |
//! | 610.43.02 | nothing: the 580 surface, declared again rather than inherited | — |
//!
//! ★ **The limit that remains.** Only the rows this port carries are split — nvproxy's
//! `controlCmd` map also gains graphics-, profiling- and fabric-capability rows at
//! 560.28.03 / 565.57.01 / 570.86.15 / 580.65.06 that the C's compute filter excluded and
//! this port therefore never had. Those are absent at every boundary here, which is the
//! same answer the C gave; splitting them would be splitting rows that do not exist.
//!
//! ## Deliberately not ported
//!
//! - **The frontend-ioctl NR allowlist** (23 rows, `C: nvkvm_fe_alloc_allowlist.h`) and
//!   **the UVM schema** (31 rows, `C: nvkvm_isolate_handlers.c:516`). Both gate an
//!   *ioctl* transport that Mode 2 does not have: the guest's `nvidia-uvm` talks to the
//!   guest's own `nvidia` module, and neither its ioctls nor the frontend escapes ever
//!   reach us. Porting them now would be rows no traffic can reach, which is the
//!   opposite of coverage.
//! - **The 1 MiB inner-params cap** (`RMAPI_PARAM_COPY_MAX_PARAMS_SIZE`, enforced in the
//!   C's code). [`kayfabe_rmrpc`]'s `MAX_REASSEMBLED_BODY` already binds at 64 KiB, which
//!   is sixteen times tighter, so the C's number could never fire on this transport.
//!
//! [`kayfabe_rmrpc`]: https://docs.rs/kayfabe-rmrpc

use kayfabe_arch::ids::{ClassId, ControlCmd};

/// `RM_GSS_LEGACY_MASK` — a command with this bit is a legacy GSS/GSP control whose
/// params cannot contain app pointers, so nvproxy passes it through without a map entry
/// (`gvisor/pkg/abi/nvgpu/ctrl.go:23`, applied at
/// `gvisor/pkg/sentry/devices/nvproxy/frontend.go:756-816`).
pub const RM_GSS_LEGACY_MASK: u32 = 0x0000_8000;

/// `NV2081_BINAPI` — the binary-API subdevice class. Every command on it forwards
/// straight to GSP, so nvproxy passes the whole class through
/// (`gvisor/pkg/abi/nvgpu/classes.go:69`).
pub const NV2081_BINAPI_CLASS: u32 = 0x2081;

/// Where a permitted row came from. Not decoration: it is what distinguishes a row with
/// an upstream oracle from one this project added on its own evidence, and the two are
/// audited differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// gVisor `nvproxy`'s 575-ABI map, replayed through its version chain
    /// (`C: docs/audits/nvproxy_control_allowlist.md`,
    /// `C: docs/audits/nvproxy_frontend_alloc.md`).
    Nvproxy,
    /// Observed on a real host RM ioctl stream and added by the C with an argument in
    /// its own comments — the caps queries the guest's CUDA driver library issues at
    /// init that nvproxy's app never exercised, and the `GLX_nvidia` enumeration surface
    /// (`C:` task #84).
    ///
    /// ★ The CUDA library's real file name is spelled out nowhere here on purpose: it
    /// contains a substring the hexagonal-boundary gate refuses in a logic crate, and the
    /// gate's own instruction is *"reword the comment; never weaken this gate"*.
    Empirical,
    /// ★ **Reaches us only because Mode 2's transport is GSP RPC.** Never on the C's
    /// list, because as an ioctl it never crossed the boundary the C was gating.
    ///
    /// ★★ **Corrected 2026-08-01: this is a provenance, not a control-only tag.** The
    /// sentence here used to be *"every row here has a consumer in
    /// [`crate::versions::DriverAbiTable::control_params`]"*, which was true of the six
    /// [`ControlEntry`] rows and quietly asserted that only controls could ever carry
    /// this origin. They cannot: the same argument holds for an **allocation class** the
    /// guest's own kernel RM asks for, and `NV01_EVENT_KERNEL_CALLBACK_EX` is the first
    /// one (see its row in [`CLASSES_SHARED`]). The obligation the sentence was really
    /// making is the one that generalises — **a row with this origin has a consumer in
    /// the table that decides its params shape**: [`crate::versions::DriverAbiTable::control_params`]
    /// for a control, [`crate::versions::DriverAbiTable::alloc_params`] for a class.
    /// Nothing carries this tag on the strength of a header alone.
    Mode2Rpc,
}

/// One permitted control command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlEntry {
    /// The `cmd` word.
    pub cmd: u32,
    /// NVIDIA's own name for it. Load-bearing for review, and what a refusal test asserts
    /// against — a hex number alone makes a wrong row invisible.
    pub name: &'static str,
    /// Provenance.
    pub origin: Origin,
}

/// One permitted allocation class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassEntry {
    /// `hClass`.
    pub class: u32,
    /// NVIDIA's own name for it.
    pub name: &'static str,
    /// Provenance.
    pub origin: Origin,
}

/// Why a row is refused *by name*, rather than merely being absent.
///
/// ★ The distinction is the whole point of [`DeniedEntry`]: "we never heard of this" and
/// "we know exactly what this is and it does not cross this boundary" are different
/// findings, and a census that collapses them cannot tell a guest probing the surface
/// from a guest we simply have not modelled yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeniedBecause {
    /// Arbitrary GPU register peek/poke. nvproxy gates these on `CapProfiling` and calls
    /// that capability *privileged*; there is no version of this we want reachable from a
    /// guest.
    RegisterAccess,
    /// Hardware performance counters / HWPM streaming — the profiler surface, also
    /// `CapProfiling` in nvproxy.
    PerformanceCounters,
    /// NVLink / fabric IMEX management (`CapFabricIMEXManagement`). A host-topology verb;
    /// the guest has no fabric.
    FabricManagement,
    /// Privileged video memory. nvproxy deliberately omits the class and so do we.
    PrivilegedMemory,
    /// Pins a descriptor over the **caller's own** address range. In Mode 2 the caller is
    /// the guest kernel and the range is guest RAM, so honouring it would hand the host
    /// driver a guest-chosen pointer.
    CallerMemoryDescriptor,
    /// ★★★ **SM-level debugger / profiler trapping, which this port does not model — and
    /// `NV83DE` resume is NOT MMU fault replay.**
    ///
    /// The two share no code. MMU replay is `MEM_OP_C.TLB_INVALIDATE_REPLAY` on a
    /// pushbuffer (`ogkm-580: kernel-open/nvidia-uvm/uvm_volta_host.c:234-264`);
    /// `NV83DE_CTRL_CMD_DEBUG_RESUME_CONTEXT` resumes a *warp* from an SM exception. This
    /// port implements neither, and the reason it must say so out loud is that the
    /// surface is **reachable by an unprivileged guest application**: `GT200_DEBUGGER`
    /// allocates with `RS_FLAGS_ALLOC_NON_PRIVILEGED`
    /// (`ogkm-580: src/nvidia/src/kernel/rmapi/resource_list.h:186-196`) and the debug
    /// controls carry flags `0x10248` — `NON_PRIVILEGED`
    /// (`ogkm-580: src/nvidia/generated/g_kernel_sm_debugger_session_nvoc.c:562`, `:577`).
    ///
    /// ⚠ It needs SM error state, warp trap handling and single-step on a GR context
    /// whose golden image is the silicon boundary this project forwards across, so it is
    /// **out of scope permanently**, not merely unbuilt
    /// (`docs/design/resume_from_fault.md` §S4, §8 row L2).
    ///
    /// ★★ And refusing it is what keeps task #111 honest. *MMU debug mode*
    /// (`NV83DE_CTRL_CMD_DEBUG_SET_MODE_MMU_DEBUG`) changes what RM does on a fault:
    /// `kgmmuServiceMmuFault_GV100` writes the error notifier and resets the channel
    /// **only if it is disabled**
    /// (`ogkm-580: src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:2059-2073`,
    /// `:2207-2211`). An emitter that ignored that flag would kill a context a debugger
    /// explicitly asked it to preserve. Denying the control means the flag can never be
    /// set, which closes the sub-case instead of leaving it to be remembered.
    SmDebuggerTrapping,
    /// ★★ A **fault-reporting mechanism this port does not implement**, refused rather
    /// than succeeded.
    ///
    /// Answering `NV_OK` to a fault-plumbing verb we do not service is the shape this
    /// project has named most often: the guest arms a path, believes it armed, and the
    /// path is inert. `docs/design/resume_from_fault.md` §7 step 1 calls the C's generic
    /// `NV_OK` echo here *"a false green"*; this is the row that makes the refusal say
    /// which mechanism, rather than *"never heard of it"*.
    FaultMechanismNotModelled,
    /// ★★★ **A physical board bus this emulated GPU does not have — and RM's own source
    /// says the alloc is expected to fail.**
    ///
    /// `NV40_I2C` is the handle onto the card's I²C/SMBus segments: DDC/EDID on the display
    /// connectors, board thermal sensors, gsync. There is no such bus behind this device,
    /// and nothing this port could put behind the class would be a reading of anything.
    ///
    /// ★ The refusal costs the guest nothing, and that is **measured in RM's own code, not
    /// assumed**. The sole in-tree allocator is the UNIX bootstrap, and the call site is
    /// wrapped in a comment saying so (`ogkm-580:
    /// src/nvidia/arch/nvalloc/unix/src/osinit.c:1764-1778`, verbatim at `ogkm-610:
    /// :1835-1847`):
    ///
    /// ```text
    /// // The NV40_I2C allocation expected to fail, if it is disabled with RM config.
    /// if (pRmApi->Alloc(..., NV40_I2C, NULL, 0) != NV_OK) { nv->rmapi.hI2C = 0; }
    /// ```
    ///
    /// No `goto fail`, no propagation: `RmUnixAllocRmApi` still returns `NV_TRUE`, and
    /// every consumer is handle-guarded (`RmI2cAddGpuPorts` is `if (pNv->rmapi.hI2C != 0)`,
    /// `ogkm-580: osapi.c:4101`), so the GPU simply exposes no i2c adapters.
    /// `[measured 2026-08-08, boots ship_7a881a7 and ship_7a881a7_b, rev 7a881a7]` the
    /// adapter initialises and `nvidia-smi` enumerates the device with this class refused.
    ///
    /// ⊘ **Serving it would be the worse shape, for `GT200_DEBUGGER`'s exact reason.** The
    /// alloc itself is trivial to fake — `RS_NONE` params, and `i2capiConstruct` is a
    /// `return NV_OK` (`ogkm-580: src/nvidia/src/kernel/gpu/i2c/i2c_api.c:26-35`). But the
    /// five `0x402c01xx` controls behind it have **no kernel-side implementation in the
    /// open tree at all** (`g_i2c_api_nvoc.c:130-206`, flags `0x48` = route-to-physical),
    /// so accepting the attach means inventing a port map for a bus that does not exist —
    /// too-capable, which is the same defect as too-strict
    /// (`mock_fidelity_both_directions`).
    NoPhysicalBoardBus,
}

/// A row this port refuses deliberately, with a reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeniedEntry {
    /// The `cmd` word or `hClass`.
    pub id: u32,
    /// NVIDIA's own name for it.
    pub name: &'static str,
    /// Why.
    pub why: DeniedBecause,
}

/// Why a control or class did not pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denial {
    /// On the explicit deny table: named, with a reason.
    Refused {
        /// NVIDIA's own name for it.
        name: &'static str,
        /// Why.
        why: DeniedBecause,
    },
    /// ★ **The default.** Not on any list — the answer for everything this project has
    /// never seen, which is the state the whole module exists to make loud.
    NotOnAllowlist,
}

/// What the boundary makes of a control command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum ControlPermit {
    /// On the allowlist.
    Listed {
        /// NVIDIA's own name for it.
        name: &'static str,
        /// Provenance.
        origin: Origin,
    },
    /// Passed by the GSS-legacy rule ([`RM_GSS_LEGACY_MASK`]) — no table row needed.
    GssLegacyRule,
    /// Passed by the binary-API class rule ([`NV2081_BINAPI_CLASS`]).
    BinApiRule,
    /// Refused.
    Denied(Denial),
}

impl ControlPermit {
    /// Did it pass?
    #[must_use]
    pub const fn is_permitted(self) -> bool {
        !matches!(self, ControlPermit::Denied(_))
    }

    /// Which **rule** admitted it, if a rule did rather than a table row.
    ///
    /// `None` for [`Self::Listed`] (a row named it) and for [`Self::Denied`]. See
    /// [`PassthroughRule`] for why a caller downstream of the gate needs to be able to
    /// tell the two apart.
    #[must_use]
    pub const fn passthrough_rule(self) -> Option<PassthroughRule> {
        match self {
            ControlPermit::GssLegacyRule => Some(PassthroughRule::GssLegacy),
            ControlPermit::BinApiRule => Some(PassthroughRule::BinApi),
            ControlPermit::Listed { .. } | ControlPermit::Denied(_) => None,
        }
    }
}

/// Which of the two **rule-based passthroughs** admitted a control that no table row
/// names.
///
/// ## The two rules share one justification, and it is nvproxy's
///
/// Both say: this command is serviced by the **GPU System Processor**, so its parameters
/// cannot reasonably contain application pointers and the sentry need not model them.
/// nvproxy states it in as many words at
/// `gvisor/pkg/sentry/devices/nvproxy/frontend.go:769-780` —
///
/// > *"This is a 'legacy GSS control' that is implemented by the GPU System Processor
/// > (GSP). Consequently, its parameters cannot reasonably contain application pointers,
/// > and the control is in any case undocumented."*
///
/// — and then hands the blob to `rmControlSimple` (`frontend.go:818`), which copies a
/// bounded flat byte range and never interprets a pointer. That is **principled, not
/// lazy**: nvproxy's security property is *pointer-translation safety*, and it is
/// preserved by construction. Semantic validation is left to the host RM, which is
/// downstream and real.
///
/// ## ★★★ Why this enum exists: the second half of that sentence does not transfer
///
/// nvproxy is a **Mode-1-shaped** situation — the guest's ioctl is replayed against a real
/// host `/dev/nvidia*`, so a real GSP does eventually answer, and "the host RM validates
/// it" is true. In Mode 2 the guest's GSP **is ours**. A command admitted by one of these
/// rules is, *by the rule's own definition*, one that nothing downstream will service:
/// there is no host RM behind our fake GSP to be the adult in the room. So the rule
/// answers *"may the guest send it?"* — which is all a sandbox needs — and leaves
/// *"what do we answer?"* wide open.
///
/// [`ControlPermit::is_permitted`] therefore has a **narrower meaning than its name
/// suggests** for these two arms, and a consumer that reads `Listed` and a rule arm as the
/// same fact has silently inherited a premise that is false here.
/// [`ControlPermit::passthrough_rule`] is how a consumer keeps them apart;
/// `kayfabe_rmrpc::BridgeRefusal::GspRuleControlUnserviced` is the consumer that does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub enum PassthroughRule {
    /// [`RM_GSS_LEGACY_MASK`] — bit 15 of the command word. Half the command space.
    GssLegacy,
    /// [`NV2081_BINAPI_CLASS`] — the whole binary-API subdevice class.
    BinApi,
}

impl PassthroughRule {
    /// The rule's own name, for a census tag or a message.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            PassthroughRule::GssLegacy => "GssLegacy",
            PassthroughRule::BinApi => "BinApi",
        }
    }
}

/// What the boundary makes of an allocation class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum AllocPermit {
    /// On the allowlist.
    Listed {
        /// NVIDIA's own name for it.
        name: &'static str,
        /// Provenance.
        origin: Origin,
    },
    /// Refused.
    Denied(Denial),
}

impl AllocPermit {
    /// Did it pass?
    #[must_use]
    pub const fn is_permitted(self) -> bool {
        !matches!(self, AllocPermit::Denied(_))
    }
}

/// The rows **every** declared boundary shares — the floor, after everything that is not
/// universally shared has been stripped out of it.
///
/// ★★★ *Stripped*, not *inherited*. A row lives here only if every boundary in
/// [`crate::versions::TABLES`] has it; the moment one boundary does not, the row moves
/// out into per-boundary blocks. That is what makes a removal expressible at all, and
/// `the_shared_base_holds_only_what_every_boundary_shares` is the gate that keeps it
/// honest in both directions: nothing here may be missing from a boundary, and nothing
/// that *every* boundary owns may stay outside.
///
/// The two deny lists live here and nowhere else, and that is a claim about their
/// content rather than an omission. They are **this project's policy** — register
/// peek/poke, HWPM, fabric, privileged memory, caller-supplied descriptors — and a
/// hazard of that kind does not appear or disappear with a driver version, so there is
/// no boundary at which one of them would differ. The one thing that *could* make a
/// version-specific denial necessary is a command word being **repurposed** across a
/// boundary, which is precisely what 575.51.02 does to `0x20801358`; so
/// `no_denied_id_is_a_boundary_specific_control` asserts that no denied id is a
/// per-boundary row, and that test — not this paragraph — is what will fire first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedCapabilities {
    controls: &'static [ControlEntry],
    classes: &'static [ClassEntry],
    denied_controls: &'static [DeniedEntry],
    denied_classes: &'static [DeniedEntry],
}

/// One driver boundary's **complete** capability surface, stated as
/// `shared ∪ own_blocks`.
///
/// Depth is **two, always**: there is no `inherits` pointer and no chain to walk. A
/// boundary that must *not* have a row simply does not name the block that carries it —
/// see the module doc, and [`CAPS_575_51_02`] for the case that motivated the shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityTable {
    shared: &'static SharedCapabilities,
    /// The control blocks **this** boundary has beyond [`SharedCapabilities`], named in
    /// full rather than inherited. Disjoint from the shared set and from each other —
    /// `no_boundary_repeats_a_shared_or_duplicated_row` is the gate.
    own_controls: &'static [&'static [ControlEntry]],
    /// The class blocks this boundary has beyond [`SharedCapabilities`].
    own_classes: &'static [&'static [ClassEntry]],
    /// Why this boundary exists — the same in-the-data justification
    /// [`crate::versions::DriverAbiTable::note`] carries.
    pub note: &'static str,
}

fn find_control(rows: &'static [ControlEntry], cmd: u32) -> Option<&'static ControlEntry> {
    rows.binary_search_by_key(&cmd, |e| e.cmd)
        .ok()
        .map(|i| &rows[i])
}

fn find_class(rows: &'static [ClassEntry], class: u32) -> Option<&'static ClassEntry> {
    rows.binary_search_by_key(&class, |e| e.class)
        .ok()
        .map(|i| &rows[i])
}

fn find_denied(rows: &'static [DeniedEntry], id: u32) -> Option<&'static DeniedEntry> {
    rows.binary_search_by_key(&id, |e| e.id)
        .ok()
        .map(|i| &rows[i])
}

impl CapabilityTable {
    /// Every permitted control **at this boundary**: its own blocks, then the shared
    /// base. Sorted within a block; unordered across blocks.
    ///
    /// ★ This is the **resolved** set, not a delta — which is the property the old shape
    /// could not offer, because there `all_controls` meant "everything anyone below me
    /// ever added". A census taken here is this boundary's answer and nobody else's.
    pub fn all_controls(&'static self) -> impl Iterator<Item = &'static ControlEntry> {
        self.own_controls
            .iter()
            .flat_map(|b| b.iter())
            .chain(self.shared.controls.iter())
    }

    /// Every permitted class at this boundary — the resolved set.
    pub fn all_classes(&'static self) -> impl Iterator<Item = &'static ClassEntry> {
        self.own_classes
            .iter()
            .flat_map(|b| b.iter())
            .chain(self.shared.classes.iter())
    }

    /// Every explicitly-denied control. Shared by every boundary — see
    /// [`SharedCapabilities`] for why that is a claim and not an omission.
    pub fn all_denied_controls(&'static self) -> impl Iterator<Item = &'static DeniedEntry> {
        self.shared.denied_controls.iter()
    }

    /// Every explicitly-denied class.
    pub fn all_denied_classes(&'static self) -> impl Iterator<Item = &'static DeniedEntry> {
        self.shared.denied_classes.iter()
    }

    /// May the guest issue this control? **Default-deny.** See the module doc for the
    /// order the four answers are decided in.
    pub fn control(&'static self, cmd: ControlCmd) -> ControlPermit {
        if let Some(d) = find_denied(self.shared.denied_controls, cmd.0) {
            return ControlPermit::Denied(Denial::Refused {
                name: d.name,
                why: d.why,
            });
        }
        if cmd.0 & RM_GSS_LEGACY_MASK != 0 {
            return ControlPermit::GssLegacyRule;
        }
        if (cmd.0 >> 16) & 0xffff == NV2081_BINAPI_CLASS {
            return ControlPermit::BinApiRule;
        }
        // ★ Own blocks before the shared base. The two are disjoint by construction
        // (`no_boundary_repeats_a_shared_or_duplicated_row`), so this order cannot change
        // an answer today — it is written this way so that if the invariant is ever
        // broken, the BOUNDARY-SPECIFIC row wins, which is the direction a version table
        // must fail in.
        for block in self.own_controls {
            if let Some(e) = find_control(block, cmd.0) {
                return ControlPermit::Listed {
                    name: e.name,
                    origin: e.origin,
                };
            }
        }
        if let Some(e) = find_control(self.shared.controls, cmd.0) {
            return ControlPermit::Listed {
                name: e.name,
                origin: e.origin,
            };
        }
        ControlPermit::Denied(Denial::NotOnAllowlist)
    }

    /// May the guest allocate this class? **Default-deny.**
    pub fn alloc_class(&'static self, class: ClassId) -> AllocPermit {
        if let Some(d) = find_denied(self.shared.denied_classes, class.0) {
            return AllocPermit::Denied(Denial::Refused {
                name: d.name,
                why: d.why,
            });
        }
        for block in self.own_classes {
            if let Some(e) = find_class(block, class.0) {
                return AllocPermit::Listed {
                    name: e.name,
                    origin: e.origin,
                };
            }
        }
        if let Some(e) = find_class(self.shared.classes, class.0) {
            return AllocPermit::Listed {
                name: e.name,
                origin: e.origin,
            };
        }
        AllocPermit::Denied(Denial::NotOnAllowlist)
    }
}
/// The controls **every** declared boundary permits.
///
/// ★ Five rows the C's 575-era list carried were **stripped out of here** by task #122,
/// because nvproxy does not have all five at all eight boundaries: they live in the
/// per-boundary blocks below. Sorted by `cmd` — [`CapabilityTable::control`]
/// binary-searches it.
pub(crate) static CONTROLS_SHARED: &[ControlEntry] = &[
    ControlEntry { cmd: 0x00000101, name: "NV0000_CTRL_CMD_SYSTEM_GET_BUILD_VERSION", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000102, name: "NV0000_CTRL_CMD_SYSTEM_GET_CPU_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000127, name: "NV0000_CTRL_CMD_SYSTEM_GET_P2P_CAPS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x0000012b, name: "NV0000_CTRL_CMD_SYSTEM_GET_P2P_CAPS_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000136, name: "NV0000_CTRL_CMD_SYSTEM_GET_FABRIC_STATUS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x0000013a, name: "NV0000_CTRL_CMD_SYSTEM_GET_P2P_CAPS_MATRIX", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x000001f0, name: "NV0000_CTRL_CMD_SYSTEM_GET_FEATURES", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000201, name: "NV0000_CTRL_CMD_GPU_GET_ATTACHED_IDS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000202, name: "NV0000_CTRL_CMD_GPU_GET_ID_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000204, name: "NV0000_CTRL_CMD_GPU_GET_DEVICE_IDS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000205, name: "NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000214, name: "NV0000_CTRL_CMD_GPU_GET_PROBED_IDS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000215, name: "NV0000_CTRL_CMD_GPU_ATTACH_IDS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000216, name: "NV0000_CTRL_CMD_GPU_DETACH_IDS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x0000021b, name: "NV0000_CTRL_CMD_GPU_GET_PCI_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000275, name: "NV0000_CTRL_CMD_GPU_GET_UUID_FROM_GPU_ID", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000279, name: "NV0000_CTRL_CMD_GPU_QUERY_DRAIN_STATE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x0000027b, name: "NV0000_CTRL_CMD_GPU_GET_MEMOP_ENABLE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000288, name: "NV0000_CTRL_CMD_GPU_GET_ACTIVE_DEVICE_IDS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000289, name: "NV0000_CTRL_CMD_GPU_ASYNC_ATTACH_ID", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000290, name: "NV0000_CTRL_CMD_GPU_WAIT_ATTACH_ID", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000301, name: "NV0000_CTRL_CMD_GSYNC_GET_ATTACHED_IDS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000a04, name: "NV0000_CTRL_CMD_SYNC_GPU_BOOST_GROUP_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000d01, name: "NV0000_CTRL_CMD_CLIENT_GET_ADDR_SPACE_TYPE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00000d04, name: "NV0000_CTRL_CMD_CLIENT_SET_INHERITED_SHARE_POLICY", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00003d05, name: "NV0000_CTRL_CMD_OS_UNIX_EXPORT_OBJECT_TO_FD", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00003d06, name: "NV0000_CTRL_CMD_OS_UNIX_IMPORT_OBJECT_FROM_FD", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00410110, name: "NV0041_CTRL_CMD_GET_SURFACE_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00730101, name: "NV0073_CTRL_CMD_SYSTEM_GET_CAPS_V2", origin: Origin::Empirical },
    ControlEntry { cmd: 0x00800201, name: "NV0080_CTRL_CMD_GPU_GET_CLASSLIST", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00800280, name: "NV0080_CTRL_CMD_GPU_GET_NUM_SUBDEVICES", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00800288, name: "NV0080_CTRL_CMD_GPU_QUERY_SW_STATE_PERSISTENCE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00800289, name: "NV0080_CTRL_CMD_GPU_GET_VIRTUALIZATION_MODE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x0080028b, name: "UNKNOWN_CONTROL_COMMAND_80028B", origin: Origin::Empirical },
    ControlEntry { cmd: 0x0080028e, name: "NV0080_CTRL_CMD_GPU_GET_VGX_CAPS", origin: Origin::Empirical },
    ControlEntry { cmd: 0x00800292, name: "NV0080_CTRL_CMD_GPU_GET_CLASSLIST_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00800294, name: "NV0080_CTRL_CMD_GPU_GET_BRAND_CAPS", origin: Origin::Empirical },
    ControlEntry { cmd: 0x00801102, name: "NV0080_CTRL_CMD_GR_GET_CAPS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00801104, name: "NV0080_CTRL_CMD_GR_GET_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00801109, name: "NV0080_CTRL_CMD_GR_GET_CAPS_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00801301, name: "NV0080_CTRL_CMD_FB_GET_CAPS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00801307, name: "NV0080_CTRL_CMD_FB_GET_CAPS_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00801401, name: "NV0080_CTRL_CMD_HOST_GET_CAPS", origin: Origin::Empirical },
    ControlEntry { cmd: 0x00801402, name: "NV0080_CTRL_CMD_HOST_GET_CAPS_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00801701, name: "NV0080_CTRL_CMD_FIFO_GET_CAPS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00801707, name: "NV0080_CTRL_CMD_FIFO_GET_ENGINE_CONTEXT_PROPERTIES", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x0080170d, name: "NV0080_CTRL_CMD_FIFO_GET_CHANNELLIST", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00801713, name: "NV0080_CTRL_CMD_FIFO_GET_CAPS_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00801806, name: "NV0080_CTRL_CMD_DMA_ADV_SCHED_GET_VA_CAPS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x0080180d, name: "NV0080_CTRL_CMD_DMA_GET_CAPS", origin: Origin::Nvproxy },
    ControlEntry { cmd: crate::generated::ctrl::NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY, name: "NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY", origin: Origin::Mode2Rpc },
    ControlEntry { cmd: crate::versions::NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY, name: "NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY", origin: Origin::Mode2Rpc },
    ControlEntry { cmd: 0x00801909, name: "NV0080_CTRL_CMD_PERF_CUDA_LIMIT_SET_CONTROL", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00801b01, name: "NV0080_CTRL_CMD_MSENC_GET_CAPS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00801c02, name: "NV0080_CTRL_CMD_BSP_GET_CAPS_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00da0002, name: "NV_SEMAPHORE_SURFACE_CTRL_CMD_BIND_CHANNEL", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00de0001, name: "NV00DE_CTRL_CMD_REQUEST_DATA_POLL", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00f80103, name: "NV00F8_CTRL_CMD_ATTACH_MEM", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00fd0101, name: "NV00FD_CTRL_CMD_GET_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00fd0102, name: "NV00FD_CTRL_CMD_ATTACH_MEM", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00fd0104, name: "NV00FD_CTRL_CMD_ATTACH_GPU", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x00fd0105, name: "NV00FD_CTRL_CMD_DETACH_MEM", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800102, name: "NV2080_CTRL_CMD_GPU_GET_INFO_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800110, name: "NV2080_CTRL_CMD_GPU_GET_NAME_STRING", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800111, name: "NV2080_CTRL_CMD_GPU_GET_SHORT_NAME_STRING", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800119, name: "NV2080_CTRL_CMD_GPU_GET_SIMULATION_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800123, name: "NV2080_CTRL_CMD_GPU_GET_ENGINES", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080012b, name: "NV2080_CTRL_CMD_GPU_PROMOTE_CTX", origin: Origin::Mode2Rpc },
    ControlEntry { cmd: 0x2080012f, name: "NV2080_CTRL_CMD_GPU_QUERY_ECC_STATUS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800131, name: "NV2080_CTRL_CMD_GPU_QUERY_COMPUTE_MODE_RULES", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800133, name: "NV2080_CTRL_CMD_GPU_QUERY_ECC_CONFIGURATION", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080013f, name: "NV2080_CTRL_CMD_GPU_GET_OEM_BOARD_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800142, name: "NV2080_CTRL_CMD_GPU_GET_ID", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800145, name: "NV2080_CTRL_CMD_GPU_ACQUIRE_COMPUTE_MODE_RESERVATION", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800146, name: "NV2080_CTRL_CMD_GPU_RELEASE_COMPUTE_MODE_RESERVATION", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800147, name: "NV2080_CTRL_CMD_GPU_GET_ENGINE_PARTNERLIST", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080014a, name: "NV2080_CTRL_CMD_GPU_GET_GID_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080014b, name: "NV2080_CTRL_CMD_GPU_GET_INFOROM_OBJECT_VERSION", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800156, name: "NV2080_CTRL_CMD_GPU_GET_INFOROM_IMAGE_VERSION", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800157, name: "NV2080_CTRL_CMD_GPU_QUERY_INFOROM_ECC_SUPPORT", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080016c, name: "NV2080_CTRL_CMD_GPU_GET_ENCODER_CAPACITY", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800170, name: "NV2080_CTRL_CMD_GPU_GET_ENGINES_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080018b, name: "NV2080_CTRL_CMD_GPU_GET_ACTIVE_PARTITION_IDS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080018d, name: "NV2080_CTRL_CMD_GPU_GET_PIDS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080018e, name: "NV2080_CTRL_CMD_GPU_GET_PID_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800195, name: "NV2080_CTRL_CMD_GPU_GET_COMPUTE_POLICY_CONFIG", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x208001a3, name: "NV2080_CTRL_CMD_GET_GPU_FABRIC_PROBE_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800301, name: "NV2080_CTRL_CMD_EVENT_SET_NOTIFICATION", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800403, name: "NV2080_CTRL_CMD_TIMER_GET_TIME", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800406, name: "NV2080_CTRL_CMD_TIMER_GET_GPU_CPU_TIME_CORRELATION_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800407, name: "NV2080_CTRL_CMD_TIMER_SET_GR_TICK_FREQ", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20800802, name: "NV2080_CTRL_CMD_BIOS_GET_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: crate::versions::NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER, name: "NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER", origin: Origin::Mode2Rpc },
    ControlEntry { cmd: 0x2080110b, name: "NV2080_CTRL_CMD_FIFO_DISABLE_CHANNELS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801201, name: "NV2080_CTRL_CMD_GR_GET_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801206, name: "NV2080_CTRL_CMD_GR_GET_ZCULL_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801208, name: "NV2080_CTRL_CMD_GR_CTXSW_ZCULL_BIND", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801210, name: "NV2080_CTRL_CMD_GR_SET_CTXSW_PREEMPTION_MODE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801218, name: "NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_SIZE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801219, name: "NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_INFO", origin: Origin::Mode2Rpc },
    ControlEntry { cmd: 0x2080121b, name: "NV2080_CTRL_CMD_GR_GET_GLOBAL_SM_ORDER", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801227, name: "NV2080_CTRL_CMD_GR_GET_CAPS_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080122a, name: "NV2080_CTRL_CMD_GR_GET_GPC_MASK", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080122b, name: "NV2080_CTRL_CMD_GR_GET_TPC_MASK", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801230, name: "NV2080_CTRL_CMD_GR_GET_SM_ISSUE_RATE_MODIFIER", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801301, name: "NV2080_CTRL_CMD_FB_GET_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801303, name: "NV2080_CTRL_CMD_FB_GET_INFO_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801315, name: "NV2080_CTRL_CMD_FB_GET_GPU_CACHE_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801320, name: "NV2080_CTRL_CMD_FB_GET_FB_REGION_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801352, name: "NV2080_CTRL_CMD_FB_GET_SEMAPHORE_SURFACE_LAYOUT", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801701, name: "NV2080_CTRL_CMD_MC_GET_ARCH_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801702, name: "NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801801, name: "NV2080_CTRL_CMD_BUS_GET_PCI_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801802, name: "NV2080_CTRL_CMD_BUS_GET_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801803, name: "NV2080_CTRL_CMD_BUS_GET_PCI_BAR_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801823, name: "NV2080_CTRL_CMD_BUS_GET_INFO_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080182a, name: "NV2080_CTRL_CMD_BUS_GET_PCIE_SUPPORTED_GPU_ATOMICS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080182b, name: "NV2080_CTRL_CMD_BUS_GET_C2C_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080200a, name: "NV2080_CTRL_CMD_PERF_BOOST", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20802068, name: "NV2080_CTRL_CMD_PERF_GET_CURRENT_PSTATE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20802209, name: "NV2080_CTRL_CMD_RC_GET_WATCHDOG_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x2080220c, name: "NV2080_CTRL_CMD_RC_RELEASE_WATCHDOG_REQUESTS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20802210, name: "NV2080_CTRL_CMD_RC_SOFT_DISABLE_WATCHDOG", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20802a02, name: "NV2080_CTRL_CMD_CE_GET_CE_PCE_MASK", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20802a03, name: "NV2080_CTRL_CMD_CE_GET_CAPS_V2", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20802a0a, name: "NV2080_CTRL_CMD_CE_GET_ALL_CAPS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20803001, name: "NV2080_CTRL_CMD_NVLINK_GET_NVLINK_CAPS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20803002, name: "NV2080_CTRL_CMD_NVLINK_GET_NVLINK_STATUS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20803125, name: "NV2080_CTRL_CMD_FLCN_GET_CTX_BUFFER_SIZE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20803601, name: "NV2080_CTRL_CMD_GSP_GET_FEATURES", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20803801, name: "NV2080_CTRL_CMD_GRMGR_GET_GR_FS_INFO", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20803d07, name: "NV2080_CTRL_CMD_OS_UNIX_VIDMEM_PERSISTENCE_STATUS", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x208f1105, name: "NV208F_CTRL_CMD_GPU_VERIFY_INFOROM", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x503c0102, name: "NV503C_CTRL_CMD_REGISTER_VA_SPACE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x503c0104, name: "NV503C_CTRL_CMD_REGISTER_VIDMEM", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x503c0105, name: "NV503C_CTRL_CMD_UNREGISTER_VIDMEM", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x906f0101, name: "NV906F_CTRL_GET_CLASS_ENGINEID", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x906f0102, name: "NV906F_CTRL_CMD_RESET_CHANNEL", origin: Origin::Nvproxy },
    // ★★★★★ **w288 TIER 2 — the ONLY control that carries a fault's ADDRESS.** The error
    // notifier has `status`/`info32`/`info16` and no address at all, so *"the guest observed
    // THE SAME FAULT, by identity"* is unanswerable without this id. ⊘ `Mode2Rpc`, not
    // `Nvproxy`: it reaches us only because Mode 2's transport is GSP RPC — it is
    // `ROUTE_TO_PHYSICAL`, so on a GSP client the guest RPCs it to the GSP, which is us.
    // ⚠ Admitting it here is NOT serving it (`admitted_and_served_are_different_gates`);
    // the arm that serves it is `kayfabe_rmrpc`'s `OBJECT_CONTROLS` entry, and the two are
    // held in lockstep by `tests/tests/admitted_is_served.rs`.
    ControlEntry { cmd: crate::submit::NV906F_CTRL_CMD_GET_MMU_FAULT_INFO, name: "NV906F_CTRL_CMD_GET_MMU_FAULT_INFO", origin: Origin::Mode2Rpc },
    ControlEntry { cmd: 0x90960101, name: "NV9096_CTRL_CMD_SET_ZBC_COLOR_CLEAR", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x90960106, name: "NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE_SIZE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x90960107, name: "NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE_ENTRY", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x90e60102, name: "NV90E6_CTRL_CMD_MASTER_GET_VIRTUAL_FUNCTION_ERROR_CONT_INTR_MASK", origin: Origin::Nvproxy },
    ControlEntry { cmd: crate::versions::NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES, name: "NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES", origin: Origin::Mode2Rpc },
    ControlEntry { cmd: 0xa06c0101, name: "NVA06C_CTRL_CMD_GPFIFO_SCHEDULE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0xa06c0103, name: "NVA06C_CTRL_CMD_SET_TIMESLICE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0xa06c0105, name: "NVA06C_CTRL_CMD_PREEMPT", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0xa06f0103, name: "NVA06F_CTRL_CMD_GPFIFO_SCHEDULE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0xa06f0104, name: "NVA06F_CTRL_CMD_BIND", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0xc36f0108, name: "NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0xc36f010a, name: "NVC36F_CTRL_CMD_GPFIFO_SET_WORK_SUBMIT_TOKEN_NOTIF_INDEX", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0xc56f010b, name: "NVC56F_CTRL_CMD_GET_KMB", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0xcb330101, name: "NV_CONF_COMPUTE_CTRL_CMD_SYSTEM_GET_CAPABILITIES", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0xcb330104, name: "NV_CONF_COMPUTE_CTRL_CMD_SYSTEM_GET_GPUS_STATE", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0xcb33010b, name: "NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_NUM_SECURE_CHANNELS", origin: Origin::Nvproxy },
];

pub(crate) static CLASSES_SHARED: &[ClassEntry] = &[
    ClassEntry {
        class: 0x00000000,
        name: "NV01_ROOT",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00000001,
        name: "NV01_ROOT_NON_PRIV",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00000002,
        name: "NV01_CONTEXT_DMA",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00000005,
        name: "NV01_EVENT",
        origin: Origin::Empirical,
    },
    ClassEntry {
        class: 0x0000003e,
        name: "NV01_MEMORY_SYSTEM",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00000040,
        name: "NV01_MEMORY_LOCAL_USER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00000041,
        name: "NV01_ROOT_CLIENT",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00000070,
        name: "NV01_MEMORY_VIRTUAL",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00000073,
        name: "NV04_DISPLAY_COMMON",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00000079,
        name: "NV01_EVENT_OS_EVENT",
        origin: Origin::Nvproxy,
    },
    // ★★★ `Origin::Mode2Rpc`, and it is the FIRST class row to carry it — the same
    // argument the six control rows make, one axis over.
    //
    // nvproxy does not have this class, and that is not an omission on its part: it gates
    // the **userspace** `/dev/nvidiactl` alloc surface, and it explicitly rewrites the
    // one neighbouring id it does see (`NV01_EVENT` -> `NV01_EVENT_OS_EVENT`,
    // `gvisor/pkg/sentry/devices/nvproxy/frontend.go:1139-1141`); its 575 map lists
    // `NV01_EVENT_OS_EVENT` (`0x79`) and nothing else in the family
    // (`version.go:412`, `:723`). `NV01_EVENT_KERNEL_CALLBACK_EX` is allocated by the
    // guest's own KERNEL RM during `RmInitAdapter`, so as an ioctl it never crosses the
    // boundary nvproxy is gating at all. It reaches us only because in Mode 2 the
    // transport is GSP RPC and we are the GSP.
    //
    // ★ `[measured]` — the 2026-08-01 boot (`docs/design/boot_measured_2026_08_01.md`
    // §3): `hClass=0x0000007e` is the fourth and last class `rpcRmApiAlloc_GSP` asks for
    // before `RmInitAdapter failed! (0x24:0x40:1220)`. It is on this list because a boot
    // sent it, not because a header names it.
    //
    // ⊘ What admitting it does NOT admit: its `NV0005_ALLOC_PARAMETERS` carries an
    // `NvP64 data` guest-kernel callback pointer, and
    // `DriverAbiTable::alloc_params` answers `AllocParams::NoDeclaredFacts` for it — the
    // params are never decoded, so the widest thing a hostile one can be is bytes nobody
    // reads. The object reaches the model as an edge and nothing else.
    ClassEntry {
        class: 0x0000007e,
        name: "NV01_EVENT_KERNEL_CALLBACK_EX",
        origin: Origin::Mode2Rpc,
    },
    ClassEntry {
        class: 0x00000080,
        name: "NV01_DEVICE_0",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000000da,
        name: "NV_SEMAPHORE_SURFACE",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000000de,
        name: "RM_USER_SHARED_DATA",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000000e0,
        name: "NV_MEMORY_EXPORT",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000000f1,
        name: "NV_IMEX_SESSION",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000000f8,
        name: "NV_MEMORY_FABRIC",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000000fb,
        name: "NV_MEMORY_FABRIC_IMPORTED_REF",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000000fd,
        name: "NV_MEMORY_MULTICAST_FABRIC",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000000fe,
        name: "NV_MEMORY_MAPPER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00002080,
        name: "NV20_SUBDEVICE_0",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00002081,
        name: "NV2081_BINAPI",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000208f,
        name: "NV20_SUBDEVICE_DIAG",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000503b,
        name: "NV50_P2P",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000503c,
        name: "NV50_THIRD_PARTY_P2P",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000050a0,
        name: "NV50_MEMORY_VIRTUAL",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000902d,
        name: "FERMI_TWOD_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00009067,
        name: "FERMI_CONTEXT_SHARE_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00009072,
        name: "GF100_DISP_SW",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x00009096,
        name: "GF100_ZBC_CLEAR",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000090cc,
        name: "GF100_PROFILER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000090e6,
        name: "GF100_SUBDEVICE_MASTER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000090e7,
        name: "GF100_SUBDEVICE_INFOROM",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x000090f1,
        name: "FERMI_VASPACE_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000a06c,
        name: "KEPLER_CHANNEL_GROUP_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000a0bc,
        name: "NVENC_SW_SESSION",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000a140,
        name: "KEPLER_INLINE_TO_MEMORY_B",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000b2cc,
        name: "MAXWELL_PROFILER_DEVICE",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000b8b0,
        name: "NVB8B0_VIDEO_DECODER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000b8d1,
        name: "NVB8D1_VIDEO_NVJPG",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000b8fa,
        name: "NVB8FA_VIDEO_OFA",
        origin: Origin::Nvproxy,
    },
    // ★★★ `GP100_UVM_SW` (`0xc076`) — the LAST step of UVM's channel allocation, and the
    // one that was destroying every UVM channel.
    //
    // `[measured 2026-08-09, boot s22_f4f3865]` four of these are refused in the `cuInit`
    // window, one per UVM channel, and they are the last thing the guest asks for before
    // it tears the adapter down — `hClient=0xc1d0000a; hParent=0xcaf000{12,1d,28,33};
    // hClass=0x0000c076; paramsSize=0x00000000; status=0x00000056`. The refusal is fatal
    // and has no forgiving caller: `channelAllocate` does `goto cleanup_free_controlpage`
    // on it (`ogkm-580: src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:6110-6122`), which fails
    // `nvGpuOpsChannelAllocate`, `uvm_channel_manager_create` and `UVM_REGISTER_GPU`.
    //
    // ★ `Origin::Mode2Rpc` is exact, not a fallback. `grep -rn 0xc076 gvisor/` is EMPTY:
    // the class is allocated by the guest's own KERNEL RM inside `nvGpuOpsChannelAllocate`
    // (`RS_FLAGS_ALLOC_PRIVILEGED`, `Parents = RS_LIST(classId(KernelChannel))`,
    // `ogkm-580: resource_list.h:1535-1544`), so as an ioctl it never crosses the boundary
    // nvproxy gates. It reaches us only because in Mode 2 the transport is GSP RPC.
    // That origin's obligation — *"a row with this origin has a consumer in the table that
    // decides its params shape"* — is discharged by the `alloc_params` arm added with it.
    //
    // ⊘ What admitting it does NOT admit: the object's only in-band use is
    // `uvm_hal_pascal_host_init` -> `NV_PUSH_1U(C076, SET_OBJECT, GP100_UVM_SW)`
    // (`ogkm-580: kernel-open/nvidia-uvm/uvm_pascal_host.c:314-318`), reserving a
    // subchannel for `FAULT_CANCEL_A`. The cancel methods are pushed only from UVM's
    // fault-service path, which this port never enters because it raises no fault. A guest
    // whose faults we DO start delivering is the case this row does not cover.
    ClassEntry {
        class: 0x0000c076,
        name: "GP100_UVM_SW",
        origin: Origin::Mode2Rpc,
    },
    ClassEntry {
        class: 0x0000c361,
        name: "VOLTA_USERMODE_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c461,
        name: "TURING_USERMODE_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c46f,
        name: "TURING_CHANNEL_GPFIFO_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c4b0,
        name: "NVC4B0_VIDEO_DECODER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c4b7,
        name: "NVC4B7_VIDEO_ENCODER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c4d1,
        name: "NVC4D1_VIDEO_NVJPG",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c56f,
        name: "AMPERE_CHANNEL_GPFIFO_A",
        origin: Origin::Nvproxy,
    },
    // ★★ `UVM_CHANNEL_RETAINER` — UVM's reference on a channel it did not create. Admitted
    // because serving it is BOOKKEEPING rather than a forgery: both of its two parameters
    // are `[IN]` handles and `uvmchanrtnrConstruct_IMPL` writes nothing back, so echoing the
    // request body invents no value the guest will act on. ⊘ Never forwarded to the host:
    // the channel it retains is one in OUR object model, so the handle pair it carries names
    // nothing the host knows. (A host refusal for this class is reported by the C artifact;
    // ⊘ no run of it is cited here, so it is not leaned on — the object-model argument stands
    // alone.) See `generated::classes::UVM_CHANNEL_RETAINER` for the full citation.
    //
    // ⚠ Admitting it is NOT expected to move the progress fraction: the wall this port is at
    // is a completion, not a control-plane refusal.
    ClassEntry {
        class: 0x0000c574,
        name: "UVM_CHANNEL_RETAINER",
        origin: Origin::Mode2Rpc,
    },
    ClassEntry {
        class: 0x0000c597,
        name: "TURING_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c5b5,
        name: "TURING_DMA_COPY_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c5c0,
        name: "TURING_COMPUTE_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c661,
        name: "HOPPER_USERMODE_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c697,
        name: "AMPERE_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c6b0,
        name: "NVC6B0_VIDEO_DECODER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c6b5,
        name: "AMPERE_DMA_COPY_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c6c0,
        name: "AMPERE_COMPUTE_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c6fa,
        name: "NVC6FA_VIDEO_OFA",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c797,
        name: "AMPERE_B",
        origin: Origin::Empirical,
    },
    ClassEntry {
        class: 0x0000c7b0,
        name: "NVC7B0_VIDEO_DECODER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c7b5,
        name: "AMPERE_DMA_COPY_B",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c7b7,
        name: "NVC7B7_VIDEO_ENCODER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c7c0,
        name: "AMPERE_COMPUTE_B",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c7fa,
        name: "NVC7FA_VIDEO_OFA",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c86f,
        name: "HOPPER_CHANNEL_GPFIFO_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c8b5,
        name: "HOPPER_DMA_COPY_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c997,
        name: "ADA_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c9b0,
        name: "NVC9B0_VIDEO_DECODER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c9b7,
        name: "NVC9B7_VIDEO_ENCODER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c9c0,
        name: "ADA_COMPUTE_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c9d1,
        name: "NVC9D1_VIDEO_NVJPG",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c9fa,
        name: "NVC9FA_VIDEO_OFA",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cb33,
        name: "NV_CONFIDENTIAL_COMPUTE",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cb97,
        name: "HOPPER_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cba2,
        name: "HOPPER_SEC2_WORK_LAUNCH_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cbc0,
        name: "HOPPER_COMPUTE_A",
        origin: Origin::Nvproxy,
    },
];

pub(crate) static CLASSES_FROM_560_28_03: &[ClassEntry] = &[
    ClassEntry {
        class: 0x0000c96f,
        name: "BLACKWELL_CHANNEL_GPFIFO_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000c9b5,
        name: "BLACKWELL_DMA_COPY_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cd40,
        name: "BLACKWELL_INLINE_TO_MEMORY_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cd97,
        name: "BLACKWELL_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cdb0,
        name: "NVCDB0_VIDEO_DECODER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cdc0,
        name: "BLACKWELL_COMPUTE_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cdd1,
        name: "NVCDD1_VIDEO_NVJPG",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cdfa,
        name: "NVCDFA_VIDEO_OFA",
        origin: Origin::Nvproxy,
    },
];

pub(crate) static CLASSES_FROM_570_86_15: &[ClassEntry] = &[
    ClassEntry {
        class: 0x0000c761,
        name: "BLACKWELL_USERMODE_A",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000ca6f,
        name: "BLACKWELL_CHANNEL_GPFIFO_B",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cab5,
        name: "BLACKWELL_DMA_COPY_B",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000ce97,
        name: "BLACKWELL_B",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cec0,
        name: "BLACKWELL_COMPUTE_B",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000cfb7,
        name: "NVCFB7_VIDEO_ENCODER",
        origin: Origin::Nvproxy,
    },
];

pub(crate) static CLASSES_FROM_580_65_06: &[ClassEntry] = &[
    ClassEntry {
        class: 0x0000ceb7,
        name: "NVCEB7_VIDEO_ENCODER",
        origin: Origin::Nvproxy,
    },
    ClassEntry {
        class: 0x0000d1b7,
        name: "NVD1B7_VIDEO_ENCODER",
        origin: Origin::Nvproxy,
    },
];

/// ★ The nine rows of the C's control allowlist that the two rule-based passthroughs
/// **already cover**, and which are therefore NOT carried as table rows here.
///
/// Each has either the [`RM_GSS_LEGACY_MASK`] bit set or sits in the
/// [`NV2081_BINAPI_CLASS`], so [`CapabilityTable::control`] answers `GssLegacyRule` /
/// `BinApiRule` for it whether or not a row exists — a row that can never change an
/// answer is not coverage, and none of the nine has a name in nvproxy's map or in either
/// vendored open-kernel tree, so none could be reviewed either.
///
/// Kept as data rather than deleted because *"a rule covers it"* is a claim with a
/// lifetime: `the_c_rows_dropped_as_rule_covered_are_still_permitted` re-checks all nine
/// against the live rules, so narrowing a rule turns nine silent new denials into a red
/// test.
pub static RULE_COVERED_C_ROWS: &[u32] = &[
    0x20808159, 0x20808162, 0x2080852e, 0x2080852f, 0x2080a0d1, 0x2080a612, 0x2080a618, 0x20810107,
    0x20810108,
];

/// Controls this port refuses **by name**, with a reason.
///
/// ★ Most rows here are already absent from the allowlist, so they change what the
/// refusal *says* — and therefore what a census can distinguish — rather than what a
/// guest can do. The C makes the same exclusions implicitly and says so in prose
/// (*"reg-ops/HWPM/debug/fabric/power fall out automatically"*); this is that sentence,
/// in a form a test can bite.
///
/// ★★★ **Three rows are an exception and really do narrow the surface**, and they are
/// called out here because the sentence above used to be unqualified and was true:
/// `0x83de0309`, `0x83de030c` and `0x83de0310` were on the **allowlist** (ported from
/// nvproxy, which permits them because gVisor forwards to a real GPU that implements
/// them). This port does not implement SM debugger trapping at all
/// ([`DeniedBecause::SmDebuggerTrapping`]), so permitting the controls was a promise it
/// could not keep. What a guest observes is unchanged — `BridgeRefusal::rpc_result` is
/// `NV_ERR_NOT_SUPPORTED` for every variant, and `GT200_DEBUGGER` had no
/// [`crate::versions::AllocParams`] row either, so an alloc already refused as
/// `UnmappedAllocClass` — but the refusal now names the mechanism instead of the
/// modelling gap.
///
/// Sorted by `id` — [`CapabilityTable::control`] binary-searches it.
pub(crate) static DENIED_CONTROLS: &[DeniedEntry] = &[
    DeniedEntry {
        id: 0x00e0_0102,
        name: "NV00E0_CTRL_CMD_IMPORT_MEM",
        why: DeniedBecause::FabricManagement,
    },
    DeniedEntry {
        id: 0x00f1_0003,
        name: "NV00F1_CTRL_CMD_DISABLE_IMPORTERS",
        why: DeniedBecause::FabricManagement,
    },
    DeniedEntry {
        id: 0x2080_0122,
        name: "NV2080_CTRL_CMD_GPU_EXEC_REG_OPS",
        why: DeniedBecause::RegisterAccess,
    },
    DeniedEntry {
        id: 0x2080_0177,
        name: "NV2080_CTRL_CMD_GPU_REPORT_NON_REPLAYABLE_FAULT",
        why: DeniedBecause::FaultMechanismNotModelled,
    },
    DeniedEntry {
        id: 0x2080_3083,
        name: "NV2080_CTRL_CMD_NVLINK_GET_PLATFORM_INFO",
        why: DeniedBecause::FabricManagement,
    },
    // ★★ The `NV83DE` block. The first three MOVED here from the allowlist; the last
    // three were already absent and are named so a census can tell a debugger attaching
    // from an unmodelled command. `0x83de0307` is the load-bearing one — see
    // `DeniedBecause::SmDebuggerTrapping`'s second half.
    DeniedEntry {
        id: 0x83de_0307,
        name: "NV83DE_CTRL_CMD_DEBUG_SET_MODE_MMU_DEBUG",
        why: DeniedBecause::SmDebuggerTrapping,
    },
    DeniedEntry {
        id: 0x83de_0309,
        name: "NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK",
        why: DeniedBecause::SmDebuggerTrapping,
    },
    DeniedEntry {
        id: 0x83de_030c,
        name: "NV83DE_CTRL_CMD_DEBUG_READ_ALL_SM_ERROR_STATES",
        why: DeniedBecause::SmDebuggerTrapping,
    },
    DeniedEntry {
        id: 0x83de_0310,
        name: "NV83DE_CTRL_CMD_DEBUG_CLEAR_ALL_SM_ERROR_STATES",
        why: DeniedBecause::SmDebuggerTrapping,
    },
    DeniedEntry {
        id: 0x83de_0317,
        name: "NV83DE_CTRL_CMD_DEBUG_SUSPEND_CONTEXT",
        why: DeniedBecause::SmDebuggerTrapping,
    },
    DeniedEntry {
        id: 0x83de_0318,
        name: "NV83DE_CTRL_CMD_DEBUG_RESUME_CONTEXT",
        why: DeniedBecause::SmDebuggerTrapping,
    },
    DeniedEntry {
        id: 0xb0cc_0105,
        name: "NVB0CC_CTRL_CMD_ALLOC_PMA_STREAM",
        why: DeniedBecause::PerformanceCounters,
    },
    DeniedEntry {
        id: 0xb0cc_010a,
        name: "NVB0CC_CTRL_CMD_EXEC_REG_OPS",
        why: DeniedBecause::RegisterAccess,
    },
];

/// Classes this port refuses **by name**, with a reason.
///
/// Both are classes nvproxy deliberately omits from its `allocationClass` map and the C
/// omitted with it (`C: nvkvm_fe_alloc_allowlist.h:8-11`, which names exactly these two
/// plus bare `NV01_EVENT` — and `NV01_EVENT` was later *added* on graphics evidence, so
/// it is on the allowlist above and not here).
pub(crate) static DENIED_CLASSES: &[DeniedEntry] = &[
    DeniedEntry {
        id: 0x0000_003f,
        name: "NV01_MEMORY_LOCAL_PRIVILEGED",
        why: DeniedBecause::PrivilegedMemory,
    },
    DeniedEntry {
        id: 0x0000_0071,
        name: "NV01_MEMORY_SYSTEM_OS_DESCRIPTOR",
        why: DeniedBecause::CallerMemoryDescriptor,
    },
    // ★★★ MOVED off the allowlist. nvproxy permits it because gVisor forwards to real
    // silicon; this port emulates the GPU, and there is no SM state behind the class.
    // Allowing the alloc and refusing every control would let a debugger *attach* and
    // then fail at first use — a worse shape than refusing the attach.
    DeniedEntry {
        id: 0x0000_402c,
        name: "NV40_I2C",
        why: DeniedBecause::NoPhysicalBoardBus,
    },
    DeniedEntry {
        id: 0x0000_83de,
        name: "GT200_DEBUGGER",
        why: DeniedBecause::SmDebuggerTrapping,
    },
];

// ═══ The per-boundary control blocks ═════════════════════════════════════════════════
//
// ★★★ Each block is a set of rows some boundaries have and others do not. A boundary
// declares the blocks it has; a boundary that must NOT have a row simply does not name
// its block. That absent name IS the removal — there is nothing else to it, and nothing
// to un-inherit.

/// Present at 550.54.04 and 550.90.07; **deleted** at 555.42.02
/// (`gvisor nvproxy: version.go:933` — `delete(abi.controlCmd,
/// nvgpu.NVC36F_CTRL_GET_CLASS_ENGINEID)`) and never re-added.
///
/// ★ The C's list is the 575-era set, so it never had this row and neither did this port
/// until task #122 — the same defect the DRAM-encryption pair has, in the other
/// direction: a command a 550 guest legitimately issues, refused at the only boundaries
/// that should permit it. Carried as [`Origin::Nvproxy`] because nvproxy's own base map
/// holds it under the compute capability (`gvisor nvproxy: version.go:360`).
pub(crate) static CONTROLS_UNTIL_555_42_02: &[ControlEntry] = &[ControlEntry {
    cmd: 0xc36f0101,
    name: "NVC36F_CTRL_GET_CLASS_ENGINEID",
    origin: Origin::Nvproxy,
}];

/// Added at 550.90.07 (`gvisor nvproxy: version.go:906`) and never removed, so every
/// boundary from there up names it.
pub(crate) static CONTROLS_FROM_550_90_07: &[ControlEntry] = &[ControlEntry {
    cmd: 0xcb33010c,
    name: "NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE",
    origin: Origin::Nvproxy,
}];

/// Added at 560.28.03 (`gvisor nvproxy: version.go:955`).
///
/// The two other controls nvproxy adds at that boundary are deliberately absent:
/// `NV2080_CTRL_CMD_NVLINK_GET_PLATFORM_INFO` is on [`DENIED_CONTROLS`] as fabric
/// management, and `NV2080_CTRL_CMD_BUS_GET_PCIE_CPL_ATOMICS_CAPS` is graphics-capability
/// and outside the C's compute filter.
pub(crate) static CONTROLS_FROM_560_28_03: &[ControlEntry] = &[ControlEntry {
    cmd: 0x00da0006,
    name: "NV_SEMAPHORE_SURFACE_CTRL_CMD_UNBIND_CHANNEL",
    origin: Origin::Nvproxy,
}];

/// ★★★ **The block 575.51.02 does not name.** Added at 570.86.15
/// (`gvisor nvproxy: version.go:1005-1006`) and deleted at 575.51.02
/// (`gvisor nvproxy: version.go:1039-1040`), so it belongs to exactly one boundary here.
///
/// `NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_PENDING_CONFIGURATION` (`0x20801355`) arrives
/// at the same boundary and is **not** here: it is graphics-capability, and the port
/// carries only what the C's compute filter admitted.
pub(crate) static CONTROLS_DRAM_ENCRYPTION_570: &[ControlEntry] = &[
    ControlEntry {
        cmd: 0x20801358,
        name: "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT",
        origin: Origin::Nvproxy,
    },
    ControlEntry {
        cmd: 0x20801359,
        name: "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS",
        origin: Origin::Nvproxy,
    },
];

/// ★★★ **The block that replaces it.** 575.51.02 re-adds the same two commands one
/// number lower and adds a third (`gvisor nvproxy: version.go:1041-1043`).
///
/// `0x20801358` is in **both** this block and [`CONTROLS_DRAM_ENCRYPTION_570`], under
/// **different NVIDIA names** — `..._STATUS_V575` here, `..._INFOROM_SUPPORT` there. That
/// is what an add-only table could not represent: not a missing row, but one command word
/// meaning two different things on two sides of a boundary.
pub(crate) static CONTROLS_FROM_575_51_02: &[ControlEntry] = &[
    ControlEntry {
        cmd: 0x20800513,
        name: "NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2",
        origin: Origin::Nvproxy,
    },
    ControlEntry {
        cmd: 0x20801357,
        name: "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT_V575",
        origin: Origin::Nvproxy,
    },
    ControlEntry {
        cmd: 0x20801358,
        name: "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS_V575",
        origin: Origin::Nvproxy,
    },
];

/// The floor every boundary stands on — see [`SharedCapabilities`].
pub static SHARED_CAPS: SharedCapabilities = SharedCapabilities {
    controls: CONTROLS_SHARED,
    classes: CLASSES_SHARED,
    denied_controls: DENIED_CONTROLS,
    denied_classes: DENIED_CLASSES,
};

/// 550.54.04 — the oldest supported boundary: the shared floor plus the one control
/// nvproxy still has here and deletes at 555.42.02.
pub static CAPS_550_54_04: CapabilityTable = CapabilityTable {
    shared: &SHARED_CAPS,
    own_controls: &[CONTROLS_UNTIL_555_42_02],
    own_classes: &[],
    note: "the C's ported set: nvproxy 575-ABI control map (compute-filtered) + its \
           575 class set MINUS the classes nvproxy adds after 550.54.04, + the six \
           Mode-2 GSP-RPC controls the ioctl boundary never saw, + \
           NVC36F_CTRL_GET_CLASS_ENGINEID, which nvproxy still has here \
           (version.go:360)",
};

/// 550.90.07 — `gvisor nvproxy: version.go:906`. Additive only.
pub static CAPS_550_90_07: CapabilityTable = CapabilityTable {
    shared: &SHARED_CAPS,
    own_controls: &[CONTROLS_UNTIL_555_42_02, CONTROLS_FROM_550_90_07],
    own_classes: &[],
    note: "v550_90_07 adds NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE \
           (version.go:906) and changes nothing else this port carries",
};

/// ★★★ 555.42.02 — a **purely SUBTRACTIVE** boundary, and one the old shape could not
/// have expressed at all.
///
/// It names [`CONTROLS_FROM_550_90_07`] and not [`CONTROLS_UNTIL_555_42_02`]. There is no
/// removal *operation* anywhere in this file: the block is simply not in the list.
pub static CAPS_555_42_02: CapabilityTable = CapabilityTable {
    shared: &SHARED_CAPS,
    own_controls: &[CONTROLS_FROM_550_90_07],
    own_classes: &[],
    note: "★ SUBTRACTIVE: v555_42_02 deletes NVC36F_CTRL_GET_CLASS_ENGINEID \
           (version.go:933) and adds nothing this port carries",
};

/// 560.28.03 — `gvisor nvproxy: version.go:945-977`.
pub static CAPS_560_28_03: CapabilityTable = CapabilityTable {
    shared: &SHARED_CAPS,
    own_controls: &[CONTROLS_FROM_550_90_07, CONTROLS_FROM_560_28_03],
    own_classes: &[CLASSES_FROM_560_28_03],
    note: "v560_28_03 adds NVCDB0/NVCDD1/NVCDFA and the first Blackwell channel, \
           copy, graphics, compute and inline-to-memory classes, plus \
           NV_SEMAPHORE_SURFACE_CTRL_CMD_UNBIND_CHANNEL",
};

/// 570.86.15 — `gvisor nvproxy: version.go:990-1027`.
pub static CAPS_570_86_15: CapabilityTable = CapabilityTable {
    shared: &SHARED_CAPS,
    own_controls: &[
        CONTROLS_FROM_550_90_07,
        CONTROLS_FROM_560_28_03,
        CONTROLS_DRAM_ENCRYPTION_570,
    ],
    own_classes: &[CLASSES_FROM_560_28_03, CLASSES_FROM_570_86_15],
    note: "v570_86_15 adds the Blackwell B channel/copy/graphics/compute pair, \
           BLACKWELL_USERMODE_A, NVCFB7_VIDEO_ENCODER and the two DRAM-encryption \
           controls at their PRE-575 numbers",
};

/// ★★★ 575.51.02 — the boundary that motivated task #122: it **replaces** two controls
/// rather than adding any (`gvisor nvproxy: version.go:1036-1053`).
///
/// The replacement is one line. This table names [`CONTROLS_FROM_575_51_02`] and does
/// **not** name [`CONTROLS_DRAM_ENCRYPTION_570`]; read the two `own_controls` lists here
/// and at [`CAPS_570_86_15`] side by side and the whole boundary is visible without
/// resolving anything.
pub static CAPS_575_51_02: CapabilityTable = CapabilityTable {
    shared: &SHARED_CAPS,
    own_controls: &[
        CONTROLS_FROM_550_90_07,
        CONTROLS_FROM_560_28_03,
        CONTROLS_FROM_575_51_02,
    ],
    own_classes: &[CLASSES_FROM_560_28_03, CLASSES_FROM_570_86_15],
    note: "★ REPLACES: v575_51_02 deletes the two DRAM-encryption controls and re-adds \
           them one number lower, and adds NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2 \
           (version.go:1036-1053). No allocation class changes",
};

/// 580.65.06 — `gvisor nvproxy: version.go:1057-1078`.
///
/// ★ This boundary is the one that makes the version seam **observable**: the C's list
/// is the 575 set, so `NVCEB7`/`NVD1B7` are two classes a 580 guest may allocate and a
/// 580.65.05 guest may not. The two controls nvproxy also adds here
/// (`GPU_GET_SKYLINE_INFO`, `ECC_GET_REPAIR_STATUS`) are `CapGraphics`-only and are
/// therefore **not** carried, exactly as the C's compute filter excluded every other
/// graphics-only row.
pub static CAPS_580_65_06: CapabilityTable = CapabilityTable {
    shared: &SHARED_CAPS,
    own_controls: &[
        CONTROLS_FROM_550_90_07,
        CONTROLS_FROM_560_28_03,
        CONTROLS_FROM_575_51_02,
    ],
    own_classes: &[
        CLASSES_FROM_560_28_03,
        CLASSES_FROM_570_86_15,
        CLASSES_FROM_580_65_06,
    ],
    note: "v580_65_06 adds NVCEB7_VIDEO_ENCODER and NVD1B7_VIDEO_ENCODER",
};

/// 610.43.02 — the 580.65.06 surface, **declared again rather than inherited**.
///
/// nvproxy changes no control and no class this port carries between 580.65.06 and here,
/// so the content is 580's; it is spelled out so a reader of the 610 row sees 610's whole
/// surface without following a pointer, and so the day 610 diverges the edit is local to
/// this table. The wire layouts *do* move at this boundary — that is
/// [`crate::versions::GspElementWire`]'s business, not this module's.
pub static CAPS_610_43_02: CapabilityTable = CapabilityTable {
    shared: &SHARED_CAPS,
    own_controls: &[
        CONTROLS_FROM_550_90_07,
        CONTROLS_FROM_560_28_03,
        CONTROLS_FROM_575_51_02,
    ],
    own_classes: &[
        CLASSES_FROM_560_28_03,
        CLASSES_FROM_570_86_15,
        CLASSES_FROM_580_65_06,
    ],
    note: "the 580.65.06 capability surface, declared again rather than inherited: \
           nvproxy changes no control and no class this port carries between there and \
           610.43.02 — only the GSP wire layouts move",
};

/// Every boundary in this module, ascending — the **universe** the structural tests are
/// quantified over.
///
/// ★★ Derived-from, not parallel-to: `the_boundary_list_is_the_whole_universe` checks
/// this against [`crate::versions::TABLES`], so a driver row added there without a
/// boundary here turns the suite red instead of quietly shrinking every gate below.
/// (`gates_quantified_over_a_list`: shortening a list weakens a gate with zero red
/// tests.)
pub static ALL_BOUNDARIES: &[&CapabilityTable] = &[
    &CAPS_550_54_04,
    &CAPS_550_90_07,
    &CAPS_555_42_02,
    &CAPS_560_28_03,
    &CAPS_570_86_15,
    &CAPS_575_51_02,
    &CAPS_580_65_06,
    &CAPS_610_43_02,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DriverVersion;
    use crate::versions::{AllocParams, ControlParams, table_for};
    use std::collections::BTreeSet;

    fn at(major: u16, minor: u16, patch: u16) -> &'static CapabilityTable {
        table_for(DriverVersion {
            major,
            minor,
            patch,
        })
        .expect("in range")
        .capabilities()
    }

    /// One boundary's expected **resolved** surface: a label, its driver version, the
    /// resolved control and class counts, and the NVIDIA names of the controls it has
    /// beyond the shared base.
    type ResolvedExpectation = (
        &'static str,
        (u16, u16, u16),
        usize,
        usize,
        &'static [&'static str],
    );

    /// One boundary's expected answer for a single command word: the driver version, and
    /// the NVIDIA name it must be permitted under — `None` meaning refused.
    type PerVersionAnswer = ((u16, u16, u16), Option<&'static str>);

    /// The bench's own driver — the surface every other test here reasons about.
    fn bench() -> &'static CapabilityTable {
        at(580, 159, 4)
    }

    // ── Structure: the properties `binary_search` and the deny-first order need ──────

    /// Every block a boundary names is sorted and duplicate-free, in every boundary.
    ///
    /// ★ Not decoration: [`CapabilityTable::control`] binary-searches **within a block**,
    /// so an unsorted slice does not fail loudly — it silently *misses* rows, i.e.
    /// quietly turns permitted commands into denials. This is what makes a hand-edited
    /// insertion in the wrong place a red test instead of a shrug.
    #[test]
    fn every_boundarys_rows_are_sorted_and_unique() {
        for t in ALL_BOUNDARIES {
            for block in t.own_controls {
                assert!(
                    block.windows(2).all(|w| w[0].cmd < w[1].cmd),
                    "own controls unsorted/duplicated at {:?}",
                    t.note
                );
            }
            for block in t.own_classes {
                assert!(
                    block.windows(2).all(|w| w[0].class < w[1].class),
                    "own classes unsorted/duplicated at {:?}",
                    t.note
                );
            }
            assert!(
                t.shared.controls.windows(2).all(|w| w[0].cmd < w[1].cmd),
                "shared controls unsorted/duplicated"
            );
            assert!(
                t.shared.classes.windows(2).all(|w| w[0].class < w[1].class),
                "shared classes unsorted/duplicated"
            );
            assert!(
                t.shared
                    .denied_controls
                    .windows(2)
                    .all(|w| w[0].id < w[1].id),
                "denied controls unsorted/duplicated"
            );
            assert!(
                t.shared
                    .denied_classes
                    .windows(2)
                    .all(|w| w[0].id < w[1].id),
                "denied classes unsorted/duplicated"
            );
        }
    }

    // ── The shape itself: shared base + per-boundary blocks, depth two ───────────────

    /// ★★★ **The universe every structural test here is quantified over is DERIVED.**
    ///
    /// [`ALL_BOUNDARIES`] must be exactly the set of tables [`crate::versions::TABLES`]
    /// points at. A driver row added there whose `caps` is a table missing from here
    /// would sit outside every gate below — a smaller universe is a smaller true
    /// statement, and shortening a list weakens a gate with zero red tests
    /// (`gates_quantified_over_a_list`).
    ///
    /// ★ Identity is by `note`, not by address: a raw pointer may not be *held* outside
    /// a `*_unsafe.rs` file (the host-pointer gate, `l1_os_shell.md` §9.3), and the notes
    /// are asserted distinct here so comparing them IS comparing identity.
    #[test]
    fn the_boundary_list_is_the_whole_universe() {
        let declared: BTreeSet<&str> = ALL_BOUNDARIES.iter().map(|t| t.note).collect();
        assert_eq!(
            declared.len(),
            ALL_BOUNDARIES.len(),
            "two boundaries share a note, so `note` is not an identity here and the \
             comparison below would pass while conflating them"
        );
        let from_tables: BTreeSet<&str> = crate::versions::TABLES
            .iter()
            .map(|t| t.capabilities().note)
            .collect();
        assert_eq!(
            from_tables, declared,
            "ALL_BOUNDARIES and the tables TABLES points at are not the same set — a \
             boundary outside this list sits outside every gate in this module"
        );
        // Eight boundaries, eight driver rows, and the rows outnumber nothing: a
        // `TABLES` that grew a row without a boundary would already have failed above,
        // but the literal is what says how big the universe is meant to be.
        assert_eq!(ALL_BOUNDARIES.len(), 8);
        assert_eq!(crate::versions::TABLES.len(), 8);
    }

    /// ★★★ **The strip is real in both directions**, which is the property that makes a
    /// removal expressible.
    ///
    /// - Nothing in [`SHARED_CAPS`] may be absent from any boundary — trivially true
    ///   given the lookup, and asserted anyway so a future `own`-shadows-`shared`
    ///   mistake is caught here rather than by a guest.
    /// - **Nothing that every boundary owns may stay outside it.** A row in *all eight*
    ///   `own` sets is a row that belongs in the shared base; leaving it in eight blocks
    ///   is eight places to forget, which is the failure mode this shape exists to avoid.
    #[test]
    fn the_shared_base_holds_only_what_every_boundary_shares() {
        for t in ALL_BOUNDARIES {
            for e in SHARED_CAPS.controls {
                assert!(
                    t.control(ControlCmd(e.cmd)).is_permitted(),
                    "{:#010x} is shared but refused at {:?}",
                    e.cmd,
                    t.note
                );
            }
            for e in SHARED_CAPS.classes {
                assert!(
                    t.alloc_class(ClassId(e.class)).is_permitted(),
                    "{:#010x} is shared but refused at {:?}",
                    e.class,
                    t.note
                );
            }
        }
        // The other direction: no id is in every boundary's OWN rows.
        let own_ctl = |t: &'static CapabilityTable| -> BTreeSet<u32> {
            t.own_controls
                .iter()
                .flat_map(|b| b.iter())
                .map(|e| e.cmd)
                .collect()
        };
        let own_cls = |t: &'static CapabilityTable| -> BTreeSet<u32> {
            t.own_classes
                .iter()
                .flat_map(|b| b.iter())
                .map(|e| e.class)
                .collect()
        };
        let mut ctl = own_ctl(ALL_BOUNDARIES[0]);
        let mut cls = own_cls(ALL_BOUNDARIES[0]);
        for t in &ALL_BOUNDARIES[1..] {
            ctl = ctl.intersection(&own_ctl(t)).copied().collect();
            cls = cls.intersection(&own_cls(t)).copied().collect();
        }
        assert!(
            ctl.is_empty(),
            "control(s) {ctl:#010x?} are owned by EVERY boundary — they belong in \
             SHARED_CAPS, not in eight blocks"
        );
        assert!(
            cls.is_empty(),
            "class(es) {cls:#010x?} are owned by EVERY boundary — they belong in \
             SHARED_CAPS"
        );
        // Non-vacuity: the intersection above is over a set that is not empty to begin
        // with, or "no common row" would be true for an uninteresting reason.
        assert!(!own_ctl(ALL_BOUNDARIES[0]).is_empty());
        assert!(!own_cls(ALL_BOUNDARIES[7]).is_empty());
    }

    /// ★★ A boundary's own blocks are disjoint from the shared base and from each other.
    ///
    /// If they were not, [`CapabilityTable::control`]'s "own first" order would start
    /// deciding which of two rows for the same command word answers — and the answer
    /// carries the NVIDIA **name**, so a shadowed row is a gate that reports the wrong
    /// command by name while permitting the right number.
    #[test]
    fn no_boundary_repeats_a_shared_or_duplicated_row() {
        let shared_ctl: BTreeSet<u32> = SHARED_CAPS.controls.iter().map(|e| e.cmd).collect();
        let shared_cls: BTreeSet<u32> = SHARED_CAPS.classes.iter().map(|e| e.class).collect();
        for t in ALL_BOUNDARIES {
            let mut seen = BTreeSet::new();
            for e in t.own_controls.iter().flat_map(|b| b.iter()) {
                assert!(
                    !shared_ctl.contains(&e.cmd),
                    "{} ({:#010x}) is in both SHARED_CAPS and an own block at {:?}",
                    e.name,
                    e.cmd,
                    t.note
                );
                assert!(
                    seen.insert(e.cmd),
                    "{} ({:#010x}) is in two of {:?}'s own blocks",
                    e.name,
                    e.cmd,
                    t.note
                );
            }
            let mut seen = BTreeSet::new();
            for e in t.own_classes.iter().flat_map(|b| b.iter()) {
                assert!(
                    !shared_cls.contains(&e.class),
                    "{} ({:#010x}) is in both SHARED_CAPS and an own block at {:?}",
                    e.name,
                    e.class,
                    t.note
                );
                assert!(
                    seen.insert(e.class),
                    "{} ({:#010x}) is in two of {:?}'s own blocks",
                    e.name,
                    e.class,
                    t.note
                );
            }
        }
    }

    /// ★★ The deny table is shared, and that is only safe while no denied id is also a
    /// **boundary-specific** row.
    ///
    /// The hazard is concrete rather than hypothetical: 575.51.02 repurposes
    /// `0x20801358`, so a number can mean two commands. A deny keyed on a repurposed
    /// number would refuse a command nobody decided to refuse. Today the sets are
    /// disjoint; the day they are not, [`SharedCapabilities`]'s deny lists need a
    /// per-boundary half, and this is what says so.
    #[test]
    fn no_denied_id_is_a_boundary_specific_control() {
        let denied: BTreeSet<u32> = SHARED_CAPS.denied_controls.iter().map(|e| e.id).collect();
        let denied_cls: BTreeSet<u32> = SHARED_CAPS.denied_classes.iter().map(|e| e.id).collect();
        assert!(!denied.is_empty() && !denied_cls.is_empty());
        for t in ALL_BOUNDARIES {
            for e in t.own_controls.iter().flat_map(|b| b.iter()) {
                assert!(
                    !denied.contains(&e.cmd),
                    "{} ({:#010x}) is denied globally and permitted at {:?}",
                    e.name,
                    e.cmd,
                    t.note
                );
            }
            for e in t.own_classes.iter().flat_map(|b| b.iter()) {
                assert!(!denied_cls.contains(&e.class), "{:#010x}", e.class);
            }
        }
    }

    /// ★★★ **Each boundary's RESOLVED set, materialised** — counts, and the exact rows
    /// that boundary has beyond the shared base, spelled as literals.
    ///
    /// This is the gate the task asks for by name. A delta model — this one included —
    /// states a boundary's content indirectly, and the failure mode this repo has been
    /// bitten by is an edit that changes a *resolved* answer while every table still
    /// reads plausibly. Here the resolved answer is written down, so an edit that moves
    /// one changes this test or it changes nothing.
    ///
    /// ⊘ The expected values are literals, never read back out of the tables under test.
    #[test]
    fn each_boundarys_resolved_delta_is_materialised() {
        // (version, resolved control count, resolved class count, own control names)
        //
        // ★★ **+1 CONTROL at EVERY boundary on 2026-08-13:
        // `NV906F_CTRL_CMD_GET_MMU_FAULT_INFO` (`0x906f0106`).** It joined `CONTROLS_SHARED`,
        // which is in every boundary's resolved set by construction — so the numbers move
        // **together**, and that is the evidence rather than the bookkeeping. A change that
        // moved only SOME of them would mean a shared row had stopped being shared.
        //
        // ★ Every class count went up by ONE on 2026-08-01: `NV01_EVENT_KERNEL_CALLBACK_EX`
        // (`0x7e`) joined `CLASSES_SHARED`, and `CLASSES_SHARED` is in every boundary's
        // resolved set by construction. A change that moved only SOME of these numbers
        // would mean a shared row had stopped being shared, which is why they are pinned
        // per boundary rather than as one total.
        let want: &[ResolvedExpectation] = &[
            (
                "550.54.04",
                (550, 54, 4),
                156,
                77,
                &["NVC36F_CTRL_GET_CLASS_ENGINEID"],
            ),
            (
                "550.90.07",
                (550, 90, 7),
                157,
                77,
                &[
                    "NVC36F_CTRL_GET_CLASS_ENGINEID",
                    "NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE",
                ],
            ),
            (
                "555.42.02",
                (555, 42, 2),
                156,
                77,
                &["NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE"],
            ),
            (
                "560.28.03",
                (560, 28, 3),
                157,
                85,
                &[
                    "NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE",
                    "NV_SEMAPHORE_SURFACE_CTRL_CMD_UNBIND_CHANNEL",
                ],
            ),
            (
                "570.86.15",
                (570, 86, 15),
                159,
                91,
                &[
                    "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT",
                    "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS",
                    "NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE",
                    "NV_SEMAPHORE_SURFACE_CTRL_CMD_UNBIND_CHANNEL",
                ],
            ),
            (
                "575.51.02",
                (575, 51, 2),
                160,
                91,
                &[
                    "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT_V575",
                    "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS_V575",
                    "NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2",
                    "NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE",
                    "NV_SEMAPHORE_SURFACE_CTRL_CMD_UNBIND_CHANNEL",
                ],
            ),
            (
                "580.65.06",
                (580, 65, 6),
                160,
                93,
                &[
                    "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT_V575",
                    "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS_V575",
                    "NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2",
                    "NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE",
                    "NV_SEMAPHORE_SURFACE_CTRL_CMD_UNBIND_CHANNEL",
                ],
            ),
            (
                "610.43.02",
                (610, 43, 2),
                160,
                93,
                &[
                    "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT_V575",
                    "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS_V575",
                    "NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2",
                    "NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE",
                    "NV_SEMAPHORE_SURFACE_CTRL_CMD_UNBIND_CHANNEL",
                ],
            ),
        ];
        assert_eq!(
            want.len(),
            ALL_BOUNDARIES.len(),
            "a boundary has no row here"
        );
        for (label, (a, b, c), n_ctl, n_cls, own) in want {
            let t = at(*a, *b, *c);
            assert_eq!(t.all_controls().count(), *n_ctl, "{label} controls");
            assert_eq!(t.all_classes().count(), *n_cls, "{label} classes");
            let mut got: Vec<&str> = t
                .own_controls
                .iter()
                .flat_map(|blk| blk.iter())
                .map(|e| e.name)
                .collect();
            got.sort_unstable();
            assert_eq!(&got, own, "{label} boundary-specific controls");
        }
        // ★ Non-vacuity for the whole table: the resolved counts must not all be the
        // same number, or every assertion above would pass on a shape with no seam.
        let counts: BTreeSet<usize> = want.iter().map(|w| w.2).collect();
        assert!(counts.len() > 1, "no boundary changes the control count");
    }

    /// No id is permitted **and** denied, and no boundary re-adds a row a lower one
    /// already has.
    ///
    /// The second half is what keeps `all_controls()`/`all_classes()` an honest census:
    /// a duplicated row would be counted twice and the counts below would drift for a
    /// reason that has nothing to do with coverage.
    #[test]
    fn the_permitted_and_denied_sets_are_disjoint_and_each_id_appears_once() {
        let ctl: Vec<u32> = bench().all_controls().map(|e| e.cmd).collect();
        let cls: Vec<u32> = bench().all_classes().map(|e| e.class).collect();
        let dctl: BTreeSet<u32> = bench().all_denied_controls().map(|e| e.id).collect();
        let dcls: BTreeSet<u32> = bench().all_denied_classes().map(|e| e.id).collect();

        assert_eq!(
            ctl.len(),
            ctl.iter().collect::<BTreeSet<_>>().len(),
            "a control is listed at two boundaries"
        );
        assert_eq!(
            cls.len(),
            cls.iter().collect::<BTreeSet<_>>().len(),
            "a class is listed at two boundaries"
        );
        for c in &ctl {
            assert!(
                !dctl.contains(c),
                "control {c:#010x} is both listed and denied"
            );
        }
        for c in &cls {
            assert!(
                !dcls.contains(c),
                "class {c:#010x} is both listed and denied"
            );
        }
    }

    /// Every row names itself with an NVIDIA identifier.
    ///
    /// A hex number with no name is a row nobody can review, and the C's own list has
    /// exactly that problem for nine rows — which is why those nine are not carried here
    /// (see [`RULE_COVERED_C_ROWS`]).
    #[test]
    fn every_row_carries_a_name() {
        // ★ The predicate is "an upper-case NVIDIA-style identifier", not "starts with
        // NV" — that stronger form was written first and it was WRONG, in both
        // directions. Class names are frequently not `NV*` (`AMPERE_B`, `TURING_A`,
        // `KEPLER_CHANNEL_GROUP_A`), and one *control* is not either: see the pin below.
        let named = |n: &str| {
            n.len() > 3
                && n.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        };
        for e in bench().all_controls() {
            assert!(named(e.name), "control {:#010x} name {:?}", e.cmd, e.name);
        }
        for e in bench().all_classes() {
            assert!(named(e.name), "class {:#010x} name {:?}", e.class, e.name);
        }
        for e in bench().all_denied_controls() {
            assert!(
                named(e.name),
                "denied control {:#010x} name {:?}",
                e.id,
                e.name
            );
        }
        for e in bench().all_denied_classes() {
            assert!(
                named(e.name),
                "denied class {:#010x} name {:?}",
                e.id,
                e.name
            );
        }
    }

    /// ★ **Exactly one row has no NVIDIA name**, and it is pinned so the hole cannot
    /// grow quietly.
    ///
    /// `0x0080028b` appears in nvproxy's map under the placeholder
    /// `UNKNOWN_CONTROL_COMMAND_80028B` and is in **neither** vendored open-kernel tree,
    /// so there is no name to give it. It is carried because the C carried it. Every
    /// other row of the 160 resolves to a real identifier — nine C rows that did not
    /// were dropped instead (see [`RULE_COVERED_C_ROWS`]).
    #[test]
    fn exactly_one_control_row_is_an_upstream_placeholder() {
        let unnamed: Vec<u32> = bench()
            .all_controls()
            .filter(|e| !e.name.starts_with("NV"))
            .map(|e| e.cmd)
            .collect();
        assert_eq!(unnamed, vec![0x0080_028b]);
    }

    // ── Non-vacuity: every row can matter, and the default is a denial ───────────────

    /// ★★ **Every listed control is load-bearing.** For each row, the answer is
    /// `Listed` — not `GssLegacyRule`, not `BinApiRule`. A row the rules already cover
    /// could be deleted with no observable effect, which is a row that is not coverage.
    ///
    /// This is what forced nine of the C's 165 rows out of the port: they all have the
    /// GSS-legacy bit set or sit in the binary-API class, so the C's table was carrying
    /// nine entries that could never change an answer.
    #[test]
    fn every_listed_control_is_the_reason_it_is_permitted() {
        for e in bench().all_controls() {
            match bench().control(ControlCmd(e.cmd)) {
                ControlPermit::Listed { name, .. } => assert_eq!(name, e.name),
                other => panic!(
                    "{} ({:#010x}) is permitted by {other:?}, not by its own row — \
                     delete the row or explain it",
                    e.name, e.cmd
                ),
            }
        }
    }

    /// Every listed class is answered by its own row.
    #[test]
    fn every_listed_class_is_the_reason_it_is_permitted() {
        for e in bench().all_classes() {
            match bench().alloc_class(ClassId(e.class)) {
                AllocPermit::Listed { name, .. } => assert_eq!(name, e.name),
                other => panic!("{} ({:#010x}) answered {other:?}", e.name, e.class),
            }
        }
    }

    /// ★ The nine C rows this port drops are still permitted — by rule.
    ///
    /// Pinned by hex, because *"we removed them because a rule covers them"* is only
    /// true while the rule covers them. If a future edit narrows
    /// [`RM_GSS_LEGACY_MASK`] or [`NV2081_BINAPI_CLASS`], this is what says that nine
    /// commands a real driver issues just became denials.
    #[test]
    fn the_c_rows_dropped_as_rule_covered_are_still_permitted() {
        for &cmd in RULE_COVERED_C_ROWS {
            let p = bench().control(ControlCmd(cmd));
            assert!(
                matches!(p, ControlPermit::GssLegacyRule | ControlPermit::BinApiRule),
                "{cmd:#010x} was dropped from the table as rule-covered and is now {p:?}"
            );
        }
        assert_eq!(
            RULE_COVERED_C_ROWS.len(),
            9,
            "the C had exactly nine such rows"
        );
    }

    /// ★★★ **The gap this module closes.** A command nobody has ever seen is refused,
    /// and refused with the *default* reason rather than a named one.
    #[test]
    fn an_unknown_control_is_denied_by_default() {
        // ★ Every value here has bit 15 CLEAR. The first draft used `0xdeadbeef` and
        // `0x2080ffff` and both came back `GssLegacyRule` — see
        // `the_gss_legacy_rule_passes_half_the_command_space`, which is the finding that
        // mistake surfaced.
        for cmd in [
            0x0000_0000u32,
            0x0080_1815,
            0x2080_7fff,
            0xdead_3eef,
            0xcafe_0001,
        ] {
            assert_eq!(cmd & RM_GSS_LEGACY_MASK, 0, "bad fixture: {cmd:#010x}");
            assert_eq!(
                bench().control(ControlCmd(cmd)),
                ControlPermit::Denied(Denial::NotOnAllowlist),
                "{cmd:#010x}"
            );
        }
    }

    /// ★★★ **The widest hole in this gate, measured and pinned rather than discovered
    /// later.**
    ///
    /// [`RM_GSS_LEGACY_MASK`] is bit 15 of the command word, so the rule passes
    /// **exactly half** of the 32-bit command space with no table row and no review.
    /// That is nvproxy's own posture (`gvisor/pkg/sentry/devices/nvproxy/frontend.go:769-780`
    /// — *"its parameters cannot reasonably contain application pointers, and the
    /// control is in any case undocumented"*) and the C's
    /// (`C: nvkvm_isolate_handlers.c`, `cmd & 0x8000u`), so it is ported verbatim: this
    /// is a port, and narrowing it would be a redesign made on no evidence.
    ///
    /// But *"default-deny"* is only true of the half of the space without bit 15, and a
    /// reader of this module is owed that sentence in a form that cannot rot. If the rule
    /// is ever narrowed, this test is the one that has to be edited — deliberately.
    #[test]
    fn the_gss_legacy_rule_passes_half_the_command_space() {
        assert_eq!(RM_GSS_LEGACY_MASK, 1 << 15);
        // A dense sample across the space: every value with the bit set passes on the
        // rule alone, whatever else it is.
        for hi in [0x0000u32, 0x0080, 0x2080, 0xdead, 0xffff] {
            for lo in [0x8000u32, 0x8001, 0xbeef, 0xffff] {
                let cmd = (hi << 16) | lo;
                assert_eq!(
                    bench().control(ControlCmd(cmd)),
                    ControlPermit::GssLegacyRule,
                    "{cmd:#010x}"
                );
            }
        }
        // …and the binary-API class is the second, much narrower, hole: one class in
        // 65 536, and only for commands the legacy rule did not already take.
        assert_eq!(
            bench().control(ControlCmd(0x2081_0001)),
            ControlPermit::BinApiRule
        );
        assert_eq!(
            bench().control(ControlCmd(0x2082_0001)),
            ControlPermit::Denied(Denial::NotOnAllowlist)
        );
    }

    /// The same for an allocation class.
    #[test]
    fn an_unknown_alloc_class_is_denied_by_default() {
        for class in [0xf0_01u32, 0x0000_ffff, 0x0000_c798] {
            assert_eq!(
                bench().alloc_class(ClassId(class)),
                AllocPermit::Denied(Denial::NotOnAllowlist),
                "{class:#010x}"
            );
        }
    }

    /// A row on the deny table refuses **by name, with a reason** — a different answer
    /// from "never heard of it", which is the whole reason the table exists.
    #[test]
    fn a_deliberately_refused_row_says_why() {
        assert_eq!(
            bench().control(ControlCmd(0x2080_0122)),
            ControlPermit::Denied(Denial::Refused {
                name: "NV2080_CTRL_CMD_GPU_EXEC_REG_OPS",
                why: DeniedBecause::RegisterAccess,
            })
        );
        assert_eq!(
            bench().control(ControlCmd(0xb0cc_0105)),
            ControlPermit::Denied(Denial::Refused {
                name: "NVB0CC_CTRL_CMD_ALLOC_PMA_STREAM",
                why: DeniedBecause::PerformanceCounters,
            })
        );
        assert_eq!(
            bench().alloc_class(ClassId(0x0000_0071)),
            AllocPermit::Denied(Denial::Refused {
                name: "NV01_MEMORY_SYSTEM_OS_DESCRIPTOR",
                why: DeniedBecause::CallerMemoryDescriptor,
            })
        );
    }

    /// ★★ **Deny beats the rule-based passthrough**, which is the one ordering property
    /// production data cannot currently exercise: no denied row has the GSS-legacy bit,
    /// and the disjointness test above is what keeps that true. So the order is pinned
    /// against a table built for the purpose — a real `CapabilityTable`, through the real
    /// [`CapabilityTable::control`], with one synthetic row whose id *does* carry the bit.
    ///
    /// Without this, `cmd & 0x8000` would be a silent override of every future "this is
    /// dangerous" row, and nothing would say so until a guest used one.
    #[test]
    fn deny_beats_the_rule_based_passthrough() {
        static DENIED: &[DeniedEntry] = &[DeniedEntry {
            id: 0x2080_8123,
            name: "NV2080_CTRL_CMD_SYNTHETIC_LEGACY_MASKED",
            why: DeniedBecause::RegisterAccess,
        }];
        static S: SharedCapabilities = SharedCapabilities {
            controls: &[],
            classes: &[],
            denied_controls: DENIED,
            denied_classes: &[],
        };
        static T: CapabilityTable = CapabilityTable {
            shared: &S,
            own_controls: &[],
            own_classes: &[],
            note: "test fixture",
        };
        // The premise: the rule really would have passed it.
        assert_ne!(0x2080_8123u32 & RM_GSS_LEGACY_MASK, 0);
        assert_eq!(
            T.control(ControlCmd(0x2080_8123)),
            ControlPermit::Denied(Denial::Refused {
                name: "NV2080_CTRL_CMD_SYNTHETIC_LEGACY_MASKED",
                why: DeniedBecause::RegisterAccess,
            })
        );
        // …and a neighbouring id with the same bit and no row still passes, so the
        // assertion above is about the row and not about the mask being broken.
        assert_eq!(
            T.control(ControlCmd(0x2080_8124)),
            ControlPermit::GssLegacyRule
        );
    }

    // ── The founding entries: shortening the list must fail a test ───────────────────

    /// ★★★ **The pin.** Named rows the port is known to need, asserted individually, so
    /// deleting any one of them fails *this* test rather than quietly shrinking the
    /// universe every other test is quantified over.
    ///
    /// Chosen to span the whole surface rather than to be a sample: the client-root and
    /// device controls CUDA cannot start without, the channel and compute controls, the
    /// four Mode-2 page-directory rows, and the two classes that only a 580 guest gets.
    #[test]
    fn the_founding_rows_are_present_by_name() {
        let ctl = |cmd: u32, name: &str| match bench().control(ControlCmd(cmd)) {
            ControlPermit::Listed { name: got, .. } => assert_eq!(got, name, "{cmd:#010x}"),
            other => panic!("{name} ({cmd:#010x}) is missing: {other:?}"),
        };
        ctl(0x0000_0101, "NV0000_CTRL_CMD_SYSTEM_GET_BUILD_VERSION");
        ctl(0x0000_0202, "NV0000_CTRL_CMD_GPU_GET_ID_INFO");
        ctl(0x0000_0205, "NV0000_CTRL_CMD_GPU_GET_ID_INFO_V2");
        ctl(0x0000_3d05, "NV0000_CTRL_CMD_OS_UNIX_EXPORT_OBJECT_TO_FD");
        ctl(0x0000_3d06, "NV0000_CTRL_CMD_OS_UNIX_IMPORT_OBJECT_FROM_FD");
        ctl(0x0080_0201, "NV0080_CTRL_CMD_GPU_GET_CLASSLIST");
        ctl(0x0080_1401, "NV0080_CTRL_CMD_HOST_GET_CAPS");
        ctl(0x2080_018e, "NV2080_CTRL_CMD_GPU_GET_PID_INFO");
        ctl(0x9096_0101, "NV9096_CTRL_CMD_SET_ZBC_COLOR_CLEAR");
        ctl(0xa06c_0101, "NVA06C_CTRL_CMD_GPFIFO_SCHEDULE");
        ctl(0xc36f_0108, "NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN");
        ctl(0xc56f_010b, "NVC56F_CTRL_CMD_GET_KMB");
        // ★ The six Mode-2 rows — the C's list has NONE of them, and that is the
        // port's most load-bearing finding: in Mode 1 these never crossed the ioctl
        // boundary, so the list the C validated against 22 applications could not have
        // contained them. Two of the six are the canonical Case-2 controls
        // (`execution_plane.md` §2.5); four are the page-directory family.
        ctl(0x2080_012b, "NV2080_CTRL_CMD_GPU_PROMOTE_CTX");
        ctl(0x2080_1219, "NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_INFO");
        ctl(0x0080_1813, "NV0080_CTRL_CMD_DMA_SET_PAGE_DIRECTORY");
        ctl(0x0080_1814, "NV0080_CTRL_CMD_DMA_UNSET_PAGE_DIRECTORY");
        ctl(
            0x2080_0a9f,
            "NV2080_CTRL_CMD_INTERNAL_GMMU_COPY_RESERVED_SPLIT_GVASPACE_PDES_TO_SERVER",
        );
        ctl(
            0x90f1_0106,
            "NV90F1_CTRL_CMD_VASPACE_COPY_SERVER_RESERVED_PDES",
        );

        let cls = |class: u32, name: &str| match bench().alloc_class(ClassId(class)) {
            AllocPermit::Listed { name: got, .. } => assert_eq!(got, name, "{class:#010x}"),
            other => panic!("{name} ({class:#010x}) is missing: {other:?}"),
        };
        cls(0x0000_0000, "NV01_ROOT");
        cls(0x0000_0041, "NV01_ROOT_CLIENT");
        cls(0x0000_0005, "NV01_EVENT");
        cls(0x0000_0080, "NV01_DEVICE_0");
        cls(0x0000_2080, "NV20_SUBDEVICE_0");
        cls(0x0000_90f1, "FERMI_VASPACE_A");
        cls(0x0000_9067, "FERMI_CONTEXT_SHARE_A");
        cls(0x0000_a06c, "KEPLER_CHANNEL_GROUP_A");
        cls(0x0000_c56f, "AMPERE_CHANNEL_GPFIFO_A");
        cls(0x0000_c7c0, "AMPERE_COMPUTE_B");
        cls(0x0000_c7b5, "AMPERE_DMA_COPY_B");
        cls(0x0000_c797, "AMPERE_B");
        // ★ §16.24 — the one class on this pin that no CUDA process and no RM bring-up
        // path allocates: UVM's own kernel-side channel code does, once per CE channel.
        cls(0x0000_c076, "GP100_UVM_SW");
        cls(0x0000_ceb7, "NVCEB7_VIDEO_ENCODER");
        cls(0x0000_d1b7, "NVD1B7_VIDEO_ENCODER");
    }

    /// The reviewed size of the ported surface, per boundary.
    ///
    /// A ratchet, in this repo's idiom: it catches a row leaving as loudly as one
    /// arriving, which the founding-rows pin alone cannot do for the 140-odd rows it
    /// does not name.
    ///
    /// 159 controls = the C's 165 minus the 9 rule-covered rows, plus the 6 Mode-2 rows,
    /// **minus the 3 `NV83DE` debug controls** this port moved onto the deny table
    /// (see [`DENIED_CONTROLS`]'s doc). 90 classes at 580 = the C's 89 plus
    /// `NVCEB7`/`NVD1B7`, which nvproxy adds at 580.65.06 and the C's 575-era list
    /// therefore could not have, **minus `GT200_DEBUGGER`**, moved for the same reason.
    ///
    /// ★ The four class counts move together because `GT200_DEBUGGER` was in the SHARED
    /// base: a class this port never modelled was permitted at *every* boundary, which is
    /// exactly the direction a default-deny table must not drift in.
    ///
    /// ★★ **+1 at every boundary on 2026-08-09 (§16.24): `GP100_UVM_SW` (`0xc076`).** It
    /// is SHARED for the same reason `GT200_DEBUGGER` was — the class is Pascal-plus and
    /// version-invariant, and UVM allocates it on every CE channel it creates on every
    /// driver this table spans. ⚠ This ratchet is what makes admitting a class a
    /// deliberate act; it moved because a boot named the row, and the four numbers moving
    /// **together** is the evidence that the row went into the shared base rather than
    /// into one boundary by accident.
    ///
    /// ★★ **+1 again at every boundary on 2026-08-10: `UVM_CHANNEL_RETAINER` (`0xc574`).**
    /// SHARED for the same reason as its two predecessors — the class is version-invariant
    /// and UVM takes the reference on every driver this table spans — and the four numbers
    /// again move **together**, which is the evidence rather than the bookkeeping.
    ///
    /// ⚠ Unlike `GP100_UVM_SW`, this row was **not** admitted because a boot showed it to be
    /// fatal. It is admitted because serving it is cheap and honest: both params are `[IN]`
    /// and the constructor writes nothing back, so an echoed reply invents nothing. ⊘ It is
    /// therefore **not** expected to move the progress fraction, and a report that credits it
    /// with movement is reading a coincidence — `mode2_cuctxcreate_resume.md` §0.6-0.7 rules
    /// this port's wall to be a completion, not a control-plane refusal.
    #[test]
    fn the_ported_surface_is_the_reviewed_size() {
        // ★ **+1 on 2026-08-13: `NV906F_CTRL_CMD_GET_MMU_FAULT_INFO` (`0x906f0106`).**
        // SHARED, so this number and every boundary's below move **together** — which is the
        // evidence the row went into the shared base rather than into one boundary by
        // accident. ⊘ It is version-invariant: `ctrl906f.h`'s params struct is identical
        // across the tags this table spans.
        assert_eq!(bench().all_controls().count(), 160, "controls");
        assert_eq!(at(550, 54, 4).all_classes().count(), 77, "classes at 550");
        assert_eq!(at(560, 28, 3).all_classes().count(), 85, "classes at 560");
        assert_eq!(at(570, 86, 15).all_classes().count(), 91, "classes at 570");
        assert_eq!(bench().all_classes().count(), 93, "classes at 580");
        assert_eq!(bench().all_denied_controls().count(), 13, "denied controls");
        assert_eq!(bench().all_denied_classes().count(), 4, "denied classes");
    }

    /// The origins are all populated — a `Mode2Rpc` count of zero would mean the
    /// transport delta silently vanished, and the port's most load-bearing finding with
    /// it.
    #[test]
    fn each_origin_is_represented() {
        let n = |o: Origin| bench().all_controls().filter(|e| e.origin == o).count();
        // ★ 6 → 7 on 2026-08-13: `NV906F_CTRL_CMD_GET_MMU_FAULT_INFO`. `Mode2Rpc` is the
        // right provenance and not a convenience: the control is `ROUTE_TO_PHYSICAL`, so on a
        // GSP client it is RPC'd to the GSP — us — and as an ioctl it never crossed the
        // boundary nvproxy gates.
        assert_eq!(n(Origin::Mode2Rpc), 7);
        assert_eq!(n(Origin::Empirical), 5);
        assert_eq!(n(Origin::Nvproxy), 148);
        assert_eq!(
            bench()
                .all_classes()
                .filter(|e| e.origin == Origin::Empirical)
                .count(),
            2,
            "NV01_EVENT and AMPERE_B, the C's two #84 class additions"
        );
    }

    // ── The version seam actually bites ──────────────────────────────────────────────

    /// ★ Adding a driver version is a table row, and the row **changes the answer**.
    ///
    /// Three boundaries, each asserted from both sides. Without the "one patch below"
    /// half, a table keyed on the major would pass this too — which is the C's own
    /// version-key bug, and the reason `versions.rs` keys on all three numbers.
    #[test]
    fn a_class_added_at_a_boundary_is_denied_one_patch_below_it() {
        let denied = |t: &'static CapabilityTable, c: u32| {
            assert_eq!(
                t.alloc_class(ClassId(c)),
                AllocPermit::Denied(Denial::NotOnAllowlist),
                "{c:#010x} should not exist yet"
            );
        };
        let listed = |t: &'static CapabilityTable, c: u32| {
            assert!(t.alloc_class(ClassId(c)).is_permitted(), "{c:#010x}");
        };
        // BLACKWELL_A arrives at 560.28.03.
        denied(at(560, 28, 2), 0x0000_cd97);
        listed(at(560, 28, 3), 0x0000_cd97);
        // BLACKWELL_B arrives at 570.86.15.
        denied(at(570, 86, 14), 0x0000_ce97);
        listed(at(570, 86, 15), 0x0000_ce97);
        // NVCEB7 arrives at 580.65.06 — the boundary the C's 575-era list predates.
        denied(at(580, 65, 5), 0x0000_ceb7);
        listed(at(580, 65, 6), 0x0000_ceb7);
        // …and the shared base really is shared: it answers at the top too.
        listed(at(610, 43, 2), 0x0000_0080);
        listed(at(610, 43, 2), 0x0000_cd97);
    }

    // ── Derived, not listed: the crate's own model must be inside its own gate ───────

    /// ★★★ **Derivation, not a list.** Sweep the *entire* 16-bit class space: any class
    /// [`crate::versions::DriverAbiTable::alloc_params`] claims to decode MUST be
    /// permitted, or the port models something its own boundary would refuse.
    ///
    /// This is the property the task's trap asks for — the universe is the class space
    /// itself, so a decoder added tomorrow without a table row is a **failure by
    /// default**, not an omission nobody notices.
    #[test]
    fn every_class_this_port_decodes_is_permitted() {
        let abi = table_for(crate::versions::BENCH_DRIVER).expect("bench");
        let mut seen = 0usize;
        for class in 0u32..=0xffff {
            let Some(params) = abi.alloc_params(ClassId(class)) else {
                continue;
            };
            seen += 1;
            assert!(
                abi.capabilities()
                    .alloc_class(ClassId(class))
                    .is_permitted(),
                "{class:#010x} decodes as {params:?} but the capability table refuses it"
            );
        }
        // 11 → 12 on 2026-08-08 (`execution_plane_increments.md` §14.26): `AMPERE_B`
        // (`0xc797`), the golden-image channel's 3D object, which boot `pro1_423bf08`
        // put on the wire for the first time.
        // 12 → 13 on 2026-08-08 (`execution_plane_increments.md` §14.28): `NV2081_BINAPI`
        // (`0x2081`), the class an injection experiment on a real GA106 measured to be the
        // difference between `cuInit(0) = 100` and `cuInit(0) = 0`.
        // 13 → 14 on 2026-08-09 (`execution_plane_increments.md` §16.24): `GP100_UVM_SW`
        // (`0xc076`), UVM's per-channel fault-cancel SW object — refused four times in
        // boot `s22_f4f3865`'s `cuInit` window, once per UVM channel, each refusal fatal
        // to its channel at `ogkm-580: nv_gpu_ops.c:6120`.
        // 14 → 15 on 2026-08-10 (`execution_plane_increments.md` §16.76):
        // `NV01_EVENT_OS_EVENT` (`0x79`), the class libcuda binds its blocking-sync os-event
        // to. ★ The class was PERMITTED here from the beginning and undecodable in the
        // params table, so seven registrations in `w209_ffc80f8_ctl` were refused
        // `0x56` by the decoder gate and not by the boundary — which is exactly the
        // asymmetry this test's own docs describe, seen from the other side.
        // 15 → 16 on 2026-08-10: `UVM_CHANNEL_RETAINER` (`0xc574`), UVM's reference on a
        // channel it did not create. ★ The only row on this list admitted for its PARAMETER
        // DIRECTIONS rather than for a measured failure: `{hClient, hChannel}` are both
        // `[IN]` and `uvmchanrtnrConstruct_IMPL` writes nothing back, so `NoDeclaredFacts`
        // plus an echoed body forges no value. ⊘ Cheap and correct is the whole claim; it is
        // not claimed to be the wall.
        assert_eq!(seen, 16, "the port decodes sixteen classes today");
        // The sweep must really have covered a class the table refuses, or it proves
        // nothing about the table.
        assert!(
            !abi.capabilities()
                .alloc_class(ClassId(0x0000_0071))
                .is_permitted()
        );
    }

    /// ★★★ **The two live consequences of task #118, now asserting the RIGHT answer.**
    ///
    /// This test was `the_575_boundary_is_subtractive_and_this_shape_cannot_carry_it`: a
    /// characterisation that pinned two wrong answers because the shape could not say
    /// anything else. Task #122 rebuilt the shape, so the same two questions are asked
    /// here and the expected answers are nvproxy's.
    ///
    /// nvproxy's `v575_51_02` deletes two control commands and re-adds them one number
    /// lower (`gvisor nvproxy: version.go:1036-1053`); the pair first appears at
    /// `v570_86_15` (`gvisor nvproxy: version.go:1005-1006`) and exists at no earlier
    /// version:
    ///
    /// | command | ≤ 565 | 570 | ≥ 575 |
    /// |---|---|---|---|
    /// | `..._FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT` | — | `0x20801358` | `0x20801357` |
    /// | `..._FB_QUERY_DRAM_ENCRYPTION_STATUS` | — | `0x20801359` | `0x20801358` |
    ///
    /// What changed, stated as before → after:
    ///
    /// 1. `0x20801359` was **refused at every version**. It is now **permitted at
    ///    570.86.15**, which is the only boundary whose vendor map has it, and refused
    ///    at 550/555/560 (it did not exist) and at 575+ (deleted).
    /// 2. `0x20801358` was permitted **at every version under the 575-era name**. It is
    ///    now `..._INFOROM_SUPPORT` at 570, `..._STATUS_V575` at 575+, and **refused**
    ///    below 570 — the same number, two commands, and neither answer borrowed from
    ///    the other.
    ///
    /// ⊘ Every expected value here is a literal. Nothing is read back out of the table
    /// under test.
    #[test]
    fn the_575_boundary_replaces_two_dram_encryption_commands() {
        const INFOROM_PRE575: u32 = 0x2080_1358;
        const STATUS_PRE575: u32 = 0x2080_1359;
        const INFOROM_V575: u32 = 0x2080_1357;

        // The 575 row is its own row: a fall-through to 570's would make the whole test
        // assert about one boundary twice. `DriverAbiTable` exposes no version accessor,
        // so it is identified by the one public field that distinguishes it.
        let row = table_for(DriverVersion {
            major: 575,
            minor: 51,
            patch: 2,
        })
        .expect("575.51.02 has a row");
        assert!(
            row.note.contains("CAPS_575_51_02"),
            "575.51.02 resolved to a row whose note is {:?} — it must resolve to its OWN \
             row, not fall through to 570's",
            row.note
        );

        // Expected answer per boundary, as (version, permitted?, NVIDIA name).
        // `None` = refused; a name = permitted under exactly that name.
        let expect = |cmd: u32, rows: &[PerVersionAnswer]| {
            for ((a, b, c), want) in rows {
                let got = at(*a, *b, *c).control(ControlCmd(cmd));
                match want {
                    Some(name) => assert_eq!(
                        got,
                        ControlPermit::Listed {
                            name,
                            origin: Origin::Nvproxy
                        },
                        "{cmd:#010x} at {a}.{b}.{c}"
                    ),
                    None => assert_eq!(
                        got,
                        ControlPermit::Denied(Denial::NotOnAllowlist),
                        "{cmd:#010x} at {a}.{b}.{c}"
                    ),
                }
            }
        };

        // (1) The pre-575 STATUS number: refused, then permitted for exactly one
        //     boundary, then refused again. Before #122 every one of these was refused.
        expect(
            STATUS_PRE575,
            &[
                ((550, 54, 4), None),
                ((550, 90, 7), None),
                ((555, 42, 2), None),
                ((560, 28, 3), None),
                (
                    (570, 86, 15),
                    Some("NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS"),
                ),
                ((575, 51, 2), None),
                ((580, 65, 6), None),
                ((610, 43, 2), None),
            ],
        );

        // (2) The overlapping number: two different commands on two sides of 575, and
        //     nothing at all below 570. Before #122 it was `..._STATUS_V575` everywhere.
        expect(
            INFOROM_PRE575,
            &[
                ((550, 54, 4), None),
                ((550, 90, 7), None),
                ((555, 42, 2), None),
                ((560, 28, 3), None),
                (
                    (570, 86, 15),
                    Some("NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT"),
                ),
                (
                    (575, 51, 2),
                    Some("NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS_V575"),
                ),
                (
                    (580, 65, 6),
                    Some("NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS_V575"),
                ),
                (
                    (610, 43, 2),
                    Some("NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS_V575"),
                ),
            ],
        );

        // …and the third number in the family, which only ever means the 575 command.
        expect(
            INFOROM_V575,
            &[
                ((550, 54, 4), None),
                ((570, 86, 15), None),
                (
                    (575, 51, 2),
                    Some("NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT_V575"),
                ),
            ],
        );

        // ★ The boundary is exact on the patch, not on the major: 575.51.01 is still a
        // 570 guest and must still get the 570 answer.
        assert_eq!(
            at(575, 51, 1).control(ControlCmd(STATUS_PRE575)),
            ControlPermit::Listed {
                name: "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS",
                origin: Origin::Nvproxy,
            }
        );
    }

    /// ★★ The **second** removal, at an independent boundary and in the other direction:
    /// a row present at the bottom of the range and deleted partway up.
    ///
    /// `NVC36F_CTRL_GET_CLASS_ENGINEID` is in nvproxy's base control map
    /// (`gvisor nvproxy: version.go:360`, compute capability) and deleted at `v555_42_02`
    /// (`gvisor nvproxy: version.go:933`). Under the old shape the port had no row for it
    /// at all — the C's list is the 575-era set, by which time it was long gone — so a
    /// 550 guest issuing it was refused.
    ///
    /// It matters that this is a *different* boundary from 575.51.02: a shape that
    /// happened to work for one replace-in-place could still be wrong for a plain delete,
    /// and one worked example is not a mechanism.
    #[test]
    fn the_555_boundary_deletes_a_control_the_550_guest_still_has() {
        const GET_CLASS_ENGINEID: u32 = 0xc36f_0101;
        let listed = ControlPermit::Listed {
            name: "NVC36F_CTRL_GET_CLASS_ENGINEID",
            origin: Origin::Nvproxy,
        };
        let gone = ControlPermit::Denied(Denial::NotOnAllowlist);
        for ((a, b, c), want) in [
            ((550u16, 54u16, 4u16), listed),
            ((550, 90, 7), listed),
            ((555, 42, 1), listed),
            ((555, 42, 2), gone),
            ((560, 28, 3), gone),
            ((580, 65, 6), gone),
            ((610, 43, 2), gone),
        ] {
            assert_eq!(
                at(a, b, c).control(ControlCmd(GET_CLASS_ENGINEID)),
                want,
                "{GET_CLASS_ENGINEID:#010x} at {a}.{b}.{c}"
            );
        }
        // ★ The sibling numbers in the same class are untouched by the delete — without
        // this, deleting the whole `0xc36f` prefix would pass the assertions above.
        assert!(
            at(555, 42, 2)
                .control(ControlCmd(0xc36f_0108))
                .is_permitted(),
            "NVC36F_CTRL_CMD_GPFIFO_GET_WORK_SUBMIT_TOKEN is not deleted at 555.42.02"
        );
    }

    #[test]
    fn every_control_this_port_models_is_permitted() {
        let abi = table_for(crate::versions::BENCH_DRIVER).expect("bench");
        let prefixes: BTreeSet<u32> = abi
            .capabilities()
            .all_controls()
            .map(|e| e.cmd >> 16)
            .chain(abi.capabilities().all_denied_controls().map(|e| e.id >> 16))
            .collect();
        assert!(prefixes.len() > 10, "the sweep would be trivial");
        let mut modelled = 0usize;
        for p in prefixes {
            for lo in 0u32..=0xffff {
                let cmd = ControlCmd((p << 16) | lo);
                let Some(kind) = abi.control_params(cmd) else {
                    continue;
                };
                modelled += 1;
                assert!(
                    abi.capabilities().control(cmd).is_permitted(),
                    "{:#010x} models {kind:?} but the capability table refuses it",
                    cmd.0
                );
            }
        }
        assert_eq!(modelled, 5, "the port models five controls today");
    }

    /// ★ The capability gate must not **pre-empt** the more informative refusals.
    ///
    /// `SetPageDir` and `VaspacePublishedPdes` are the controls the port turns into a
    /// fact and `PageDirNotModelled` is the port's named diagnostic for the one it does
    /// not; all three are only reachable if the gate lets them through first. This is the
    /// test that fails if someone "tidies up" the Mode-2 rows out of the table because the
    /// C did not have them.
    #[test]
    fn the_page_directory_controls_survive_the_gate() {
        let abi = table_for(crate::versions::BENCH_DRIVER).expect("bench");
        for (cmd, want) in [
            (0x0080_1813u32, ControlParams::SetPageDir),
            (0x0080_1814, ControlParams::PageDirNotModelled),
            // ★★★ The two PUBLICATION ids, and they are the ones that actually carry a
            // page-directory base on the boot path — §14.9 measured `0x00801813` at zero
            // occurrences in a whole init and these two at five.
            (0x2080_0a9f, ControlParams::VaspacePublishedPdes),
            (0x90f1_0106, ControlParams::VaspacePublishedPdes),
            // ★ The address-plane control joins the same pairing: it is the only other
            // control the port turns into facts, and it is equally worth failing loudly
            // if a tidy-up removes its row.
            (0x2080_012b, ControlParams::PromoteCtx),
        ] {
            assert!(
                abi.capabilities().control(ControlCmd(cmd)).is_permitted(),
                "{cmd:#010x}"
            );
            assert_eq!(abi.control_params(ControlCmd(cmd)), Some(want));
        }
    }

    /// `AllocParams` is reachable for every class the founding pin names as decodable —
    /// the other half of the previous test's pairing, so a class that stays permitted
    /// but loses its decoder is also visible.
    #[test]
    fn the_decoded_classes_keep_their_decoders() {
        let abi = table_for(crate::versions::BENCH_DRIVER).expect("bench");
        for (class, want) in [
            (0x0000_0000u32, AllocParams::ClientRoot),
            (0x0000_0041, AllocParams::ClientRoot),
            (0x0000_0080, AllocParams::Device),
            (0x0000_a06c, AllocParams::Tsg),
            (0x0000_9067, AllocParams::CtxShare),
            (0x0000_c56f, AllocParams::Channel),
        ] {
            assert_eq!(
                abi.alloc_params(ClassId(class)),
                Some(want),
                "{class:#010x}"
            );
        }
    }
}
