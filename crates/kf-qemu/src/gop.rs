// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **The boot display, kf3's side** (`docs/design/V3_DISPLAY.md` §4.11, property `gop`, default
//! off).
//!
//! kf3 shows a picture before the guest's NVIDIA driver loads the way a real card does: its PCI
//! expansion ROM carries a UEFI GOP driver whose framebuffer is a BAR1 range. Here that ROM is the
//! constant `kf-gop` driver embedded in kayfabe (`kf_gop_image::KF_GOP_EFI`, built from
//! `firmware/kf-gop` during the build — owner ruling `OWNER_RULINGS.md` §K) wrapped, per device,
//! with the PCI ids and class kf3 already presents and a
//! `KFGP` descriptor naming the framebuffer: **BAR1, offset 0, G bytes** of the virtual monitor's
//! preferred mode (`kf_rm::display::monitors`, so the firmware mode is the native one).
//!
//! What `gop=on` changes, and where (each is `None`/absent with `gop=off`, so every path is today's):
//! - the ROM BAR: [`BootPlan::rom`] at realize, served by `kf3_option_rom` (KF3 ABI 11) and
//!   registered by the C device in the shape of QEMU's `pci_add_option_rom`;
//! - store `[0, G)`: zeroed by the GPU walker at realize, then shown at BAR1 offset 0 by ONE store
//!   view — the seed (`kf_mem::cpuwin::CpuWindow::seed`), retired at the first BAR1 change;
//! - the display worker: the boot layer ([`BootPlan::surface`] → `kf_disp::scanout::boot_layer`)
//!   until the guest arms a head;
//! - fn 72 and fn 65: ONE [`ConsoleWiring`] decides both — with `gop=on` the GSP state machine keeps
//!   fn 72's body and fn 65 serves the guest's preserved console as region 0
//!   (`kf_rm::staticinfo::ConsoleSeat`); with `gop=off` there is no cell and no seat at all;
//! - the host BAR1 budget: + G for the seed and the guest's console view overlapping while the seed
//!   retires (`crate::cardbudget::Demand::with_boot_fb`).
//!
//! ⊘ Nothing here traps, and nothing reads guest vidmem on the CPU: the ROM BAR is read-only RAM, the
//! seed is a placement inside the existing BAR1 memslot, and the zeroing and the scanout are GPU work.

use kf_oprom::{BootFramebuffer, ClassCode, Geometry, Identity};
use kf_rm::staticinfo::ConsoleSeat;

/// The PCI BAR the framebuffer is in — BAR1, where CPU-RM looks for a console to preserve
/// (`ogkm-580: src/nvidia/arch/nvalloc/unix/src/osinit.c:1073-1079`).
pub const FB_BAR: u8 = 1;
/// Its offset in the BAR: 0, where CPU-RM maps a preserved console (`kern_bus_gm107.c:1131-1153`) and
/// where FB physical 0 — store offset 0 — sits before BAR1 goes virtual.
pub const FB_OFFSET: u64 = 0;

/// ★ What `gop=on` decided at realize: the framebuffer and the monitor's EDID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootPlan {
    /// BAR1, offset 0, and the mode's geometry (pitch rounded to 256 bytes, G to 64 KiB).
    pub fb: BootFramebuffer,
    /// The virtual monitor's EDID (the same bytes NVKMS reads later).
    pub edid: Vec<u8>,
}

