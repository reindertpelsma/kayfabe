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
//! ## Ordering, which is a security property
//!
//! [`CapabilityTable::control`] answers in this order, and the order is what the
//! `deny_beats_the_rule_based_passthrough` test pins:
//!
//! 1. an **explicit denial** ([`DeniedEntry`]) — named, with a reason;
//! 2. the two **rule-based passthroughs** the C implements in code rather than in a
//!    table: the GSS-legacy mask ([`RM_GSS_LEGACY_MASK`]) and the binary-API class
//!    ([`NV2081_BINAPI_CLASS`]). Both are GSP-routed with no app pointers
//!    (`gvisor/pkg/sentry/devices/nvproxy/frontend.go:756-816`);
//! 3. the **allowlist**, walked up the inheritance chain;
//! 4. otherwise [`Denial::NotOnAllowlist`].
//!
//! Denial first is stricter than nvproxy, which checks its two rules *before* the map. It
//! costs nothing today — the two sets are provably disjoint, which is its own test — and
//! it means a future "this is dangerous" row cannot be silently outvoted by a bit.
//!
//! ## The version seam
//!
//! [`CapabilityTable`] is **inherit-then-add**, the same shape as
//! [`crate::versions::TABLES`] and as nvproxy's own registry
//! (`gvisor/pkg/sentry/devices/nvproxy/version.go`), and each
//! [`crate::versions::DriverAbiTable`] names one. So *adding a driver version costs a
//! table entry and edits no logic crate* — which is the constraint, stated as data.
//!
//! Three boundaries are wired, all read out of nvproxy's own chain:
//!
//! | boundary | what changes |
//! |---|---|
//! | 550.54.04 | the base: the 575-ABI control map (compute-filtered, as the C filtered it) + the 575 class set minus everything added after 550 |
//! | 560.28.03 | +8 alloc classes (`version.go:945-977`) |
//! | 570.86.15 | +6 alloc classes (`version.go:990-1027`) |
//! | 580.65.06 | +2 alloc classes, `NVCEB7`/`NVD1B7` (`version.go:1057-1078`) |
//!
//! ★ **Two limits, stated rather than papered over.**
//!
//! - **The control set is not version-split.** nvproxy's `controlCmd` map also changes at
//!   550.90.07 / 555.42.02 / 565.57.01 / 575.51.02 / 580.65.06, and reproducing that
//!   chain means replaying five more builders for deltas no consumer reads. The one set
//!   here is the 575 set the C shipped. When a consumer needs the split, it is more
//!   `CapabilityTable`s — not a code change.
//! - **There is no deletion.** nvproxy deletes rows at a boundary (575 replaces two
//!   DRAM-encryption commands, `version.go:1039-1042`); this shape can only add. Neither
//!   deleted command is on the C's list, so nothing is wrong today; a boundary that needs
//!   a removal needs a field, and it should be added *with* its first user.
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
    /// list, because as an ioctl it never crossed the boundary the C was gating. Every
    /// row here has a consumer in [`crate::versions::DriverAbiTable::control_params`].
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

