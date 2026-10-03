// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **The GPU-copy broker rung, kf3's side** (`docs/design/V3_DISPLAY.md` §8.11,
//! `OWNER_RULINGS.md` §L): the realize-time probe, the provisioning of kayfabe's own VRAM frame
//! slots, and the worker's adoption of each. The decisions are GPU-free in
//! `kf_broker::gpucopy`; the ioctls in `kf_linux_raw::drm`; the ABI in `kf_abi::drmnv`.
//!
//! **Per slot** (§8.11's sequence; never on the frame path, the main loop or a vCPU — the
//! NVKMS-backed ioctls take RM locks):
//! 1. the render node of THIS GPU (by PCI address, `st_rdev`-checked), opened at realize before
//!    QEMU drops privileges; `GET_DEV_INFO` (`supports_alloc` = nvidia-drm `modeset=1`); the ABI
//!    gate ([`kf_abi::drmnv::abi_measured`], the host driver tag, and nvidia-drm's own version);
//! 2. a VRAM object from the DISPLAY's own RM client ([`kf_host::HostRm::alloc_display_slot`];
//!    `OWNER_RULINGS.md` §N's client split — the store, guest RAM and twins are in another client);
//! 3. exported to a fresh nvidiactl fd ([`kf_host::HostRm::export_display_slot`]);
//! 4. `GEM_IMPORT_NVKMS_MEMORY` + `PRIME_HANDLE_TO_FD` → a dma-buf, checked `DMA_BUF_MAGIC`, its
//!    identity checked against every backing the ring carries (a collision is retried with a fresh
//!    GEM import: PRIME caches one dma-buf per GEM object, so re-exporting cannot help);
//! 5. on the display WORKER (it owns the CUDA context): the CUDA import from the same export fd,
//!    a clear, the pack self-test on the first slot, then the export fd is CLOSED (both imports
//!    hold the object; only the dma-buf leaves the process) and the slot is installed into the
//!    ring while it is free.
//!
//! Nothing here is the guest's: no guest surface, no slice of the store, is ever named to the
//! render node — the only object exported is a [`kf_host::DisplaySlot`], which only
//! `alloc_display_slot` makes.

use kf_abi::drmnv;
use kf_broker::gpucopy::Request;
use kf_broker::{FrameRing, VramFds};
use kf_disp::vramslot::{SLOT_CLASS0, SLOT_H_LOG2};
use kf_linux_raw::drm::{DrmNodes, DrmRender, NvImport, dev_dri, drm_nodes_for};
use kf_linux_raw::{CharDevice, DMA_BUF_MAGIC};
use std::os::fd::{AsFd, OwnedFd};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

/// ★ What realize established about the rung on this host (steps 1-2), and the display's own RM
/// client. VM-lifetime (leaked with the device).
pub struct VramSetup {
    render: DrmRender,
    nodes: DrmNodes,
    info: drmnv::DevInfo,
    modifier: u64,
    rm: kf_host::HostRm,
    attrs: kf_abi::submit::SlotAttrs,
    /// Every slot object: retired, never freed, while the device lives (a compositor may hold an
    /// import of any of them).
    kept: Mutex<Vec<kf_host::DisplaySlot>>,
}

impl std::fmt::Debug for VramSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VramSetup")
            .field("node", self.render.id())
            .field("modifier", &format_args!("{:#018x}", self.modifier))
            .field("attrs", &self.attrs.name)
            .finish_non_exhaustive()
    }
}

/// One slot, made by the provisioning thread, for the worker to adopt (step 5).
#[derive(Debug)]
pub struct Provisioned {
    /// The ring slot it backs.
    pub slot: usize,
    /// Its size.
    pub bytes: u64,
    /// The RM export (a fresh nvidiactl fd) — CUDA imports from it, then it is closed.
    export: CharDevice,
    /// The dma-buf the broker receives.
    dmabuf: OwnedFd,
}

impl Provisioned {
    /// The export fd's number, for the CUDA import.
    #[must_use]
    pub fn export_fd(&self) -> i32 {
        self.export.fd_number()
    }