impl BootPlan {
    /// ★ The plan for a device's configuration: `None` with `gop=off` (today's device, unchanged),
    /// or the boot framebuffer for `monitor`'s preferred mode.
    ///
    /// # Errors
    /// By name, before anything is reserved: `gop=on` without `display=on` (the boot display is the
    /// virtual NVDisplay's first picture, and a displayless device has no monitor to show it on); a
    /// mode the ROM cannot describe; a framebuffer larger than the guest's BAR1.
    pub fn for_config(
        gop: bool,
        display: bool,
        bar1_bytes: u64,
        monitor: &kf_disp::edid::Monitor,
    ) -> Result<Option<BootPlan>, String> {
        if !gop {
            return Ok(None);
        }
        if !display {
            return Err(
                "gop=on needs display=on: the boot display is the virtual NVDisplay's first picture \
                 (docs/design/V3_DISPLAY.md §4.11)"
                    .into(),
            );
        }
        let (w, h) = (monitor.preferred.h_active, monitor.preferred.v_active);
        let geometry = Geometry::for_mode(w, h)
            .map_err(|e| format!("gop=on: the monitor's {w}x{h} mode: {e}"))?;
        let fb = BootFramebuffer {
            bar: FB_BAR,
            offset: FB_OFFSET,
            geometry,
        };
        if !fb.fits(bar1_bytes) {
            return Err(format!(
                "gop=on: the {w}x{h} boot framebuffer needs {:#x} bytes at BAR1 offset 0, and bar1-size \
                 is {bar1_bytes:#x} — refused by name, never clamped",
                geometry.fb_size
            ));
        }
        let edid = monitor
            .edid()
            .map_err(|e| format!("gop=on: the monitor's EDID: {e}"))?
            .to_vec();
        Ok(Some(BootPlan { fb, edid }))
    }

    /// G: the framebuffer's bytes of BAR1 and of store.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.fb.geometry.fb_size
    }

    /// ★ The device's option ROM: the embedded driver, unmodified — or its signed copy from
    /// `gop-efi=` ([`SignedGop`]) — behind a ROM header and PCIR carrying `vendor`/`device`/`class`
    /// (the identity kf3 presents) and this plan's `KFGP` descriptor.
    ///
    /// # Errors
    /// `kf_oprom::PackError` by name (a class that is not a display controller, above all), or the
    /// signed copy's refusal.
    pub fn rom(
        &self,
        vendor: u16,
        device: u16,
        class: u32,
        signed: Option<&SignedGop>,
    ) -> Result<Vec<u8>, String> {
        let id = Identity {
            vendor,
            device,
            class: ClassCode::from_u24(class),
        };
        let packed = match signed {
            None => kf_gop_image::pack_kf_gop(&id, &self.fb, &self.edid).map_err(|e| e.to_string()),
            Some(s) => kf_gop_image::pack_kf_gop_signed(&s.bytes, &id, &self.fb, &self.edid)
                .map(|(rom, _)| rom)
                .map_err(|e| format!("gop-efi={}: {e}", s.path)),
        };
        packed.map_err(|e| {
            format!("gop=on: packing the option ROM for {vendor:04x}:{device:04x}: {e}")
        })
    }

    /// The surface the display worker shows until the guest arms a head.
    #[must_use]
    pub fn surface(&self) -> kf_disp::scanout::BootSurface {
        let g = self.fb.geometry;
        kf_disp::scanout::BootSurface {
            width: g.width,
            height: g.height,
            pitch: g.pitch,
            bytes: g.fb_size,
        }
    }
}

/// The largest file `gop-efi=` may name: far above a kf-gop build plus its certificate table, and
/// the bound on what a path can make realize read.
pub const GOP_EFI_MAX: u64 = 1 << 20;

/// ★ **A signed copy of the embedded driver** (`gop-efi=<path>`, 2026-10-04, branch `v3-windows`,
/// `docs/OWNER_RULINGS.md` §K: the ROM's EFI driver signed with a per-install key enrolled in the VM's
/// `db`). Read once at realize and accepted only as `kf_gop_image::KF_GOP_EFI` plus an Authenticode
/// certificate table (`kf_oprom::pe::signed_twin_of`): kf3 still serves exactly the driver it
/// embeds, and the key never enters the build. The signature itself is verified by the guest's
/// firmware against its `db`, never here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedGop {
    /// The path it was read from (for the log and refusals).
    pub path: String,
    /// The signed file.
    pub bytes: Vec<u8>,
    /// Where its certificate table is.
    pub info: kf_oprom::pe::SignedInfo,
}

