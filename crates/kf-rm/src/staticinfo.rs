//! The command policy that answers `GET_GSP_STATIC_INFO` (fn 65) from a chip row.
//!
//! ## ★★ Why this is here and not in a logic crate
//!
//! Same join as [`crate::inittables`]: the *rows* are a fact about a board
//! ([`crate::ChipProfile::fb_regions`], [`crate::ChipProfile::fb_length`]) and the
//! *layout* is the Axis-A quarantine (`kf_abi::gspstaticinfo`). This crate is where
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

use kf_abi::NV_ERR_NOT_SUPPORTED;
use kf_abi::gspstaticinfo::{
    FbRegion, GSP_STATIC_CONFIG_INFO_SIZE, GpuGid, GpuName, GspStaticInfo, encode_gsp_static_info,
};
use kf_abi::versions::DriverAbiTable;
use kf_gsp::{CommandPolicy, Reply, RpcCommand, RpcFunction, StashedSystemInfo, SystemInfoCell};

use crate::BoardFacts;
use std::sync::Arc;

/// `NV_OK`.
const NV_OK: u32 = 0;

/// ★★ **The boot display's seat in fn 65** (`docs/design/V3_DISPLAY.md` §4.11.4 rows 1–2).
///
/// When the guest's CPU-RM preserves a firmware console of `C` bytes — the GOP framebuffer kf3's
/// option ROM put at BAR1 offset 0 — it describes ALL of `fbRegion[0]` as that console and maps it
/// at BAR1 VA 0 (`ogkm-580: src/nvidia/src/kernel/gpu/mem_mgr/arch/maxwell/mem_mgr_gm107.c:2068-2110`,
/// `src/nvidia/src/kernel/gpu/bus/arch/maxwell/kern_bus_gm107.c:1084-1162`). Region 0 must then BE
/// `[0, C)` (`kf_chip::bar0::fb_layout_with_console`). `C` reaches us only in fn 72's
/// `GspSystemInfo.consoleMemSize` (`src/nvidia/src/kernel/vgpu/rpc.c:10585`), which the GSP state
/// machine keeps unanswered in [`SystemInfoCell`] (`kf_gsp::sysinfo`); it is decoded HERE, at fn 65,
/// with the table that serves fn 65 — fn 72 precedes the fn-1 re-select ([`crate::ReselectAtFn1`]).
#[derive(Debug, Clone)]
pub struct ConsoleSeat {
    /// Fn 72's body, kept by the state machine — ONE cell across every chain rebuild.
    pub system_info: SystemInfoCell,
    /// The guest's BAR1 aperture (`bar1-size`): the console is mapped at VA 0 there, so `C` must fit.
    pub bar1_bytes: u64,
    /// The boot framebuffer's size `G` when kf3 serves its option ROM (`gop=on`). `None`: `C` is
    /// read and logged, and today's table is served whatever it says.
    /// ⊘ CORRECTED 2026-10-03 (`v3-gop`): `None` no longer describes kf3's `gop=off`. kf3 builds a
    /// seat only with `gop=on` (`kf_qemu::gop::ConsoleWiring`); with `gop=off` the policy has no seat
    /// at all, which is the posture before the boot display existed. `None` stays a supported seat
    /// (read and log only), pinned byte-identical by `tests/console_region.rs`.
    pub boot_fb: Option<u64>,
}

/// Why fn 65 cannot serve the guest's preserved console (refused by name in the envelope).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleRefusal {
    /// The serving version's `GspSystemInfo` is not in the driver matrix, or has no `consoleMemSize`.
    Unmeasured {
        /// The serving version.
        version: kf_abi::DriverVersion,
        /// What the matrix said.
        why: String,
    },
    /// The guest's fn 72 body is not the serving version's `GspSystemInfo` — its fields would be
    /// read at another version's offsets.
    WrongSize {
        /// The serving version.
        version: kf_abi::DriverVersion,
        /// The length the guest's envelope declared.
        declared: usize,
        /// `sizeof(GspSystemInfo)` in the driver matrix at the serving version.
        matrix: usize,
    },
    /// `C` does not fit the guest's BAR1, where CPU-RM maps it at VA 0.
    LargerThanBar1 {
        /// The guest's `consoleMemSize`.
        console: u64,
        /// The BAR1 aperture.
        bar1_bytes: u64,
    },
    /// kf-chip refused the region table ([`kf_chip::bar0::ConsoleRefused`]).
    Layout(kf_chip::bar0::ConsoleRefused),
    /// The board's own table is not kf-chip's layout (a captured or hand-written row), so no
    /// console can be carved out of it.
    NotOurLayout,
}

