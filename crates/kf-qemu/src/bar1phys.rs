// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★ **When the guest's RM gives BAR1 up** (`docs/design/V3_DISPLAY.md` §4.11.13, box test B5).
//!
//! The two guest-visible acts of a non-preserving RM teardown that concern BAR1, in the order the
//! guest performs them (`gpuEnterShutdown_IMPL`: `gpuStateUnload` then `kgspUnloadRm`,
//! `ogkm-580: src/nvidia/src/kernel/gpu/gpu.c:3637-3655`):
//! 1. CPU-RM's `kbusStatePreUnload_GM107` destroys its BAR1 VA space (the preserved console mapping
//!    at VA 0 goes first, `kbusUnmapPreservedConsole_GM107`, `kern_bus_gm107.c:1278-1310`) and writes
//!    the BAR1-mode register back to PHYSICAL, target VID_MEM (`kbusTeardownMailbox_GM107`,
//!    `kern_bus_gm107.c:746-765`; [`kf_chip::bar1mode`]);
//! 2. `kgspUnloadRm` sends `UNLOADING_GUEST_DRIVER` (fn 47, `kernel_gsp.c:4301`) — GSP-RM's own
//!    unload, which runs the same `kbusStatePreUnload` on the firmware side.
//!
//! A PM transition (`bInPMTransition`) preserves BAR1 on both sides
//! (`kbusStatePreUnload_GM107` acts only without `GPU_STATE_FLAGS_PRESERVING`, `:775-786`).

/// The fn-47 body (`rpc_unloading_guest_driver_v1F_07`, `g_rpc-structures.h:378-383`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unloading {
    /// `bInPMTransition` — a suspend/hibernate: BAR1 is preserved.
    pub pm: bool,
    /// `bGc6Entering`.
    pub gc6: bool,
    /// `newLevel`.
    pub level: u32,
}

impl Unloading {
    /// ★ Whether this unload gives BAR1 up: every one that is not a PM transition.
    #[must_use]
    pub fn gives_bar1_up(&self) -> bool {
        !self.pm
    }
}

/// Decode a fn-47 body with the generated layout of the serving version.
///
/// # Errors
/// No layout for the version, a field the layout lacks, or a body shorter than the field — by name.
pub fn decode_unloading(
    layout: Option<&kf_abi::matrix::Layout>,
    body: &[u8],
) -> Result<Unloading, String> {
    let l =
        layout.ok_or("no rpc_unloading_guest_driver_v layout for the declared guest version")?;
    let get = |name: &str| -> Result<u64, String> {
        let r = l
            .field(name)
            .and_then(kf_abi::matrix::FieldAt::range)
            .ok_or_else(|| format!("rpc_unloading_guest_driver_v has no byte field {name}"))?;
        let b = body.get(r.clone()).ok_or_else(|| {
            format!(
                "fn 47 body of {} bytes ends before {name} [{}, {})",
                body.len(),
                r.start,
                r.end
            )
        })?;
        Ok(b.iter()
            .rev()
            .fold(0u64, |acc, &x| (acc << 8) | u64::from(x)))
    };
    Ok(Unloading {
        pm: get("bInPMTransition")? != 0,
        gc6: get("bGc6Entering")? != 0,
        level: u32::try_from(get("newLevel")?).unwrap_or(u32::MAX),
    })
}