impl SignedGop {
    /// `None` without `gop-efi=` (or with an empty one).
    ///
    /// # Errors
    /// By name, before anything is reserved: `gop-efi=` with `gop=off`; a file that cannot be read
    /// or is larger than [`GOP_EFI_MAX`]; a file that is not the embedded driver plus a signature.
    pub fn for_config(gop: bool, path: Option<&str>) -> Result<Option<SignedGop>, String> {
        let Some(path) = path.filter(|p| !p.is_empty()) else {
            return Ok(None);
        };
        if !gop {
            return Err(format!(
                "gop-efi={path} needs gop=on: it is the boot display's signed driver \
                 (docs/design/V3_DISPLAY.md §4.11)"
            ));
        }
        let bytes = read_bounded(path)?;
        SignedGop::from_bytes(path, bytes).map(Some)
    }

    /// Validate `bytes` as the signed twin of the embedded driver.
    ///
    /// # Errors
    /// `kf_oprom::pe::SigError`, by name, prefixed with the path.
    pub fn from_bytes(path: &str, bytes: Vec<u8>) -> Result<SignedGop, String> {
        let info = kf_oprom::pe::signed_twin_of(&bytes, kf_gop_image::KF_GOP_EFI)
            .map_err(|e| format!("gop-efi={path}: {e}"))?;
        Ok(SignedGop {
            path: path.to_owned(),
            bytes,
            info,
        })
    }
}

fn read_bounded(path: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("gop-efi={path}: {e}"))?;
    let mut bytes = Vec::new();
    file.take(GOP_EFI_MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("gop-efi={path}: {e}"))?;
    if bytes.len() as u64 > GOP_EFI_MAX {
        return Err(format!(
            "gop-efi={path}: larger than {GOP_EFI_MAX} bytes, refused unread"
        ));
    }
    Ok(bytes)
}

/// ★ How the boot display reaches the GSP state machine (fn 72) and fn 65's encoder — ONE decision,
/// made from the plan, for both halves of the device (`crate::device::Device::realize` builds both
/// from it).
///
/// - `gop=on`: one [`kf_gsp::SystemInfoCell`], given to [`kf_gsp::GspFsm`] (which keeps fn 72's body
///   in it) and, as a [`ConsoleSeat`] with `boot_fb = Some(G)`, to every `StaticInfoPolicy` the chain
///   recipe rebuilds — so it survives `ReselectAtFn1`.
/// - `gop=off`: **no cell and no seat**. fn 72 is dropped unread and fn 65 is today's table, by
///   construction: nothing about the boot display exists in the device to be consulted.
///
/// ⊘ CORRECTED 2026-10-03 (`v3-gop`, the review of `v3-gop-kf3`): the cell used to be attached to
/// every device, so with `gop=off` the state machine still copied each fn 72 body under its mutex and
/// fn 65 decoded and logged `consoleMemSize` — "byte-identical replies", but not today's device, and
/// the state-machine test of the no-cell default covered a configuration kf3 never ran.
#[derive(Debug, Clone)]
pub struct ConsoleWiring {
    seat: Option<ConsoleSeat>,
}

impl ConsoleWiring {
    /// The wiring for `plan` (`None` = `gop=off`) on a device whose guest BAR1 is `bar1_bytes`.
    #[must_use]
    pub fn for_plan(plan: Option<&BootPlan>, bar1_bytes: u64) -> ConsoleWiring {
        ConsoleWiring {
            seat: plan.map(|p| ConsoleSeat {
                system_info: kf_gsp::SystemInfoCell::new(),
                bar1_bytes,
                boot_fb: Some(p.bytes()),
            }),
        }
    }

    /// The GSP state machine for `abi`: keeping fn 72's body in the seat's cell with `gop=on`, a
    /// fresh one with `gop=off`.
    #[must_use]
    pub fn fsm(&self, abi: kf_gsp::GspAbi) -> kf_gsp::GspFsm {
        let fsm = kf_gsp::GspFsm::new(abi);
        match &self.seat {
            Some(seat) => fsm.with_system_info_cell(seat.system_info.clone()),
            None => fsm,
        }
    }

