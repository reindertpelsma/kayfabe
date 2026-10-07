// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
//! ★★ **The host GPU's DRM render node — the GPU-copy broker rung's one door** (`docs/design/
//! V3_DISPLAY.md` §8.11, `OWNER_RULINGS.md` §L). Safe code: the discovery is `sysfs` reads, the
//! open is [`DevDir`]/[`CharDevice`], and the two nvidia-drm ioctls go through
//! [`CharDevice::ioctl`] (its size check, its pointer patch-and-scrub). The three kernel-uapi
//! verbs that hand back or take a descriptor (`PRIME_HANDLE_TO_FD`, `GEM_CLOSE`,
//! `DMA_BUF_IOCTL_EXPORT_SYNC_FILE`) are in `drm_unsafe.rs`.
//!
//! **Which node.** The render node of the GPU at PCI address `bdf` — the identity kf3 already
//! checked at realize — found under `/sys/bus/pci/devices/<bdf>/drm/renderD<N>`, and opened only
//! when the opened file IS that device: a character device whose `st_rdev` equals the `dev`
//! sysfs states. Never a `/dev/dri/renderD128` guessed by ordinal (a multi-GPU host orders its
//! nodes by probe order, not by the GPU kf3 drives).
//!
//! **What may be issued.** Only the ioctls listed here ([`NVIDIA_ALLOWED`]), each with the
//! argument size its number declares: `GET_DEV_INFO`, `GEM_IMPORT_NVKMS_MEMORY` — both
//! `DRM_RENDER_ALLOW` (`ogkm-580: kernel-open/nvidia-drm/nvidia-drm-drv.c:1754-1765`), so no KMS
//! master and no root are needed (constraint §17). ⊘ They enter NVKMS and RM and can block on RM
//! locks: realize or a provisioning thread only, never a vCPU, the main loop, or a frame path.

use crate::{CharDevice, DevDir, Indirect, RawError};
use std::os::fd::{AsRawFd, BorrowedFd};
use std::path::{Path, PathBuf};

/// `DRM_IOCTL_NVIDIA_GET_DEV_INFO` (`kf_abi::drmnv::IOCTL_GET_DEV_INFO`, pinned there and
/// cross-checked against this list by kf-qemu's tests).
pub const NVIDIA_GET_DEV_INFO: u32 = 0xC024_6443;
/// `DRM_IOCTL_NVIDIA_GEM_IMPORT_NVKMS_MEMORY` (`kf_abi::drmnv::IOCTL_GEM_IMPORT_NVKMS_MEMORY`).
pub const NVIDIA_GEM_IMPORT_NVKMS_MEMORY: u32 = 0xC020_6441;
/// ★ The nvidia-drm ioctls this crate will issue on a render node — nothing else.
pub const NVIDIA_ALLOWED: &[u32] = &[NVIDIA_GET_DEV_INFO, NVIDIA_GEM_IMPORT_NVKMS_MEMORY];

/// Why a render-node operation was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrmError {
    /// A syscall or a bound of [`CharDevice::ioctl`].
    Raw(RawError),
    /// Refused here, by name.
    Refused(String),
}

impl core::fmt::Display for DrmError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DrmError::Raw(e) => write!(f, "{e}"),
            DrmError::Refused(why) => write!(f, "{why}"),
        }
    }
}

impl From<RawError> for DrmError {
    fn from(e: RawError) -> DrmError {
        DrmError::Raw(e)
    }
}

fn refused<T>(why: String) -> Result<T, DrmError> {
    Err(DrmError::Refused(why))
}

/// One DRM device node of a PCI function, as `sysfs` states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrmNodeId {
    /// `cardN` or `renderDN`.
    pub name: String,
    /// The device number (`major`, `minor`) its `dev` file states.
    pub dev: (u32, u32),
}

/// ★ Every DRM node of the PCI function — the render node to open, and every node's device
/// number (a compositor names the device it renders on by either: X11's DRI3 fd is usually the
/// primary node, a Wayland dma-buf feedback's `main_device` the render node).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrmNodes {
    /// All of them, `cardN` and `renderDN`, sorted by name.
    pub nodes: Vec<DrmNodeId>,
}

impl DrmNodes {
    /// The render node (`renderD<N>`), if the function has one.
    #[must_use]
    pub fn render(&self) -> Option<&DrmNodeId> {
        self.nodes.iter().find(|n| n.name.starts_with("renderD"))
    }