impl core::fmt::Display for ConsoleRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ConsoleRefusal::Unmeasured { version, why } => write!(
                f,
                "GspSystemInfo.consoleMemSize is not in the driver matrix at driver {version}: {why}"
            ),
            ConsoleRefusal::WrongSize {
                version,
                declared,
                matrix,
            } => write!(
                f,
                "fn 72's GspSystemInfo is {declared} bytes, the driver matrix's layout for driver {version} \
                 is {matrix} (the guest is not the version this chain serves)"
            ),
            ConsoleRefusal::LargerThanBar1 {
                console,
                bar1_bytes,
            } => write!(
                f,
                "consoleMemSize {console:#x} does not fit the {bar1_bytes:#x}-byte BAR1 it is mapped \
                 into at VA 0"
            ),
            ConsoleRefusal::Layout(e) => write!(f, "{e}"),
            ConsoleRefusal::NotOurLayout => write!(
                f,
                "the board's region table is not kf-chip's layout; no console region can be carved"
            ),
        }
    }
}

/// ★ `GspSystemInfo.consoleMemSize` out of a kept fn 72, at `version`'s own layout in the driver matrix
/// (`kf_abi::generated::matrix::GSPSYSTEMINFO`: offset 48, 56 or 64 by version).
///
/// # Errors
/// [`ConsoleRefusal::Unmeasured`] or [`ConsoleRefusal::WrongSize`], by name — never a value read at
/// a borrowed offset.
pub fn console_mem_size(
    version: kf_abi::DriverVersion,
    fn72: &StashedSystemInfo,
) -> Result<u64, ConsoleRefusal> {
    let unmeasured = |why: String| ConsoleRefusal::Unmeasured { version, why };
    let layout = kf_abi::matrix::Resolved::of(&kf_abi::generated::matrix::GSPSYSTEMINFO, version)
        .map_err(|e| unmeasured(e.to_string()))?;
    if fn72.declared_len != layout.size() {
        return Err(ConsoleRefusal::WrongSize {
            version,
            declared: fn72.declared_len,
            matrix: layout.size(),
        });
    }
    let field = layout
        .need("consoleMemSize")
        .map_err(|e| unmeasured(e.to_string()))?
        .range()
        .filter(|r| r.len() == 8)
        .ok_or_else(|| unmeasured("consoleMemSize is not 8 bytes in the matrix's layout".into()))?;
    let bytes: [u8; 8] = fn72
        .bytes
        .get(field)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| unmeasured("the kept body is shorter than the field".into()))?;
    Ok(u64::from_le_bytes(bytes))
}

/// Answers `GET_GSP_STATIC_INFO` with the static facts a chip row states.
///
/// Every other command gets `None`, i.e. the next link in the chain, or the FSM's own
/// acknowledgement.
#[derive(Debug, Clone)]
pub struct StaticInfoPolicy {
    board: Arc<BoardFacts>,
    driver: DriverAbiTable,
    gid: GpuGid,
    name: Option<GpuName>,
    short_name: Option<GpuName>,
    engine_caps: [u32; kf_abi::gspstaticinfo::ENGINE_CAPS_WORDS],
    /// ★ The boot display's console ([`ConsoleSeat`]); `None` serves the board's table, as before.
    console: Option<ConsoleSeat>,
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
    pub fn new(board: Arc<BoardFacts>, driver: DriverAbiTable) -> StaticInfoPolicy {
        let gid = Self::gid_for_board(&board);
        StaticInfoPolicy::with_gid(board, driver, gid)
    }

    /// Bind the policy with an explicitly declared UUID — the door a VMM-supplied
    /// `gpu-uuid` comes through.
    #[must_use]
    pub fn with_gid(
        board: Arc<BoardFacts>,
        driver: DriverAbiTable,
        gid: GpuGid,
    ) -> StaticInfoPolicy {
        StaticInfoPolicy {
            board,
            driver,
            gid,
            name: None,
            short_name: None,
            engine_caps: [0; kf_abi::gspstaticinfo::ENGINE_CAPS_WORDS],
            console: None,
        }
    }

    /// ★ Seat the boot display's console ([`ConsoleSeat`]): fn 65's region table then follows the
    /// guest's `consoleMemSize` (`gop=on`), or only logs it (`gop=off`).
    #[must_use]
    pub fn with_console(mut self, seat: ConsoleSeat) -> StaticInfoPolicy {
        self.console = Some(seat);
        self
    }

