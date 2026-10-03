// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★★ **The boot display, kf3's side** (`docs/design/V3_DISPLAY.md` §4.11, property `gop`, default
//! off).
//!
//! kf3 shows a picture before the guest's NVIDIA driver loads the way a real card does: its PCI
//! expansion ROM carries a UEFI GOP driver whose framebuffer is a BAR1 range. Here that ROM is the
//! constant `kf-gop` driver embedded in kayfabe (`kf_oprom::KF_GOP_EFI`, owner ruling
//! `OWNER_RULINGS.md` §K) wrapped, per device, with the PCI ids and class kf3 already presents and a
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
//! - fn 65: the guest's preserved console becomes region 0 (`kf_rm::staticinfo::ConsoleSeat`);
//! - the host BAR1 budget: + G for the seed and the guest's console view overlapping while the seed
//!   retires (`crate::cardbudget::Demand::with_boot_fb`).
//!
//! ⊘ Nothing here traps, and nothing reads guest vidmem on the CPU: the ROM BAR is read-only RAM, the
//! seed is a placement inside the existing BAR1 memslot, and the zeroing and the scanout are GPU work.

use kf_oprom::{BootFramebuffer, ClassCode, Geometry, Identity};

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

    /// ★ The device's option ROM: the embedded driver, unmodified, behind a ROM header and PCIR
    /// carrying `vendor`/`device`/`class` (the identity kf3 presents) and this plan's `KFGP`
    /// descriptor.
    ///
    /// # Errors
    /// `kf_oprom::PackError` by name (a class that is not a display controller, above all).
    pub fn rom(&self, vendor: u16, device: u16, class: u32) -> Result<Vec<u8>, String> {
        let id = Identity {
            vendor,
            device,
            class: ClassCode::from_u24(class),
        };
        kf_oprom::pack_kf_gop(&id, &self.fb, &self.edid).map_err(|e| {
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
        let rom = p.rom(0x10de, 0x2504, 0x03_00_00).unwrap();
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
        assert_eq!(images[0].efi_payload(), Some(kf_oprom::KF_GOP_EFI));
        // a 3D controller (0x0302) is still a display controller; a USB controller is not
        assert!(p.rom(0x10de, 0x2204, 0x03_02_00).is_ok());
        let e = p.rom(0x10de, 0x2504, 0x0C_03_30).unwrap_err();
        assert!(e.contains("not a display controller"), "{e}");
    }
}