    /// ★ Whether `dev` is one of this GPU's DRM nodes (the same-GPU test of a compositor's
    /// device).
    #[must_use]
    pub fn contains(&self, dev: (u32, u32)) -> bool {
        self.nodes.iter().any(|n| n.dev == dev)
    }
}

/// ★ A PCI address as sysfs spells it: `dddd:bb:dd.f`, lower-case hex — anything else is
/// refused before it becomes a path component.
#[must_use]
pub fn is_pci_address(bdf: &str) -> bool {
    let b = bdf.as_bytes();
    let hex = |r: core::ops::Range<usize>| {
        b[r].iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    };
    b.len() == 12
        && b[4] == b':'
        && b[7] == b':'
        && b[10] == b'.'
        && hex(0..4)
        && hex(5..7)
        && hex(8..10)
        && (b'0'..=b'7').contains(&b[11])
}

fn parse_dev(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s.trim().split_once(':')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// ★ The DRM nodes of PCI function `bdf` under `sysfs_root` (`/sys` on a host; a fixture
/// directory in a test): `<root>/bus/pci/devices/<bdf>/drm/{card*,renderD*}/dev`.
///
/// # Errors
/// [`DrmError::Refused`]: a malformed `bdf`, no `drm` directory (nvidia-drm not loaded, or not
/// bound to this GPU), or a node whose `dev` file does not parse.
pub fn drm_nodes_for(sysfs_root: &Path, bdf: &str) -> Result<DrmNodes, DrmError> {
    if !is_pci_address(bdf) {
        return refused(format!("{bdf:?} is not a PCI address (dddd:bb:dd.f)"));
    }
    let dir = sysfs_root.join("bus/pci/devices").join(bdf).join("drm");
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) => {
            return refused(format!(
                "{} cannot be read ({e}): nvidia-drm is not loaded or not bound to {bdf}",
                dir.display()
            ));
        }
    };
    let mut nodes = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let numbered = |p: &str| {
            name.strip_prefix(p)
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
        };
        if !(numbered("card") || numbered("renderD")) {
            continue;
        }
        let devf = entry.path().join("dev");
        let text = std::fs::read_to_string(&devf)
            .map_err(|e| DrmError::Refused(format!("{} cannot be read: {e}", devf.display())))?;
        let Some(dev) = parse_dev(&text) else {
            return refused(format!(
                "{} says {:?}, not major:minor",
                devf.display(),
                text.trim()
            ));
        };
        nodes.push(DrmNodeId { name, dev });
    }
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(DrmNodes { nodes })
}

/// ★ An open render node, checked to BE the device sysfs named.
#[derive(Debug)]
pub struct DrmRender {
    dev: CharDevice,
    id: DrmNodeId,
}

/// The `(major, minor)` of a `dev_t` (glibc's `gnu_dev_major`/`gnu_dev_minor` arithmetic).
#[must_use]
pub fn dev_numbers(rdev: u64) -> (u32, u32) {
    let major = ((rdev >> 8) & 0xfff) | ((rdev >> 32) & !0xfff);
    let minor = (rdev & 0xff) | ((rdev >> 12) & !0xff);
    (major as u32, minor as u32)
}

impl DrmRender {
    /// ★ Open `node` from `dev_dri` (`/dev/dri` on a host), read-write, close-on-exec, and
    /// refuse it unless the opened file is a character device with `node`'s device number.
    ///
    /// # Errors
    /// The open, or [`DrmError::Refused`] naming the mismatch.
    pub fn open(dev_dri: &Path, node: &DrmNodeId) -> Result<DrmRender, DrmError> {
        use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
        if node.name.contains('/') || node.name.starts_with('.') {
            return refused(format!("{:?} is not a node name", node.name));
        }
        let cdir = std::ffi::CString::new(dev_dri.as_os_str().as_encoded_bytes())
            .map_err(|_| DrmError::Refused(format!("{} has a NUL", dev_dri.display())))?;
        let dir = DevDir::open(&cdir)?;
        let cname = std::ffi::CString::new(node.name.as_bytes())
            .map_err(|_| DrmError::Refused("a node name with a NUL".into()))?;
        let dev = CharDevice::openat(&dir, &cname)?;
        let f = std::fs::File::from(
            dev.as_fd()
                .try_clone_to_owned()
                .map_err(|e| DrmError::Refused(format!("dup of {}: {e}", node.name)))?,
        );
        let md = f
            .metadata()
            .map_err(|e| DrmError::Refused(format!("fstat of {}: {e}", node.name)))?;
        let got = dev_numbers(md.rdev());
        if !md.file_type().is_char_device() || got != node.dev {
            return refused(format!(
                "{}/{} is not the GPU's render node: sysfs states {}:{}, the file is {} {}:{}",
                dev_dri.display(),
                node.name,
                node.dev.0,
                node.dev.1,
                if md.file_type().is_char_device() {
                    "char"
                } else {
                    "not a character device,"
                },
                got.0,
                got.1
            ));
        }
        Ok(DrmRender {
            dev,
            id: node.clone(),
        })
    }