    /// ★ After the CUDA import: the export fd CLOSES here (§8.11 step 8 — both imports dup the
    /// object from RM's export client, so only the export handle goes) and the dma-buf becomes
    /// the ring's backing.
    ///
    /// # Errors
    /// The identity read.
    pub fn into_backing(self) -> Result<(usize, VramFds), String> {
        let Provisioned {
            slot,
            bytes,
            export,
            dmabuf,
        } = self;
        drop(export);
        VramFds::new(dmabuf, bytes)
            .map(|v| (slot, v))
            .map_err(|e| format!("display VRAM slot {slot}: the dma-buf's identity: {e}"))
    }
}

/// The attribute set: `KF3_VRAM_ATTRS` (`s0`, `s1`, `s2`) for the box experiment E1, else S1
/// (NVKMS's offscreen block-linear allocation). ⊘ Which set CUDA imports AND a compositor samples
/// is not measured; E1 chooses, and the default follows E1.
fn attrs_from_env() -> Result<kf_abi::submit::SlotAttrs, String> {
    use kf_abi::submit::SLOT_ATTR_SETS;
    match std::env::var("KF3_VRAM_ATTRS") {
        Err(_) => Ok(kf_abi::submit::SLOT_S1_NVKMS_OFFSCREEN),
        Ok(v) => SLOT_ATTR_SETS
            .iter()
            .find(|a| a.name.starts_with(&format!("{v}-")) || a.name == v)
            .copied()
            .ok_or_else(|| format!("KF3_VRAM_ATTRS={v:?}: the sets are s0, s1, s2")),
    }
}

impl VramSetup {
    /// ★ Steps 0-2 at realize: the ABI gate (the host driver's tag probed equal by
    /// `tools/drivermatrix/drmnv.py`, and nvidia-drm's own version the same), the render node of
    /// GPU `bdf`, `GET_DEV_INFO`, and the display's own RM client on minor `gpu_minor`.
    ///
    /// # Errors
    /// Every refusal names the GPU-copy rung and why.
    pub fn probe(
        dev: &kf_linux_raw::DevDir,
        bdf: &str,
        gpu_minor: u32,
        host_driver: &str,
    ) -> Result<VramSetup, String> {
        let v = kf_abi::host_driver::HostDriverVersion::parse(host_driver).ok_or_else(|| {
            format!("GPU-copy rung: the host driver version {host_driver:?} does not parse")
        })?;
        drmnv::abi_measured(v).map_err(|e| e.to_string())?;
        // ⊘ nvidia-drm is a separate module: the RM's tag says nothing about it on its own
        let drm_version = std::fs::read_to_string("/sys/module/nvidia_drm/version")
            .map(|s| s.trim().to_string())
            .map_err(|e| {
                format!(
                    "GPU-copy rung: /sys/module/nvidia_drm/version cannot be read ({e}) — \
                     nvidia-drm is not loaded"
                )
            })?;
        if drm_version != host_driver {
            return Err(format!(
                "GPU-copy rung: nvidia-drm is {drm_version}, the RM is {host_driver} — the private \
                 ABI is measured per tag, and these are two"
            ));
        }
        let nodes =
            drm_nodes_for(Path::new("/sys"), bdf).map_err(|e| format!("GPU-copy rung: {e}"))?;
        let node = nodes
            .render()
            .cloned()
            .ok_or_else(|| format!("GPU-copy rung: {bdf} has no DRM render node"))?;
        let render = DrmRender::open(&dev_dri(), &node).map_err(|e| {
            format!(
                "GPU-copy rung: /dev/dri/{} cannot be opened ({e}) — QEMU needs the render group \
                 or the seat's ACL on it",
                node.name
            )
        })?;
        let mut arg = [0u8; drmnv::GET_DEV_INFO_SIZE];
        render
            .nvidia_ioctl(drmnv::IOCTL_GET_DEV_INFO, &mut arg)
            .map_err(|e| format!("GPU-copy rung: DRM_IOCTL_NVIDIA_GET_DEV_INFO: {e}"))?;
        let info = drmnv::DevInfo::decode(&arg);
        let modifier = info.block_linear_modifier(SLOT_H_LOG2).ok_or_else(|| {
            "GPU-copy rung: nvidia-drm cannot import NVKMS memory here (GET_DEV_INFO \
             supports_alloc = 0: nvidia-drm runs without modeset=1)"
                .to_string()
        })?;
        let rm = kf_host::HostRm::open(
            dev,
            kf_arch::ids::GpuId(gpu_minor),
            &kf_chip::choose_host_classes,
        )
        .map_err(|e| format!("GPU-copy rung: the display's own RM client: {e}"))?;
        let attrs = attrs_from_env()?;
        eprintln!(
            "kf3: broker: GPU-copy rung possible — render node {} ({}:{}), nvidia-drm gpu_id \
             {:#x}, modifier {modifier:#018x}, slot attributes {}",
            node.name, node.dev.0, node.dev.1, info.gpu_id, attrs.name
        );
        Ok(VramSetup {
            render,
            nodes,
            info,
            modifier,
            rm,
            attrs,
            kept: Mutex::new(Vec::new()),
        })
    }