/// ★ The VA thread, idle (no walk in flight or pending): show the boot framebuffer's physical view
/// again (`kf_mem::cpuwin::CpuWindow::reseed`) — unless the window changed since the request was
/// noticed (`at_change`, its change count then): an RM took BAR1 back first, and its placements
/// win. The log line, or `None` for a window that never had a boot framebuffer (`gop=off`).
pub fn restore_physical_view<V: kf_mem::cpuwin::ViewOps>(
    w: &kf_mem::cpuwin::CpuWindow<V>,
    at_change: u64,
) -> Option<String> {
    let (at, store_off, len) = w.boot_range()?;
    let now = w.changes();
    if now != at_change {
        return Some(format!(
            "BAR1's physical view NOT restored: BAR1 changed {} time(s) after the guest's RM gave it \
             up — an RM holds BAR1 again, its placements win",
            now.wrapping_sub(at_change)
        ));
    }
    Some(match w.reseed() {
        Ok(kf_mem::cpuwin::Reseed::Placed { life }) => format!(
            "BAR1 back to its physical view: [{at:#x}, +{len:#x}) shows store [{store_off:#x}, +{len:#x}) \
             again (seed life {life}) — the guest's RM gave BAR1 up"
        ),
        Ok(kf_mem::cpuwin::Reseed::AlreadyShown) => format!(
            "BAR1 [{at:#x}, +{len:#x}) already shows its physical view (the seed never retired)"
        ),
        Ok(kf_mem::cpuwin::Reseed::NoBootFramebuffer) => return None,
        Err(e) => format!("BAR1's physical view REFUSED: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> &'static kf_abi::matrix::Layout {
        let v = kf_abi::DriverVersion::parse("580.159.04").expect("version");
        kf_abi::generated::matrix::RPC_UNLOADING_GUEST_DRIVER_V
            .at(v)
            .expect("measured")
            .expect("present")
    }

    /// `rmmod` (and RM's last close): `{bInPMTransition = 0, bGc6Entering = 0, newLevel = 0}` —
    /// BAR1 is given up.
    #[test]
    fn a_teardown_gives_bar1_up_and_a_suspend_does_not() {
        let u = decode_unloading(Some(layout()), &[0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(
            u,
            Unloading {
                pm: false,
                gc6: false,
                level: 0
            }
        );
        assert!(u.gives_bar1_up());
        let s = decode_unloading(Some(layout()), &[1, 0, 0, 0, 3, 0, 0, 0]).unwrap();
        assert_eq!((s.pm, s.level), (true, 3));
        assert!(!s.gives_bar1_up());
    }

    /// A host that does whatever it is asked (a view is its store offset).
    struct Host;
    impl kf_mem::cpuwin::ViewOps for Host {
        type View = u64;
        fn arm_store(&self, off: u64, _len: u64) -> Result<u64, String> {
            Ok(off)
        }
        fn place_view(&self, _at: u64, _len: u64, _v: &u64) -> Result<(), String> {
            Ok(())
        }
        fn place_ram(&self, _at: u64, _len: u64, _off: u64) -> Result<(), String> {
            Ok(())
        }
        fn sink(&self, _at: u64, _len: u64) -> Result<(), String> {
            Ok(())
        }
        fn release(&self, _v: u64) -> Result<(), String> {
            Ok(())
        }
    }

    const G: u64 = 0x7F_0000;

    /// One RM life on a seeded BAR1: the console mapped at VA 0, then unmapped at teardown.
    fn one_life() -> kf_mem::cpuwin::CpuWindow<Host> {
        use kf_mem::ledger::{Desired, MapTarget};
        let w = kf_mem::cpuwin::CpuWindow::new(Host, 128 << 20);
        w.seed(0, 0, G).unwrap();
        let console = Desired {
            va: 0,
            len: G,
            off: 0,
            ram: false,
            kind: 0,
            perm: kf_host::MapPerm::READ_WRITE,
        };
        w.map(&console, false).unwrap();
        w.invalidate().unwrap();
        w.unmap(0, false).unwrap();
        w.invalidate().unwrap();
        w
    }

    #[test]
    fn the_physical_view_comes_back_when_the_rm_gave_bar1_up() {
        let w = one_life();
        let line = restore_physical_view(&w, w.changes()).expect("a line");
        assert!(
            line.contains("back to its physical view") && line.contains("seed life 2"),
            "{line}"
        );
        assert_eq!(w.seeded(), Some((0, 0, G)));
        let again = restore_physical_view(&w, w.changes()).expect("a line");
        assert!(again.contains("already shows"), "{again}");
    }

    /// ⊘ An RM that took BAR1 again before the VA thread got to the request wins.
    #[test]
    fn a_bar1_change_after_the_request_keeps_the_rms_placements() {
        let w = one_life();
        let line = restore_physical_view(&w, w.changes() - 1).expect("a line");
        assert!(line.contains("NOT restored"), "{line}");
        assert_eq!(w.seeded(), None);
    }

    /// `gop=off`: a window that never had a seed says nothing and does nothing.
    #[test]
    fn without_a_boot_framebuffer_nothing_happens() {
        let w = kf_mem::cpuwin::CpuWindow::new(Host, 128 << 20);
        assert_eq!(restore_physical_view(&w, 0), None);
    }

    #[test]
    fn a_short_body_or_no_layout_is_refused_by_name() {
        assert!(
            decode_unloading(Some(layout()), &[0, 0])
                .unwrap_err()
                .contains("newLevel")
        );
        assert!(
            decode_unloading(None, &[0; 8])
                .unwrap_err()
                .contains("no rpc_unloading")
        );
    }
}