    /// ★★ The region table fn 65 serves NOW: the board's, unless kf3 serves the boot display
    /// (`gop=on`) and the guest's kept fn 72 preserves a console of `C > 0` bytes — then kf-chip's
    /// table with `[0, C)` as region 0 ([`kf_chip::bar0::fb_layout_with_console`]).
    ///
    /// ⊘ With no seat, no fn 72 kept, `C = 0`, or `gop=off`, the board's table — byte for byte what
    /// this policy served before the boot display existed.
    ///
    /// # Errors
    /// [`ConsoleRefusal`], by name (`gop=on` only: with `gop=off` nothing here can refuse).
    pub fn fb_regions_now(&self) -> Result<Vec<FbRegion>, ConsoleRefusal> {
        let today = || self.board.fb_regions.clone();
        let Some(seat) = &self.console else {
            return Ok(today());
        };
        let Some(fn72) = seat.system_info.latest() else {
            return Ok(today());
        };
        let version = self.driver.driver_version();
        let console = match console_mem_size(version, &fn72) {
            Ok(c) => c,
            Err(e) if seat.boot_fb.is_none() => {
                eprintln!(
                    "kf-rm: GET_GSP_STATIC_INFO (gop=off): consoleMemSize not read ({e}); the board's \
                     region table is served"
                );
                return Ok(today());
            }
            Err(e) => return Err(e),
        };
        if console == 0 {
            return Ok(today());
        }
        let Some(g) = seat.boot_fb else {
            eprintln!(
                "kf-rm: GET_GSP_STATIC_INFO (gop=off): the guest preserves a firmware console of \
                 {console:#x} bytes (fn 72 seq {}), but kf3 serves no boot display; the board's region \
                 table is served, as before",
                fn72.sequence
            );
            return Ok(today());
        };
        if console > seat.bar1_bytes {
            return Err(ConsoleRefusal::LargerThanBar1 {
                console,
                bar1_bytes: seat.bar1_bytes,
            });
        }
        let fb_length = self.board.fb_length;
        let carved = kf_chip::bar0::fb_layout_with_console(fb_length, console)
            .map_err(ConsoleRefusal::Layout)?;
        // ⊘ Only a board whose table IS kf-chip's layout can be carved: kf3 builds it from
        // `fb_layout`, a captured row was never ours to reshape.
        let ours = kf_chip::bar0::fb_layout(fb_length).is_some_and(|l| {
            l.regions == self.board.fb_regions
                && l.bar1_pde_base == self.board.bar1_pde_base
                && l.bar2_pde_base == self.board.bar2_pde_base
        });
        if !ours {
            return Err(ConsoleRefusal::NotOurLayout);
        }
        eprintln!(
            "kf-rm: GET_GSP_STATIC_INFO: the guest preserves a firmware console of {console:#x} bytes \
             (fn 72 seq {}; boot framebuffer G = {g:#x}{}): region 0 = [0, {console:#x}) reserved, \
             the heap starts at {console:#x}, {} regions",
            fn72.sequence,
            if console == g { "" } else { ", C != G" },
            carved.regions.len()
        );
        Ok(carved.regions)
    }