    /// The modifier an ATTACH names.
    #[must_use]
    pub fn modifier(&self) -> u64 {
        self.modifier
    }

    /// Every DRM node of the GPU (`major:minor`), for the same-GPU test.
    #[must_use]
    pub fn nodes(&self) -> Vec<(u32, u32)> {
        self.nodes.nodes.iter().map(|n| n.dev).collect()
    }

    /// ★ Steps 2-4 for ring slot `slot`: a `bytes`-byte VRAM object of the display's RM client,
    /// exported, imported by NVKMS, as a dma-buf whose identity no backing in `ring` carries.
    ///
    /// # Errors
    /// The step that refused, naming display VRAM.
    pub fn make(&self, ring: &FrameRing, slot: usize, bytes: u64) -> Result<Provisioned, String> {
        let no = |what: &str, e: String| {
            format!(
                "display VRAM slot {slot} ({} MiB, {}): {what}: {e} — the GPU-copy rung is \
                 withdrawn; the host-memory rungs remain",
                bytes >> 20,
                self.attrs.name
            )
        };
        let obj = self
            .rm
            .alloc_display_slot(bytes, &self.attrs)
            .map_err(|e| no("the RM allocation", format!("{e:?}")))?;
        let export = self
            .rm
            .export_display_slot(&obj)
            .map_err(|e| no("the RM export", format!("{e:?}")))?;
        self.kept.lock().map_err(|_| "kept poisoned")?.push(obj);
        let surface = drmnv::BlockLinearSurface {
            log2_gobs_per_block: (0, SLOT_H_LOG2, 0),
            // NVKMS consults it only for a framebuffer made WITHOUT a modifier
            // (`ogkm-580: nvkms-kapi.c:2257-2316`); every ATTACH names one
            pitch_in_blocks: if bytes <= SLOT_CLASS0 { 120 } else { 240 },
            generic_memory: true,
        };
        for attempt in 0..3 {
            let mut arg = drmnv::gem_import_arg(bytes);
            let mut params = drmnv::priv_import_params(&surface);
            self.render
                .nvidia_import(
                    drmnv::IOCTL_GEM_IMPORT_NVKMS_MEMORY,
                    &mut arg,
                    NvImport {
                        ptr_at: drmnv::GEM_IMPORT_PTR_AT,
                        size_at: drmnv::GEM_IMPORT_PARAMS_SIZE_AT,
                        params: &mut params,
                        fd_at: drmnv::PRIV_IMPORT_FD_AT,
                        mem: export.as_fd(),
                    },
                )
                .map_err(|e| no("GEM_IMPORT_NVKMS_MEMORY", e.to_string()))?;
            let handle = drmnv::gem_import_handle(&arg);
            let dmabuf = kf_linux_raw::prime_handle_to_fd(&self.render, handle)
                .map_err(|e| no("PRIME_HANDLE_TO_FD", e.to_string()))?;
            let magic = kf_linux_raw::fs_magic(dmabuf.as_fd())
                .map_err(|e| no("fstatfs of the dma-buf", e.to_string()))?;
            if magic != DMA_BUF_MAGIC {
                return Err(no(
                    "PRIME_HANDLE_TO_FD",
                    format!("the descriptor is not a dma-buf (f_type {magic:#x})"),
                ));
            }
            let id = kf_linux_raw::fd_inode(dmabuf.as_fd())
                .map_err(|e| no("the dma-buf's identity", e.to_string()))?;
            if !ring.carries_id(id) {
                return Ok(Provisioned {
                    slot,
                    bytes,
                    export,
                    dmabuf,
                });
            }
            // dma-buf inodes come from their own counter, memfd inodes from shmem's: a clash is
            // possible. A fresh GEM import makes a fresh dma-buf; this one was never sent.
            eprintln!(
                "kf3: broker: display VRAM slot {slot}: dma-buf id {id} collides (attempt {}); \
                 importing again",
                attempt + 1
            );
            drop(dmabuf);
            let _ = kf_linux_raw::gem_close(&self.render, handle);
        }
        Err(no(
            "the dma-buf identity",
            "three imports in a row collided".into(),
        ))
    }

