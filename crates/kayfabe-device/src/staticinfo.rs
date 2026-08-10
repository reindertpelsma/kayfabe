//! The command policy that answers `GET_GSP_STATIC_INFO` (fn 65) from a chip row.
//!
//! ## ★★ Why this is here and not in a logic crate
//!
//! Same join as [`crate::inittables`]: the *rows* are a fact about a board
//! ([`crate::ChipProfile::fb_regions`], [`crate::ChipProfile::fb_length`]) and the
//! *layout* is the Axis-A quarantine (`kayfabe_abi::gspstaticinfo`). This crate is where
//! a concrete chip's facts are allowed to meet a wire. Nothing below names a generation,
//! a driver version or an address — a second chip is a second row.
//!
//! ## ★★★ Fn 65 has nowhere to put a refusal, so the refusal is the envelope
//!
//! A `GSP_RM_CONTROL` carries a `status` field, so a policy that cannot serve a command
//! can say so *inside* the reply. Fn 65 carries no such field: the guest copies the body
//! into `pGpu->pGspStaticInfo` and reads it (`ogkm-580:
//! src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:4232-4236`). The only place left to refuse
//! is the RPC envelope's `rpc_result`, which the guest checks first and which makes
//! `NV_RM_RPC_GET_GSP_STATIC_INFO` fail with a line that names itself:
//!
//! ```text
//! NVRM: GET_GSP_STATIC_INFO failed: 0x<status>
//! ```
//!
//! That is strictly better than a well-formed body that is wrong, and it is what an
//! unencodable row gets. ⊘ It is *not* what an unknown function gets — those fall through
//! to the chain, which is a different statement.
//!
//! ## ★★ Fn 65's accepted reply is 1792 bytes the guest keeps — but not in the CACHE
//!
//! The guest copies this body into `pGpu->pGspStaticInfo` and reads it for the rest of the
//! boot, which is a form of stickiness this module already argues about above. It is **not**
//! the `rmapiControlCache` kind, and the two must not be conflated: the cache is populated
//! from an RPC reply at exactly two call sites in the whole driver —
//! `ogkm-580: src/nvidia/src/kernel/vgpu/rpc.c:11097` and `:11102` (`ogkm-610: :10902`,
//! `:10907`) — both inside `rpcRmApiControl_GSP`, reachable only for
//! `NV_VGPU_MSG_FUNCTION_GSP_RM_CONTROL`. Fn 65 is not that function. ⇒ a wrong static-info
//! reply dies with the driver life that read it, where a cached control answer would not
//! even be re-asked. [`crate::sticky`] §1a derives the call-site universe by grep;
//! `tests/tests/sticky_answer.rs` executes the claim against this type.

use kayfabe_abi::NV_ERR_NOT_SUPPORTED;
use kayfabe_abi::gspstaticinfo::{
    GSP_STATIC_CONFIG_INFO_SIZE, GpuGid, GpuName, GspStaticInfo, encode_gsp_static_info,
};
use kayfabe_abi::versions::DriverAbiTable;
use kayfabe_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction};

use crate::ChipProfile;

/// `NV_OK`.
const NV_OK: u32 = 0;

/// ★★★ **The model names a VMM hands this device** — the value that decides whether
/// `nvidia-smi` prints `NVIDIA GeForce RTX 3060` or `ERR!` in its Name column.
///
/// # Why a struct, and why both halves are optional and independent
///
/// The two strings have **different availability**, and collapsing them into one required
/// pair would force an operator who can supply one to invent the other.
/// `NV2080_CTRL_CMD_GPU_GET_NAME_STRING` (`0x20800110`) is what every user-facing tool reads
/// and is trivially obtainable from a host that has the card; `GET_SHORT_NAME_STRING`
/// (`0x20800111`, `"GA106-A"` on the reference part) is surfaced by nothing an operator
/// normally runs. ⇒ `Default` is *both absent*, which is exactly this port's behaviour before
/// this type existed, and each is filled in only when a source exists.
///
/// ⊘ **Absence stays absence.** There is deliberately no fallback to a chip-row constant. The
/// owner's standing **READ-NATIVE, WRITE-TRAP** ruling makes the model name a *read* whose
/// source is the host GPU's own answer, and `kayfabe_abi::gspstaticinfo::GpuName`'s own docs
/// spell out why a per-generation table is the thing that ruling exists to prevent: a port
/// that asks the host supports whatever card is in the box; a port with a table supports the
/// cards somebody remembered. A `None` here leaves the array zero and the guest says so —
/// `c_oracle_empty_rows_are_wrong`, an absent measurement must not decode as a value.
///
/// ★ **Why a VMM-supplied value is still READ-NATIVE and not a hardcode.** Nothing on the
/// fn-65 path can issue a host ioctl — fn 65 is the second RPC of the driver's whole life and
/// the isolate that carries host verbs is materialized by the first accepted guest RM event,
/// strictly later (see [`StaticInfoPolicy::with_name`]). The value must already be in hand
/// when the policy is built, so the *composition root* is the only place it can enter. This
/// type is that seam. What travels through it is a fact about the host's card, not a fact this
/// port made up; the `gpu-name` device property is where it is stated, so the boot's own
/// command line records which name the guest was told.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GpuNames {
    /// `gpuNameString` — the marketing model string, e.g. `"NVIDIA GeForce RTX 3060"`.
    pub name: Option<GpuName>,
    /// `gpuShortNameString` — the die string, e.g. `"GA106-A"`.
    pub short_name: Option<GpuName>,
}