/// One driver boundary's capability delta, inheriting everything below it.
///
/// The rows in `controls`/`classes` are the ones **added at this boundary**; a lookup
/// walks `inherits` until something answers. That is nvproxy's own shape, and it means a
/// new driver version is a new `CapabilityTable` and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityTable {
    inherits: Option<&'static CapabilityTable>,
    controls: &'static [ControlEntry],
    classes: &'static [ClassEntry],
    denied_controls: &'static [DeniedEntry],
    denied_classes: &'static [DeniedEntry],
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
    /// This boundary and every one it inherits from, nearest first.
    fn chain(&'static self) -> impl Iterator<Item = &'static CapabilityTable> {
        core::iter::successors(Some(self), |t| t.inherits)
    }

    /// Every permitted control, from every boundary in the chain. Unordered across
    /// boundaries; sorted within one.
    pub fn all_controls(&'static self) -> impl Iterator<Item = &'static ControlEntry> {
        self.chain().flat_map(|t| t.controls.iter())
    }

    /// Every permitted class, from every boundary in the chain.
    pub fn all_classes(&'static self) -> impl Iterator<Item = &'static ClassEntry> {
        self.chain().flat_map(|t| t.classes.iter())
    }

    /// Every explicitly-denied control, from every boundary in the chain.
    pub fn all_denied_controls(&'static self) -> impl Iterator<Item = &'static DeniedEntry> {
        self.chain().flat_map(|t| t.denied_controls.iter())
    }

    /// Every explicitly-denied class, from every boundary in the chain.
    pub fn all_denied_classes(&'static self) -> impl Iterator<Item = &'static DeniedEntry> {
        self.chain().flat_map(|t| t.denied_classes.iter())
    }

    /// May the guest issue this control? **Default-deny.** See the module doc for the
    /// order the four answers are decided in.
    pub fn control(&'static self, cmd: ControlCmd) -> ControlPermit {
        for t in self.chain() {
            if let Some(d) = find_denied(t.denied_controls, cmd.0) {
                return ControlPermit::Denied(Denial::Refused {
                    name: d.name,
                    why: d.why,
                });
            }
        }
        if cmd.0 & RM_GSS_LEGACY_MASK != 0 {
            return ControlPermit::GssLegacyRule;
        }
        if (cmd.0 >> 16) & 0xffff == NV2081_BINAPI_CLASS {
            return ControlPermit::BinApiRule;
        }
        for t in self.chain() {
            if let Some(e) = find_control(t.controls, cmd.0) {
                return ControlPermit::Listed {
                    name: e.name,
                    origin: e.origin,
                };
            }
        }
        ControlPermit::Denied(Denial::NotOnAllowlist)
    }

    /// May the guest allocate this class? **Default-deny.**
    pub fn alloc_class(&'static self, class: ClassId) -> AllocPermit {
        for t in self.chain() {
            if let Some(d) = find_denied(t.denied_classes, class.0) {
                return AllocPermit::Denied(Denial::Refused {
                    name: d.name,
                    why: d.why,
                });
            }
        }
        for t in self.chain() {
            if let Some(e) = find_class(t.classes, class.0) {
                return AllocPermit::Listed {
                    name: e.name,
                    origin: e.origin,
                };
            }
        }
        AllocPermit::Denied(Denial::NotOnAllowlist)
    }
}
pub(crate) static CONTROLS_BASE: &[ControlEntry] = &[
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
    ControlEntry { cmd: 0x00da0006, name: "NV_SEMAPHORE_SURFACE_CTRL_CMD_UNBIND_CHANNEL", origin: Origin::Nvproxy },
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
    ControlEntry { cmd: 0x20800513, name: "NV2080_CTRL_CMD_THERMAL_SYSTEM_EXECUTE_V2", origin: Origin::Nvproxy },
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
    ControlEntry { cmd: 0x20801357, name: "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_INFOROM_SUPPORT_V575", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x20801358, name: "NV2080_CTRL_CMD_FB_QUERY_DRAM_ENCRYPTION_STATUS_V575", origin: Origin::Nvproxy },
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
    ControlEntry { cmd: 0x83de0309, name: "NV83DE_CTRL_CMD_DEBUG_SET_EXCEPTION_MASK", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x83de030c, name: "NV83DE_CTRL_CMD_DEBUG_READ_ALL_SM_ERROR_STATES", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x83de0310, name: "NV83DE_CTRL_CMD_DEBUG_CLEAR_ALL_SM_ERROR_STATES", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x906f0101, name: "NV906F_CTRL_GET_CLASS_ENGINEID", origin: Origin::Nvproxy },
    ControlEntry { cmd: 0x906f0102, name: "NV906F_CTRL_CMD_RESET_CHANNEL", origin: Origin::Nvproxy },
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
    ControlEntry { cmd: 0xcb33010c, name: "NV_CONF_COMPUTE_CTRL_CMD_GPU_GET_KEY_ROTATION_STATE", origin: Origin::Nvproxy },
];

pub(crate) static CLASSES_BASE: &[ClassEntry] = &[
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
        class: 0x000083de,
        name: "GT200_DEBUGGER",
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

pub(crate) static CLASSES_ADDED_560_28_03: &[ClassEntry] = &[
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

pub(crate) static CLASSES_ADDED_570_86_15: &[ClassEntry] = &[
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

pub(crate) static CLASSES_ADDED_580_65_06: &[ClassEntry] = &[
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
/// ★ Every row here is already absent from the allowlist, so none of them changes what a
/// guest can do — what they change is what the refusal *says*, and therefore what a
/// census can distinguish. The C makes the same exclusions implicitly and says so in
/// prose (*"reg-ops/HWPM/debug/fabric/power fall out automatically"*); this is that
/// sentence, in a form a test can bite.
///
/// Sorted by `id` — [`CapabilityTable::control`] binary-searches it.
pub(crate) static DENIED_CONTROLS_BASE: &[DeniedEntry] = &[
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
        id: 0x2080_3083,
        name: "NV2080_CTRL_CMD_NVLINK_GET_PLATFORM_INFO",
        why: DeniedBecause::FabricManagement,
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
pub(crate) static DENIED_CLASSES_BASE: &[DeniedEntry] = &[
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
];

/// The base boundary: nvproxy's 575-ABI surface as the C shipped it, plus the six
/// Mode-2-only controls.
pub static CAPS_BASE: CapabilityTable = CapabilityTable {
    inherits: None,
    controls: CONTROLS_BASE,
    classes: CLASSES_BASE,
    denied_controls: DENIED_CONTROLS_BASE,
    denied_classes: DENIED_CLASSES_BASE,
    note: "the C's ported set: nvproxy 575-ABI control map (compute-filtered) + its \
           575 class set MINUS the classes nvproxy adds after 550.54.04, + the six \
           Mode-2 GSP-RPC controls the ioctl boundary never saw",
};

/// `gvisor/pkg/sentry/devices/nvproxy/version.go:945-977`.
pub static CAPS_560_28_03: CapabilityTable = CapabilityTable {
    inherits: Some(&CAPS_BASE),
    controls: &[],
    classes: CLASSES_ADDED_560_28_03,
    denied_controls: &[],
    denied_classes: &[],
    note: "v560_28_03 adds NVCDB0/NVCDD1/NVCDFA and the first Blackwell channel, \
           copy, graphics, compute and inline-to-memory classes",
};

/// `gvisor/pkg/sentry/devices/nvproxy/version.go:990-1027`.
pub static CAPS_570_86_15: CapabilityTable = CapabilityTable {
    inherits: Some(&CAPS_560_28_03),
    controls: &[],
    classes: CLASSES_ADDED_570_86_15,
    denied_controls: &[],
    denied_classes: &[],
    note: "v570_86_15 adds the Blackwell B channel/copy/graphics/compute pair, \
           BLACKWELL_USERMODE_A and NVCFB7_VIDEO_ENCODER",
};

/// `gvisor/pkg/sentry/devices/nvproxy/version.go:1057-1078`.
///
/// ★ This boundary is the one that makes the version seam **observable**: the C's list
/// is the 575 set, so `NVCEB7`/`NVD1B7` are two classes a 580 guest may allocate and a
/// 580.65.05 guest may not. The two controls nvproxy also adds here
/// (`GPU_GET_SKYLINE_INFO`, `ECC_GET_REPAIR_STATUS`) are `CapGraphics`-only and are
/// therefore **not** carried, exactly as the C's compute filter excluded every other
/// graphics-only row.
pub static CAPS_580_65_06: CapabilityTable = CapabilityTable {
    inherits: Some(&CAPS_570_86_15),
    controls: &[],
    classes: CLASSES_ADDED_580_65_06,
    denied_controls: &[],
    denied_classes: &[],
    note: "v580_65_06 adds NVCEB7_VIDEO_ENCODER and NVD1B7_VIDEO_ENCODER",
};

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

    /// The bench's own driver — the surface every other test here reasons about.
    fn bench() -> &'static CapabilityTable {
        at(580, 159, 4)
    }

    // ── Structure: the properties `binary_search` and the deny-first order need ──────

    /// Every boundary's rows are sorted and duplicate-free, in all four tables.
    ///
    /// ★ Not decoration: [`CapabilityTable::control`] binary-searches, so an unsorted
    /// slice does not fail loudly — it silently *misses* rows, i.e. quietly turns
    /// permitted commands into denials. The generator emits sorted output; this is what
    /// makes a hand-edited insertion in the wrong place a red test instead of a shrug.
    #[test]
    fn every_boundarys_rows_are_sorted_and_unique() {
        for t in [
            &CAPS_BASE,
            &CAPS_560_28_03,
            &CAPS_570_86_15,
            &CAPS_580_65_06,
        ] {
            assert!(
                t.controls.windows(2).all(|w| w[0].cmd < w[1].cmd),
                "controls unsorted/duplicated at {:?}",
                t.note
            );
            assert!(
                t.classes.windows(2).all(|w| w[0].class < w[1].class),
                "classes unsorted/duplicated at {:?}",
                t.note
            );
            assert!(
                t.denied_controls.windows(2).all(|w| w[0].id < w[1].id),
                "denied controls unsorted/duplicated at {:?}",
                t.note
            );
            assert!(
                t.denied_classes.windows(2).all(|w| w[0].id < w[1].id),
                "denied classes unsorted/duplicated at {:?}",
                t.note
            );
        }
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
        static T: CapabilityTable = CapabilityTable {
            inherits: None,
            controls: &[],
            classes: &[],
            denied_controls: DENIED,
            denied_classes: &[],
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
        cls(0x0000_ceb7, "NVCEB7_VIDEO_ENCODER");
        cls(0x0000_d1b7, "NVD1B7_VIDEO_ENCODER");
    }

    /// The reviewed size of the ported surface, per boundary.
    ///
    /// A ratchet, in this repo's idiom: it catches a row leaving as loudly as one
    /// arriving, which the founding-rows pin alone cannot do for the 140-odd rows it
    /// does not name.
    ///
    /// 162 controls = the C's 165 minus the 9 rule-covered rows, plus the 6 Mode-2 rows.
    /// 91 classes at 580 = the C's 89 plus `NVCEB7`/`NVD1B7`, which nvproxy adds at
    /// 580.65.06 and the C's 575-era list therefore could not have.
    #[test]
    fn the_ported_surface_is_the_reviewed_size() {
        assert_eq!(bench().all_controls().count(), 162, "controls");
        assert_eq!(at(550, 54, 4).all_classes().count(), 75, "classes at 550");
        assert_eq!(at(560, 28, 3).all_classes().count(), 83, "classes at 560");
        assert_eq!(at(570, 86, 15).all_classes().count(), 89, "classes at 570");
        assert_eq!(bench().all_classes().count(), 91, "classes at 580");
        assert_eq!(bench().all_denied_controls().count(), 6, "denied controls");
        assert_eq!(bench().all_denied_classes().count(), 2, "denied classes");
    }

    /// The origins are all populated — a `Mode2Rpc` count of zero would mean the
    /// transport delta silently vanished, and the port's most load-bearing finding with
    /// it.
    #[test]
    fn each_origin_is_represented() {
        let n = |o: Origin| bench().all_controls().filter(|e| e.origin == o).count();
        assert_eq!(n(Origin::Mode2Rpc), 6);
        assert_eq!(n(Origin::Empirical), 5);
        assert_eq!(n(Origin::Nvproxy), 151);
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
        // …and inheritance really is inheritance: the base rows survive to the top.
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
        assert_eq!(seen, 9, "the port decodes nine classes today");
        // The sweep must really have covered a class the table refuses, or it proves
        // nothing about the table.
        assert!(
            !abi.capabilities()
                .alloc_class(ClassId(0x0000_0071))
                .is_permitted()
        );
    }

    /// The same derivation for controls, over the class prefixes the table itself
    /// declares — the universe is read out of the data, never hand-written.
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
    /// `SetPageDir` is the one control the port turns into a fact and
    /// `PageDirNotModelled` is the port's most valuable diagnostic; both are only
    /// reachable if the gate lets them through first. This is the test that fails if
    /// someone "tidies up" the Mode-2 rows out of the table because the C did not have
    /// them.
    #[test]
    fn the_page_directory_controls_survive_the_gate() {
        let abi = table_for(crate::versions::BENCH_DRIVER).expect("bench");
        for (cmd, want) in [
            (0x0080_1813u32, ControlParams::SetPageDir),
            (0x0080_1814, ControlParams::PageDirNotModelled),
            (0x2080_0a9f, ControlParams::PageDirNotModelled),
            (0x90f1_0106, ControlParams::PageDirNotModelled),
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