    /// The bring-up line's `GET_DEV_INFO`.
    #[must_use]
    pub fn info(&self) -> drmnv::DevInfo {
        self.info
    }
}

/// ★ Start the provisioning thread: it serves [`Request`]s (`Provisioning`'s, sent by the worker)
/// and hands each made slot — or the first refusal, after which it stops — back to the worker.
///
/// # Errors
/// The thread could not be started.
pub fn spawn(
    setup: &'static VramSetup,
    ring: Arc<FrameRing>,
) -> Result<
    (
        mpsc::Sender<Request>,
        mpsc::Receiver<Result<Provisioned, String>>,
    ),
    String,
> {
    let (req_tx, req_rx) = mpsc::channel::<Request>();
    let (got_tx, got_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("kf3-vram-provision".into())
        .spawn(move || {
            while let Ok(r) = req_rx.recv() {
                for &slot in &r.slots {
                    let made = setup.make(&ring, slot, r.bytes);
                    let failed = made.is_err();
                    if got_tx.send(made).is_err() || failed {
                        return;
                    }
                }
            }
        })
        .map_err(|e| format!("the VRAM provisioning thread: {e}"))?;
    Ok((req_tx, got_rx))
}

/// ★ The pack self-test's synthetic frames: odd sizes at two block heights, compared byte for byte
/// with `kf_disp::vramslot::pack_reference` before the slot is ever exported.
#[must_use]
pub fn selftest_cases() -> [(u32, u32, u32); 2] {
    [(70, 40, SLOT_H_LOG2), (70, 40, 1)]
}

/// A deterministic staging pattern with no zero byte (so padding shows).
#[must_use]
pub fn selftest_staging(w: u32, h: u32) -> Vec<u8> {
    (0..u64::from(w) * u64::from(h) * 4)
        .map(|i| ((i.wrapping_mul(2_654_435_761) >> 9) as u8) | 1)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ The pinned ioctl numbers agree across the two crates that each carry them (kf-linux-raw's
    /// allowlist cannot see kf-abi).
    #[test]
    fn the_allowlisted_ioctls_are_the_abis() {
        assert!(kf_linux_raw::drm::NVIDIA_ALLOWED.contains(&drmnv::IOCTL_GET_DEV_INFO));
        assert!(kf_linux_raw::drm::NVIDIA_ALLOWED.contains(&drmnv::IOCTL_GEM_IMPORT_NVKMS_MEMORY));
        assert_eq!(kf_linux_raw::drm::NVIDIA_ALLOWED.len(), 2);
    }

    /// The slot sizes the provisioner asks for fit kf-host's cap and granule.
    #[test]
    fn the_slot_sizes_fit_the_rm_guard() {
        for b in [SLOT_CLASS0, kf_disp::vramslot::SLOT_MAX] {
            assert!(kf_host::display_slot_size_ok(b), "{b:#x}");
        }
        assert!(kf_disp::vramslot::SLOT_MAX <= kf_host::DISPLAY_SLOT_CAP);
    }

    #[test]
    fn the_attribute_set_defaults_to_nvkms_offscreen() {
        // the env var is not set in CI
        if std::env::var("KF3_VRAM_ATTRS").is_err() {
            assert_eq!(attrs_from_env().unwrap().name, "s1-nvkms-offscreen");
        }
    }

    #[test]
    fn the_selftest_frames_have_no_zero_byte() {
        for (w, h, _) in selftest_cases() {
            assert!(selftest_staging(w, h).iter().all(|b| *b != 0));
        }
    }
}