/// Answers `GET_GSP_STATIC_INFO` with the static facts a chip row states.
///
/// Every other command gets `None`, i.e. the next link in the chain, or the FSM's own
/// acknowledgement.
#[derive(Debug, Clone, Copy)]
pub struct StaticInfoPolicy {
    chip: &'static ChipProfile,
    driver: DriverAbiTable,
    gid: GpuGid,
    name: Option<GpuName>,
    short_name: Option<GpuName>,
}

impl StaticInfoPolicy {
    /// Bind the policy to a chip row and a guest driver version, with the UUID this
    /// device declares derived from the chip row.
    ///
    /// ⚠ **Per chip row, therefore not per board.** See [`GpuGid`]: two nvkvm GPUs built
    /// from the same row would declare the same UUID, and the answer is a VMM-declared
    /// value through [`StaticInfoPolicy::with_gid`]. The derivation is here rather than a
    /// constant so that the value is at least *stable* — a guest that pinned
    /// `GPU-<uuid>` finds the same device after a reboot — and so that adding a second
    /// chip row cannot silently produce a second GPU with the first one's identity.
    #[must_use]
    pub fn new(chip: &'static ChipProfile, driver: DriverAbiTable) -> StaticInfoPolicy {
        StaticInfoPolicy::with_gid(chip, driver, Self::gid_for_chip(chip))
    }

    /// Bind the policy with an explicitly declared UUID — the door a VMM-supplied
    /// `gpu-uuid` comes through.
    #[must_use]
    pub fn with_gid(
        chip: &'static ChipProfile,
        driver: DriverAbiTable,
        gid: GpuGid,
    ) -> StaticInfoPolicy {
        StaticInfoPolicy {
            chip,
            driver,
            gid,
            name: None,
            short_name: None,
        }
    }

    /// ★★★ **The door the model name comes through** — and it is deliberately the only
    /// one, with no default behind it.
    ///
    /// ⊘ There is no `ChipProfile::name_string`, on purpose. A per-generation constant is
    /// a row somebody must add for every GPU that will ever exist, and it contradicts the
    /// owner's standing **READ-NATIVE, WRITE-TRAP** ruling (2026-07-31): the model name is
    /// a *read*, and a read should be the **host GPU's own answer**
    /// (`NV2080_CTRL_CMD_GPU_GET_NAME_STRING`, `0x20800110`, already on the capability
    /// allowlist). A port that asks the host supports whatever card is in the box; a port
    /// with a table supports the cards somebody remembered.
    ///
    /// ⚠ **The lifetime, stated because it constrains who may call this.** Fn 65 is the
    /// **second RPC of the entire driver life** (`ogkm-580:
    /// src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:4232`, inside `kgspInitRm`, called from
    /// `arch/nvalloc/unix/src/osinit.c:2024`, immediately after `SET_GUEST_SYSTEM_INFO`),
    /// so **no guest RM object exists yet** — while the only device-level host-verb
    /// carrier this port has, the system proc's isolate, is materialized by the *first
    /// accepted guest RM event* (`tests/tests/isolate_spawn_is_guest_caused.rs`, E0b),
    /// which is strictly later. ⇒ nothing inside the fn-65 path can issue a host ioctl,
    /// and the value must already be in hand when the policy is built.
    ///
    /// ★ **Which is why the answer is not "query or fail".** Owner ruling, 2026-08-08:
    /// fill in every field obtainable by unprivileged ioctl, **derive** the rest on the
    /// fly for any architecture, and prove the derived structure by compiling the
    /// driver's own parser as an oracle — the pattern `#116` already runs for the VBIOS.
    /// A derived value stops being a guess the moment RM's own code accepts it. This
    /// setter is the seam both halves arrive through.
    ///
    /// ⊘ Absence is [`None`], never a stand-in: `c_oracle_empty_rows_are_wrong`.
    #[must_use]
    pub fn with_name(mut self, name: GpuName, short_name: GpuName) -> StaticInfoPolicy {
        self.name = Some(name);
        self.short_name = Some(short_name);
        self
    }

    /// [`StaticInfoPolicy::with_name`] for the case where the two strings arrive
    /// **independently** — which is the production case.
    ///
    /// ★ The pair-taking setter above is right for a caller that has both, which is every
    /// test. It is wrong for the composition root: `[measured 2026-08-10, nvdiff host
    /// reference]` the long name is one `nvidia-smi` query away on any host with the card,
    /// while the short name (`"GA106-A"`) is surfaced by nothing an operator runs — so
    /// demanding both would make an operator who has one **invent** the other, which is the
    /// exact failure `c_oracle_empty_rows_are_wrong` records. See [`GpuNames`].
    ///
    /// ⊘ Passing [`GpuNames::default`] is *not* a no-op dressed as configuration: it is the
    /// positive statement *"this VMM was told no model name"*, and it leaves the arrays zero,
    /// which is what the guest then reports.
    #[must_use]
    pub fn with_names(mut self, names: GpuNames) -> StaticInfoPolicy {
        self.name = names.name;
        self.short_name = names.short_name;
        self
    }