    /// The node this is.
    #[must_use]
    pub fn id(&self) -> &DrmNodeId {
        &self.id
    }

    /// Borrow the descriptor (`drm_unsafe.rs`' verbs take it).
    #[must_use]
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.dev.as_fd()
    }

    /// ★ One nvidia-drm ioctl with no pointer in its argument ([`NVIDIA_GET_DEV_INFO`]).
    ///
    /// # Errors
    /// [`DrmError::Refused`] for a request not in [`NVIDIA_ALLOWED`] or one that carries a
    /// pointer; [`CharDevice::ioctl`]'s refusals (its size check included).
    pub fn nvidia_ioctl(&self, request: u32, arg: &mut [u8]) -> Result<(), DrmError> {
        if request != NVIDIA_GET_DEV_INFO {
            return refused(format!(
                "nvidia-drm ioctl {request:#x} is not one this VMM issues without a pointer"
            ));
        }
        self.dev.ioctl(u64::from(request), arg, &mut [])?;
        Ok(())
    }

    /// ★ `GEM_IMPORT_NVKMS_MEMORY`: `arg`'s pointer field at `imp.ptr_at` points at `imp.params`
    /// for the call (patched and scrubbed by [`CharDevice::ioctl`]); the `u64` at `imp.size_at`
    /// must equal `imp.params.len()` — checked HERE, so the kernel never copies more than the
    /// buffer holds — and `params[fd_at..fd_at+4]` becomes `imp.mem`'s descriptor number, `mem`
    /// being borrowed (open) for the whole call.
    ///
    /// # Errors
    /// [`DrmError::Refused`] for another request, a size field that does not match, or an fd
    /// field outside the params; [`CharDevice::ioctl`]'s refusals.
    pub fn nvidia_import(
        &self,
        request: u32,
        arg: &mut [u8],
        imp: NvImport<'_>,
    ) -> Result<(), DrmError> {
        if request != NVIDIA_GEM_IMPORT_NVKMS_MEMORY {
            return refused(format!(
                "nvidia-drm ioctl {request:#x} is not the import this VMM issues"
            ));
        }
        let NvImport {
            ptr_at,
            size_at,
            params,
            fd_at,
            mem,
        } = imp;
        let Some(size) = size_at
            .checked_add(8)
            .and_then(|e| arg.get(size_at..e))
            .and_then(|b| <[u8; 8]>::try_from(b).ok())
            .map(u64::from_le_bytes)
        else {
            return refused(format!("the size field at +{size_at} leaves the argument"));
        };
        if size != params.len() as u64 {
            return refused(format!(
                "the argument states {size} bytes of private params, the buffer holds {}",
                params.len()
            ));
        }
        let Some(fd_field) = fd_at.checked_add(4).and_then(|e| params.get_mut(fd_at..e)) else {
            return refused(format!(
                "the fd field at +{fd_at} leaves the private params"
            ));
        };
        fd_field.copy_from_slice(&mem.as_raw_fd().to_le_bytes());
        let r = self.dev.ioctl(
            u64::from(request),
            arg,
            &mut [Indirect::new(ptr_at, params)],
        );
        r.map(|_| ()).map_err(DrmError::from)
    }
}

/// ★ The pointed-at half of `GEM_IMPORT_NVKMS_MEMORY` ([`DrmRender::nvidia_import`]): where the
/// argument's pointer and size fields are, the private params buffer, and the descriptor whose
/// number goes into it at `fd_at`.
#[derive(Debug)]
pub struct NvImport<'a> {
    /// The pointer field's offset in the argument.
    pub ptr_at: usize,
    /// The `u64` size field's offset in the argument.
    pub size_at: usize,
    /// The private params the pointer names (for the call only).
    pub params: &'a mut [u8],
    /// Where in `params` the descriptor number goes (4 bytes).
    pub fd_at: usize,
    /// The descriptor, borrowed for the whole call.
    pub mem: BorrowedFd<'a>,
}