    /// fn 65's console seat for one chain build: `Some` (the same cell every time) with `gop=on`,
    /// `None` with `gop=off`.
    #[must_use]
    pub fn seat(&self) -> Option<ConsoleSeat> {
        self.seat.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kf_oprom::Descriptor;

    fn monitor() -> kf_disp::edid::Monitor {
        kf_rm::display::monitors().remove(0)
    }

    /// ⊘ The default: no plan, whatever else is configured — every `gop` path is skipped.
    #[test]
    fn gop_off_plans_nothing() {
        for display in [false, true] {
            assert_eq!(
                BootPlan::for_config(false, display, 256 << 20, &monitor()),
                Ok(None)
            );
        }
    }

    #[test]
    fn gop_on_without_display_is_refused_by_name() {
        let e = BootPlan::for_config(true, false, 256 << 20, &monitor()).unwrap_err();
        assert!(e.contains("display=on"), "{e}");
    }

    #[test]
    fn the_1080p_monitor_gives_bar1_offset_0_and_g_0x7f0000() {
        let p = BootPlan::for_config(true, true, 256 << 20, &monitor())
            .unwrap()
            .unwrap();
        assert_eq!((p.fb.bar, p.fb.offset), (1, 0));
        assert_eq!(
            (
                p.fb.geometry.width,
                p.fb.geometry.height,
                p.fb.geometry.pitch
            ),
            (1920, 1080, 7680)
        );
        assert_eq!(p.bytes(), 0x7F_0000);
        assert_eq!(p.edid, monitor().edid().unwrap().to_vec());
        let l = kf_disp::scanout::boot_layer(&p.surface()).expect("the boot layer plans");
        assert!(l.extent <= p.bytes());
    }

    #[test]
    fn a_framebuffer_larger_than_bar1_is_refused_by_name() {
        let e = BootPlan::for_config(true, true, 0x40_0000, &monitor()).unwrap_err();
        assert!(e.contains("bar1-size") && e.contains("0x7f0000"), "{e}");
        assert!(BootPlan::for_config(true, true, 0x7F_0000, &monitor()).is_ok());
    }

    /// The ROM carries the identity kf3 presents and the plan's descriptor, around the embedded
    /// driver byte for byte.
    #[test]
    fn the_rom_carries_the_presented_identity_and_the_plan() {
        let p = BootPlan::for_config(true, true, 256 << 20, &monitor())
            .unwrap()
            .unwrap();
        let rom = p.rom(0x10de, 0x2504, 0x03_00_00, None).unwrap();
        assert_eq!(rom.len() % 512, 0);
        let d = Descriptor::find(&rom).expect("a KFGP descriptor");
        assert_eq!((d.vendor, d.device, d.fb), (0x10de, 0x2504, p.fb));
        assert_eq!(d.edid, &p.edid[..]);
        let images = kf_oprom::rom::parse(&rom).unwrap();
        assert_eq!(images.len(), 1);
        let pcir = images[0].pcir;
        assert_eq!(
            (pcir.vendor, pcir.device, pcir.class),
            (0x10de, 0x2504, ClassCode::from_u24(0x03_00_00))
        );
        assert_eq!(images[0].efi_payload(), Some(kf_gop_image::KF_GOP_EFI));
        assert_eq!(
            images[0].efi.unwrap().machine,
            kf_oprom::pe::inspect(kf_gop_image::KF_GOP_EFI)
                .unwrap()
                .machine,
            "EfiMachineType is the built driver's own (§K)"
        );
        // a 3D controller (0x0302) is still a display controller; a USB controller is not
        assert!(p.rom(0x10de, 0x2204, 0x03_02_00, None).is_ok());
        let e = p.rom(0x10de, 0x2504, 0x0C_03_30, None).unwrap_err();
        assert!(e.contains("not a display controller"), "{e}");
    }

    /// ★ 2026-10-04 (v3-windows): `gop-efi=` — a signed copy of the embedded driver is served as the
    /// ROM's image; everything else is refused by name before anything is reserved.
    #[test]
    fn a_signed_copy_is_served_and_gop_efi_without_gop_is_refused() {
        assert_eq!(SignedGop::for_config(true, None), Ok(None));
        assert_eq!(SignedGop::for_config(false, Some("")), Ok(None));
        let e = SignedGop::for_config(false, Some("/x.efi")).unwrap_err();
        assert!(e.contains("needs gop=on"), "{e}");
        let e = SignedGop::for_config(true, Some("/nonexistent/kf-gop.signed.efi")).unwrap_err();
        assert!(e.contains("/nonexistent/kf-gop.signed.efi"), "{e}");

        let signed = kf_oprom::pe::synthetic_signed(kf_gop_image::KF_GOP_EFI, 1500);
        let s = SignedGop::from_bytes("mem", signed.clone()).unwrap();
        let p = BootPlan::for_config(true, true, 256 << 20, &monitor())
            .unwrap()
            .unwrap();
        let rom = p.rom(0x10de, 0x2504, 0x03_00_00, Some(&s)).unwrap();
        let images = kf_oprom::rom::parse(&rom).unwrap();
        assert_eq!(images[0].efi_payload(), Some(&signed[..]));

        let e = SignedGop::from_bytes("mem", kf_gop_image::KF_GOP_EFI.to_vec()).unwrap_err();
        assert!(e.contains("no certificate table"), "{e}");
    }

    /// A file larger than the bound is refused unread.
    #[test]
    fn an_oversized_gop_efi_is_refused() {
        let dir = std::env::temp_dir().join(format!("kf3-gop-efi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big = dir.join("big.efi");
        std::fs::write(&big, vec![0u8; (GOP_EFI_MAX + 1) as usize]).unwrap();
        let e = SignedGop::for_config(true, big.to_str()).unwrap_err();
        assert!(e.contains("larger than"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn abi() -> kf_gsp::GspAbi {
        let v = kf_abi::DriverVersion::parse("580.159.04").unwrap();
        kf_rm::abi::gsp_abi_for(v).unwrap()
    }

    /// ★ `gop=off` is today's device in BOTH halves the boot display touches — the state machine is a
    /// fresh one (no cell: fn 72 is dropped unread, `kf_gsp`'s `without_a_cell_fn72_changes_nothing`)
    /// and fn 65 has no seat (today's table, `kf-rm`'s `tests/console_region.rs`). Built through the
    /// same `ConsoleWiring` `Device::realize` uses. ⊘ Before 2026-10-03 (`v3-gop`) the device attached
    /// the cell whatever `gop` said, and this test would fail on the first assertion.
    #[test]
    fn gop_off_wires_no_cell_and_no_seat() {
        let w = ConsoleWiring::for_plan(None, 256 << 20);
        assert_eq!(
            w.fsm(abi()),
            kf_gsp::GspFsm::new(abi()),
            "no SystemInfoCell with gop=off"
        );
        assert!(w.seat().is_none(), "no ConsoleSeat with gop=off");
    }

    /// `gop=on`: the state machine's cell IS the seat's (one cell, so fn 65 reads what fn 72 kept),
    /// every chain build gets that same cell, and the seat carries G and the guest's BAR1.
    #[test]
    fn gop_on_wires_one_cell_into_both_halves() {
        let plan = BootPlan::for_config(true, true, 256 << 20, &monitor())
            .unwrap()
            .unwrap();
        let w = ConsoleWiring::for_plan(Some(&plan), 256 << 20);
        let seat = w.seat().expect("a seat with gop=on");
        assert_eq!(seat.boot_fb, Some(0x7F_0000));
        assert_eq!(seat.bar1_bytes, 256 << 20);
        assert_eq!(
            w.fsm(abi()),
            kf_gsp::GspFsm::new(abi()).with_system_info_cell(seat.system_info.clone()),
            "the state machine keeps fn 72 in the seat's own cell"
        );
        assert_eq!(
            w.seat().unwrap().system_info,
            seat.system_info,
            "every rebuild of the chain gets the same cell"
        );
        assert_ne!(
            w.fsm(abi()),
            kf_gsp::GspFsm::new(abi()),
            "and that is not the gop=off machine"
        );
    }
}