    /// The UUID [`StaticInfoPolicy::new`] derives, exposed so a test can state the
    /// default without restating the derivation.
    #[must_use]
    pub fn gid_for_chip(chip: &ChipProfile) -> GpuGid {
        // ★ The chip's PCI identity, in a fixed order. Not `chip.name`: a diagnostic
        // string is the one field this row documents as never branched on, and an
        // identity derived from it would change if somebody fixed a typo.
        let mut seed = [0u8; 8];
        seed[0..2].copy_from_slice(&chip.pci_device_id.to_le_bytes());
        seed[2..4].copy_from_slice(&chip.pci_subsystem_vendor_id.to_le_bytes());
        seed[4..6].copy_from_slice(&chip.pci_subsystem_id.to_le_bytes());
        seed[6] = chip.pci_revision;
        GpuGid::derive(&seed)
    }

    /// The body this policy would post, or the reason it cannot.
    ///
    /// Exposed so a test can ask the encoding question without building a wire message —
    /// and so the refusal path has a name that is not "some `Reply` with a non-zero
    /// status".
    ///
    /// # Errors
    ///
    /// Whatever `encode_gsp_static_info` refuses: a region table that contradicts itself
    /// or the chip's own `fb_length`, or a driver version whose struct shape this port
    /// does not encode.
    pub fn body(&self) -> Result<Vec<u8>, kayfabe_abi::gspstaticinfo::GspStaticInfoError> {
        encode_gsp_static_info(
            &GspStaticInfo {
                fb_regions: self.chip.fb_regions,
                fb_length: self.chip.fb_length,
                gid: self.gid,
                // ⊘ `None` until a source exists — see `StaticInfoPolicy::with_name`
                // and `kayfabe_abi::gspstaticinfo::GpuName`. This port has not been told
                // the model name, so it writes zero and the guest says so.
                name: self.name,
                short_name: self.short_name,
                // ★★★★ The chip's own statement of where BAR1's page directory goes. Taken
                // from the SAME row `crate::plane::RegPlane::bar1_phys` walks from, so the
                // address we tell the guest and the address we read back cannot drift —
                // that drift is unobservable by construction, because a wrong root reads as
                // an unmapped virtual address rather than as a mismatch.
                bar1_pde_base: self.chip.bar1_pde_base,
            },
            self.driver.gsp_static_info_wire(),
        )
    }
}

impl CommandPolicy for StaticInfoPolicy {
    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {
        if cmd.function != RpcFunction::GetGspStaticInfo {
            return None;
        }
        // ★★★ **The size check, and the reason it is not optional.**
        //
        // Every `InitTablePolicy` control refuses on `req.params_size != want.params_size()`
        // — the guest's own declared size against ours. Fn 65 carries no `paramsSize` field
        // to compare, so the equivalent statement is the *body length* the guest's envelope
        // declares, and this policy compared nothing at all.
        //
        // ⚠ That was silent in the worst possible way. `RpcCommand::reply` clamps the body
        // to whatever the request declared, so a disagreement produces neither a fault nor a
        // counter nor a refusal — only a `GspStaticConfigInfo` that is **truncated** (guest
        // struct smaller) or **zero-padded** (guest struct larger), copied wholesale into
        // `pGpu->pGspStaticInfo` and read from there for the rest of the boot
        // (`ogkm-580: src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:4232-4236`). Nothing logs.
        //
        // ★★ And there is **zero margin**: `1824 - 32 == 1792` exactly. This port's
        // `GSP_STATIC_CONFIG_INFO_SIZE` and the guest's `sizeof(GspStaticConfigInfo)` are
        // *the same fact stated twice* — by two teams, from two sources, keyed on a driver
        // version. When they disagree the whole reply is wrong, and the disagreement is
        // exactly what a differently-built guest brings. So the refusal goes in the
        // envelope, where the guest's own `NV_RM_RPC_GET_GSP_STATIC_INFO` fails loudly with
        // a line that names itself, rather than into a body that cannot carry one.
        if cmd.payload.len() != GSP_STATIC_CONFIG_INFO_SIZE {
            return Some(Reply {
                rpc_result: NV_ERR_NOT_SUPPORTED,
                body: Vec::new(),
            });
        }
        match self.body() {
            Ok(body) => Some(Reply {
                rpc_result: NV_OK,
                body,
            }),
            Err(_) => Some(Reply {
                rpc_result: NV_ERR_NOT_SUPPORTED,
                body: Vec::new(),
            }),
        }
    }
}

kayfabe_util::assert_send_sync!(StaticInfoPolicy);