    /// ★ `engineCaps[]` — the NV2080-indexed engine bitmask, from the SAME engine table the FIFO
    /// device-info reply is built from ([`crate::authored::engine_caps`]), so the two statements
    /// of "which engines exist" cannot disagree.
    #[must_use]
    pub fn with_engine_caps(
        mut self,
        caps: [u32; kf_abi::gspstaticinfo::ENGINE_CAPS_WORDS],
    ) -> StaticInfoPolicy {
        self.engine_caps = caps;
        self
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

    /// The UUID [`StaticInfoPolicy::new`] derives, exposed so a test can state the
    /// default without restating the derivation.
    #[must_use]
    pub fn gid_for_board(chip: &BoardFacts) -> GpuGid {
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
    pub fn body(&self) -> Result<Vec<u8>, kf_abi::gspstaticinfo::GspStaticInfoError> {
        encode_gsp_static_info(
            &GspStaticInfo {
                fb_regions: &self.board.fb_regions,
                fb_length: self.board.fb_length,
                gid: self.gid,
                // ⊘ `None` until a source exists — see `StaticInfoPolicy::with_name`
                // and `kf_abi::gspstaticinfo::GpuName`. This port has not been told
                // the model name, so it writes zero and the guest says so.
                name: self.name,
                short_name: self.short_name,
                // ★★★★ The chip's own statement of where BAR1's page directory goes. Taken
                // from the SAME row `crate::plane::RegPlane::bar1_phys` walks from, so the
                // address we tell the guest and the address we read back cannot drift —
                // that drift is unobservable by construction, because a wrong root reads as
                // an unmapped virtual address rather than as a mismatch.
                bar1_pde_base: self.board.bar1_pde_base,
                // ★★★★ P4: OUR BAR2 root — the page every BAR2 invalidate will name, and the
                // one fn 70 writes the guest's `PDE3[0]` into.
                bar2_pde_base: self.board.bar2_pde_base,
                engine_caps: self.engine_caps,
            },
            self.driver.gsp_static_info_wire(),
        )
    }

    /// ★★★ The reply body at the GUEST's MEASURED `GspStaticConfigInfo` layout
    /// (`kf_abi::gspstaticinfo::encode_gsp_static_info_at`, `V3_DRIVER_MATRIX.md` §4) — the
    /// same facts as [`Self::body`], byte-identical to it at every 580.x tag, and placed at
    /// the version's own offsets everywhere else (1656 bytes at 570/575, 1600 at 610, …).
    ///
    /// # Errors
    /// The encoder's refusals; an unmeasured version or a version without the struct refuses
    /// as `UnsupportedWire` of the table's wire.
    pub fn body_measured(&self) -> Result<Vec<u8>, kf_abi::gspstaticinfo::GspStaticInfoError> {
        self.body_measured_with(&self.board.fb_regions)
    }

    /// [`Self::body_measured`] with `fb_regions` in place of the board's own table — the table
    /// [`Self::fb_regions_now`] chose.
    ///
    /// # Errors
    /// As [`Self::body_measured`].
    pub fn body_measured_with(
        &self,
        fb_regions: &[FbRegion],
    ) -> Result<Vec<u8>, kf_abi::gspstaticinfo::GspStaticInfoError> {
        let lay = kf_abi::matrix::Resolved::of(
            &kf_abi::generated::matrix::GSPSTATICCONFIGINFO,
            self.driver.driver_version(),
        )
        .map_err(
            |_| kf_abi::gspstaticinfo::GspStaticInfoError::UnsupportedWire {
                wire: self.driver.gsp_static_info_wire(),
            },
        )?;
        kf_abi::gspstaticinfo::encode_gsp_static_info_at(
            &GspStaticInfo {
                fb_regions,
                fb_length: self.board.fb_length,
                gid: self.gid,
                name: self.name,
                short_name: self.short_name,
                bar1_pde_base: self.board.bar1_pde_base,
                bar2_pde_base: self.board.bar2_pde_base,
                engine_caps: self.engine_caps,
            },
            &lay,
        )
    }

    /// `sizeof(GspStaticConfigInfo)` at the guest's version (measured), or the bench size where
    /// the version was never measured (then `body_measured` refuses anyway).
    fn guest_struct_size(&self) -> usize {
        kf_abi::matrix::Resolved::of(
            &kf_abi::generated::matrix::GSPSTATICCONFIGINFO,
            self.driver.driver_version(),
        )
        .map_or(GSP_STATIC_CONFIG_INFO_SIZE, |l| l.size())
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
        // ⊘⊘ CORRECTED 2026-09-26 (`V3_DRIVER_MATRIX.md` §4): the size compared against is the
        // guest version's MEASURED `sizeof(GspStaticConfigInfo)`, and the body is encoded at
        // that version's layout — no longer the 1792 bytes of 580.x for every guest.
        let want = self.guest_struct_size();
        if cmd.payload.len() != want {
            // ★ Named: the guest's own struct and the measured one for its declared version
            // disagree, i.e. the declared `guest-driver=` is not what the guest is.
            eprintln!(
                "kf-rm: GET_GSP_STATIC_INFO refused: the guest's GspStaticConfigInfo is {} bytes, \
                 the measured layout for driver {} is {}",
                cmd.payload.len(),
                self.driver.driver_version(),
                want
            );
            return Some(Reply {
                rpc_result: NV_ERR_NOT_SUPPORTED,
                body: Vec::new(),
            });
        }
        // ★ The boot display: the table follows the guest's preserved console (`gop=on`).
        let regions = match self.fb_regions_now() {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "kf-rm: GET_GSP_STATIC_INFO refused: the guest's firmware console cannot be \
                     served: {e} (guest driver {})",
                    self.driver.driver_version()
                );
                return Some(Reply {
                    rpc_result: NV_ERR_NOT_SUPPORTED,
                    body: Vec::new(),
                });
            }
        };
        match self.body_measured_with(&regions) {
            Ok(body) => Some(Reply {
                rpc_result: NV_OK,
                body,
            }),
            Err(e) => {
                eprintln!(
                    "kf-rm: GET_GSP_STATIC_INFO refused: {e:?} (guest driver {})",
                    self.driver.driver_version()
                );
                Some(Reply {
                    rpc_result: NV_ERR_NOT_SUPPORTED,
                    body: Vec::new(),
                })
            }
        }
    }
}

kf_util::assert_send_sync!(StaticInfoPolicy);