/// `/dev/dri`, where a host keeps its DRM nodes.
#[must_use]
pub fn dev_dri() -> PathBuf {
    PathBuf::from("/dev/dri")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kf-drm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn node(root: &Path, bdf: &str, name: &str, dev: &str) {
        let p = root
            .join("bus/pci/devices")
            .join(bdf)
            .join("drm")
            .join(name);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("dev"), format!("{dev}\n")).unwrap();
    }

    #[test]
    fn a_pci_address_is_exactly_the_sysfs_spelling() {
        assert!(is_pci_address("0000:01:00.0"));
        assert!(is_pci_address("0000:c1:1f.7"));
        for bad in [
            "0000:01:00.8",
            "0000:01:00",
            "../../etc/x",
            "0000:01:00.0/",
            "0000:0G:00.0",
            "0000:01:00.00",
            "0000;01:00.0",
            "0000:01:0A.0",
        ] {
            assert!(!is_pci_address(bad), "{bad}");
        }
    }

    /// ★ Discovery on a sysfs fixture: both nodes, sorted, the render node found; a malformed
    /// `dev`, a missing `drm` directory and a path-like bdf each refused by name.
    #[test]
    fn the_render_node_is_found_under_the_pci_function() {
        let root = fixture("sysfs");
        let bdf = "0000:01:00.0";
        node(&root, bdf, "renderD129", "226:129");
        node(&root, bdf, "card1", "226:1");
        // another GPU's node, and a non-node entry, are not this function's
        node(&root, "0000:02:00.0", "renderD128", "226:128");
        std::fs::create_dir_all(root.join("bus/pci/devices").join(bdf).join("drm/version"))
            .unwrap();
        let n = drm_nodes_for(&root, bdf).unwrap();
        assert_eq!(
            n.nodes,
            vec![
                DrmNodeId {
                    name: "card1".into(),
                    dev: (226, 1)
                },
                DrmNodeId {
                    name: "renderD129".into(),
                    dev: (226, 129)
                },
            ]
        );
        assert_eq!(n.render().unwrap().name, "renderD129");
        assert!(n.contains((226, 1)) && n.contains((226, 129)));
        assert!(!n.contains((226, 128)), "another GPU's render node");
        let e = drm_nodes_for(&root, "0000:03:00.0").unwrap_err();
        assert!(e.to_string().contains("not loaded or not bound"), "{e}");
        assert!(matches!(
            drm_nodes_for(&root, "../x"),
            Err(DrmError::Refused(_))
        ));
        node(&root, "0000:04:00.0", "renderD130", "garbage");
        assert!(
            drm_nodes_for(&root, "0000:04:00.0")
                .unwrap_err()
                .to_string()
                .contains("not major:minor")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ★ The open checks the FILE: a symlink to `/dev/null` (char 1:3) opens when sysfs states
    /// 1:3, and is refused by name when sysfs states the render node's 226:128; a regular file
    /// is refused as not a character device.
    #[test]
    fn the_opened_node_must_be_the_device_sysfs_names() {
        let dri = fixture("dri");
        std::os::unix::fs::symlink("/dev/null", dri.join("renderD128")).unwrap();
        std::fs::write(dri.join("renderD129"), b"x").unwrap();
        let ok = DrmRender::open(
            &dri,
            &DrmNodeId {
                name: "renderD128".into(),
                dev: (1, 3),
            },
        )
        .expect("the file is char 1:3, as stated");
        assert_eq!(ok.id().dev, (1, 3));
        let e = DrmRender::open(
            &dri,
            &DrmNodeId {
                name: "renderD128".into(),
                dev: (226, 128),
            },
        )
        .unwrap_err();
        assert!(
            e.to_string().contains("is not the GPU's render node"),
            "{e}"
        );
        let e = DrmRender::open(
            &dri,
            &DrmNodeId {
                name: "renderD129".into(),
                dev: (0, 0),
            },
        )
        .unwrap_err();
        assert!(e.to_string().contains("not a character device"), "{e}");
        assert!(matches!(
            DrmRender::open(
                &dri,
                &DrmNodeId {
                    name: "../null".into(),
                    dev: (1, 3)
                }
            ),
            Err(DrmError::Refused(_))
        ));
        let _ = std::fs::remove_dir_all(&dri);
    }

    /// ★ Only the listed requests go out, and the import's size field must equal the buffer the
    /// pointer will name — checked before the syscall (`/dev/null` answers every ioctl ENOTTY, so
    /// a request that got through reports a syscall error, one refused here a `Refused`).
    fn nv<'a>(
        size_at: usize,
        params: &'a mut [u8],
        fd_at: usize,
        mem: &'a std::fs::File,
    ) -> NvImport<'a> {
        NvImport {
            ptr_at: 8,
            size_at,
            params,
            fd_at,
            mem: std::os::fd::AsFd::as_fd(mem),
        }
    }

    #[test]
    fn only_the_listed_ioctls_go_out_and_the_import_size_is_checked() {
        let dri = fixture("ioctl");
        std::os::unix::fs::symlink("/dev/null", dri.join("renderD128")).unwrap();
        let r = DrmRender::open(
            &dri,
            &DrmNodeId {
                name: "renderD128".into(),
                dev: (1, 3),
            },
        )
        .unwrap();
        let mut arg = [0u8; 36];
        assert!(matches!(
            r.nvidia_ioctl(0xC024_6444, &mut arg),
            Err(DrmError::Refused(_))
        ));
        assert!(matches!(
            r.nvidia_ioctl(NVIDIA_GEM_IMPORT_NVKMS_MEMORY, &mut arg),
            Err(DrmError::Refused(_))
        ));
        assert!(
            matches!(
                r.nvidia_ioctl(NVIDIA_GET_DEV_INFO, &mut arg),
                Err(DrmError::Raw(_))
            ),
            "a listed request reaches the kernel"
        );
        let mut short = [0u8; 35];
        assert!(matches!(
            r.nvidia_ioctl(NVIDIA_GET_DEV_INFO, &mut short),
            Err(DrmError::Raw(RawError::IoctlSizeMismatch { .. }))
        ));
        let mem = std::fs::File::open("/dev/null").unwrap();
        let mut imp = [0u8; 32];
        imp[16..24].copy_from_slice(&28u64.to_le_bytes());
        let mut params = [0u8; 28];
        assert!(matches!(
            r.nvidia_import(
                NVIDIA_GEM_IMPORT_NVKMS_MEMORY,
                &mut imp,
                nv(16, &mut params, 0, &mem)
            ),
            Err(DrmError::Raw(_))
        ));
        assert_eq!(
            &imp[8..16],
            &[0; 8],
            "the pointer is scrubbed after the call"
        );
        assert_eq!(
            i32::from_le_bytes(params[0..4].try_into().unwrap()),
            mem.as_raw_fd(),
            "the fd field is the borrowed descriptor's number"
        );
        let mut big = [0u8; 64];
        let e = r
            .nvidia_import(
                NVIDIA_GEM_IMPORT_NVKMS_MEMORY,
                &mut imp,
                nv(16, &mut big, 0, &mem),
            )
            .unwrap_err();
        assert!(e.to_string().contains("states 28 bytes"), "{e}");
        assert!(matches!(
            r.nvidia_import(NVIDIA_GET_DEV_INFO, &mut imp, nv(16, &mut params, 0, &mem)),
            Err(DrmError::Refused(_))
        ));
        assert!(matches!(
            r.nvidia_import(
                NVIDIA_GEM_IMPORT_NVKMS_MEMORY,
                &mut imp,
                nv(30, &mut params, 0, &mem)
            ),
            Err(DrmError::Refused(_))
        ));
        assert!(matches!(
            r.nvidia_import(
                NVIDIA_GEM_IMPORT_NVKMS_MEMORY,
                &mut imp,
                nv(16, &mut params, 26, &mem)
            ),
            Err(DrmError::Refused(_))
        ));
        let _ = std::fs::remove_dir_all(&dri);
    }

    #[test]
    fn dev_numbers_split_a_dev_t_like_glibc() {
        // glibc's `gnu_dev_makedev`
        let makedev = |ma: u64, mi: u64| {
            ((ma & 0xfff) << 8) | ((ma >> 12) << 44) | (mi & 0xff) | ((mi >> 8) << 20)
        };
        assert_eq!(dev_numbers(0x103), (1, 3));
        assert_eq!(dev_numbers(makedev(226, 128)), (226, 128));
        assert_eq!(dev_numbers(makedev(4095, 255)), (4095, 255));
        assert_eq!(dev_numbers(makedev(5000, 1_048_575)), (5000, 1_048_575));
    }
}
