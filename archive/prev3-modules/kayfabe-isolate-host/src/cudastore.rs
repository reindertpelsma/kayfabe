//! ★★★★★ **w755i — CAN THE SINGLE STORE BE OWNED BY CUDA AND NAMED BY RM?**
//!
//! > Owner, 2026-09-17: *"a cuda channel not managed by us but libcuda and the regular
//! > scratchpad channel for kernel ce/scrub/other? both share same scratchpad va yes was my
//! > idea"*
//!
//! The intent — one memory, every operation performs on it — is the right one. The mechanism
//! is **one shared OBJECT, not one shared VA space**, and the reason is structural: **guest VA
//! spaces can never share the scratchpad's**. The guest picks its own VAs in its own spaces,
//! which is exactly what `map_store_slice` exists for. So "one object, many address spaces" is
//! already the pattern; CUDA becomes one more consumer of it rather than a new case.
//!
//! # What this probe answers, and why it is one `fstat`
//!
//! For CUDA to own the store, its allocation must be nameable by our RM client. The import
//! path is narrow, and ogkm says exactly how narrow:
//!
//! ```text
//! cliresCtrlCmdOsUnixImportObjectFromFd_IMPL  (os.c:2494)
//!     nvfp = nv_get_file_private(pParams->fd, NV_TRUE, &priv);
//!     if (nvfp->handles == NULL || nvfp->handles[0] == 0) -> NV_ERR_INVALID_PARAMETER
//!
//! nv_get_file_private  (kernel-open/nvidia/nv.c:4096-4106)
//!     if (MAJOR(rdev) != NV_MAJOR_DEVICE_NUMBER)                 -> fail
//!     if (ctl && MINOR(rdev) != NV_MINOR_DEVICE_NUMBER_CONTROL_DEVICE) -> fail
//! ```
//!
//! ⇒ **RM imports only from an `/dev/nvidiactl` fd that RM itself exported.** It is not a
//! general dma-buf importer. So the whole design reduces to one measurable question:
//!
//! > does `cuMemExportToShareableHandle(POSIX_FILE_DESCRIPTOR)` hand back an **nvidiactl**
//! > fd, or a dma-buf?
//!
//! ★ Answerable by `fstat` on the returned fd, before any ioctl is attempted — and the
//! attempt is made anyway, because *"the majors match"* and *"RM accepted it"* are different
//! claims and this campaign has been caught taking the first for the second.
//!
//! ⊘ **If the answer is NO**, the mirror direction becomes the candidate (RM allocates and
//! exports; `cuMemImportFromShareableHandle` imports), and this probe reports that it is
//! untested rather than implying it failed.
//!
//! # ⚠ Why it is a probe and not a change
//!
//! Three times today a design was reasoned to a conclusion that a measurement then refuted.
//! The store's ownership is the most load-bearing decision left; it gets measured first.

use kayfabe_linux_raw::DevDir;

/// The major/minor of `/dev/nvidiactl` on this host, read rather than assumed.
fn nvidiactl_rdev() -> Option<(u64, u64)> {
    let m = std::fs::metadata("/dev/nvidiactl").ok()?;
    use std::os::unix::fs::MetadataExt;
    let rdev = m.rdev();
    // ⊘ The libc encoding, not the kernel's: `major`/`minor` here are glibc's split.
    Some((
        (rdev >> 8) & 0xfff,
        (rdev & 0xff) | ((rdev >> 12) & !0xffu64),
    ))
}

/// `fstat` the fd CUDA handed us and say what KIND of file it is.
fn describe_fd(fd: i32) -> String {
    let p = format!("/proc/self/fd/{fd}");
    let target = std::fs::read_link(&p)
        .map(|t| t.to_string_lossy().into_owned())
        .unwrap_or_else(|e| format!("<unreadable: {e}>"));
    let meta = std::fs::metadata(&p);
    match meta {
        Ok(m) => {
            use std::os::unix::fs::MetadataExt;
            let rdev = m.rdev();
            format!(
                "target={target} rdev={rdev:#x} major={} minor={}",
                (rdev >> 8) & 0xfff,
                (rdev & 0xff) | ((rdev >> 12) & !0xffu64)
            )
        }
        Err(e) => format!("target={target} stat-failed={e}"),
    }
}

/// ★★★★★ The probe. Prints `CS_*` rows; exit 0 means it RAN, not that it passed.
#[must_use]
pub fn cuda_store_probe(gpu: u32) -> i32 {
    println!("CS_PROBE=start gpu={gpu}");

    let Some((ctl_major, ctl_minor)) = nvidiactl_rdev() else {
        println!("CS_RESULT=UNMEASURED:no-/dev/nvidiactl");
        return 1;
    };
    println!("CS_NVIDIACTL major={ctl_major} minor={ctl_minor}");

    let cuda = match kayfabe_cuda::driver_unsafe::Cuda::open() {
        Ok(c) => c,
        Err(e) => {
            println!("CS_RESULT=UNMEASURED:no-libcuda:{e}");
            return 1;
        }
    };
    // ★★★★★ **w755v — `cuInit` FIRST, AND ITS ABSENCE MADE THIS PROBE ANSWER ITS OWN
    // QUESTION WRONG.**
    //
    // `[measured w755v, RTX 3090]` this function opened `libcuda` and went straight to
    // `export_device_allocation`. `cuMemGetAllocationGranularity` answered **rc=3**, which is
    // `CUDA_ERROR_NOT_INITIALIZED` — and the probe printed
    // `CS_RESULT=NO:cuda would not export a device allocation to an fd`.
    //
    // ⊘⊘⊘ **That is a FALSE NEGATIVE stated as a finding**, on the one question the whole
    // store-ownership design turns on. Nothing about export was tested; the call fell over on
    // a precondition and the refusal was reported as CUDA's answer.
    //
    // ⚠ The shape is this campaign's most expensive one — *a refusal is not a measurement* —
    // and it is why an `rc` is printed beside every verdict below rather than folded into a
    // word. An uninitialised driver is now `UNMEASURED`, never `NO`.
    if let Err(e) = cuda.init() {
        println!("CS_INIT=REFUSED {e}");
        println!(
            "CS_RESULT=UNMEASURED:cuInit refused, so nothing about export was tested — \
             ⊘ NOT `NO`: this says nothing about whether CUDA can export"
        );
        return 1;
    }
    println!("CS_INIT=OK");
    // ⊘ A driver without the VMM API is an ANSWER about this host, not a probe failure.
    println!("CS_VMM_API={}", cuda.has_vmm_api());
    if !cuda.has_vmm_api() {
        println!(
            "CS_RESULT=INERT:this libcuda has no cuMemCreate/Export — the CUDA-owned store is not available here"
        );
        return 0;
    }

    // 64 MiB: this measures OWNERSHIP AND NAMING, not capacity.
    let exported = match cuda.export_device_allocation(0, 64 << 20) {
        Ok(e) => e,
        Err(e) => {
            // ⚠ Distinguish a REFUSAL from an UNMEASURED: a precondition error (the driver
            // not initialised, no device) is not CUDA declining to export. `[w755v]` the
            // first version of this probe conflated them and answered `NO` on an rc=3.
            println!("CS_EXPORT=REFUSED {e}");
            println!(
                "CS_RESULT=NO:cuda would not export a device allocation to an fd ⊘ read the \
                 rc above before believing this — a precondition failure is UNMEASURED, not NO"
            );
            return 0;
        }
    };
    println!(
        "CS_EXPORT=OK fd={} bytes={} granularity={}",
        exported.fd, exported.bytes, exported.granularity
    );
    println!("CS_FD {}", describe_fd(exported.fd));

    // ★★★ THE ONE THAT DECIDES IT — and it is checked BEFORE the ioctl, so a refusal can be
    // attributed to the fd's KIND rather than to anything RM did with it.
    let meta = std::fs::metadata(format!("/proc/self/fd/{}", exported.fd));
    let is_nvidiactl = match &meta {
        Ok(m) => {
            use std::os::unix::fs::MetadataExt;
            let rdev = m.rdev();
            let major = (rdev >> 8) & 0xfff;
            let minor = (rdev & 0xff) | ((rdev >> 12) & !0xffu64);
            major == ctl_major && minor == ctl_minor
        }
        Err(_) => false,
    };
    println!("CS_FD_IS_NVIDIACTL={is_nvidiactl}");
    if !is_nvidiactl {
        println!(
            "CS_RESULT=NO:cuMemExportToShareableHandle did not return an /dev/nvidiactl fd. \
             `NV_ESC_RM_IMPORT_OBJECT_FROM_FD` refuses anything else at \
             `nv_get_file_private`'s major/minor gate (kernel-open/nvidia/nv.c:4096-4106), so \
             a CUDA-owned store cannot be named by our RM client THIS WAY. ⊘ The mirror \
             direction — RM allocates and exports, `cuMemImportFromShareableHandle` imports \
             — is UNTESTED by this probe and is the next candidate."
        );
        return 0;
    }

    // ⚠ "The majors match" and "RM accepted it" are different claims. Try the import.
    let Ok(dev) = DevDir::open(c"/dev") else {
        println!("CS_RESULT=UNMEASURED:no-devdir");
        return 1;
    };
    let conn = match crate::rm::RmConnection::open_on_host(&dev, kayfabe_arch::ids::GpuId(gpu)) {
        Ok(c) => c,
        Err(e) => {
            println!("CS_RESULT=UNMEASURED:rm-open:{e}");
            return 1;
        }
    };
    match conn.import_object_from_fd(exported.fd) {
        Ok(h) => {
            println!("CS_IMPORT=OK handle=0x{h:x}");
            println!(
                "CS_RESULT=YES: a CUDA-owned device allocation IS nameable by our RM client. \
                 The single store can be allocated through CUDA — addressable by the walk \
                 kernel LIVE at its own GPGA, by cuMemcpyAsync for the emulated CE plane, and \
                 by `map_store_slice` into guest VA spaces."
            );
        }
        Err(e) => {
            println!("CS_IMPORT=REFUSED {e:?}");
            println!(
                "CS_RESULT=NO: RM refused the import — read the status AND check the \
                 constants before believing it. `[w755v/w755x]` this verdict was reached \
                 THREE times from three different transcription bugs of mine, each producing \
                 a status that a real code path also returns. ⊘ A plausible citation for a \
                 refusal is not evidence the cited path ran. Old text follows: \
                 `[measured w755v, RTX 3090, 580.159.04]` CUDA exports fine and the fd IS an \
                 /dev/nvidiactl fd with matching major/minor, and RM still refuses with \
                 `0x3B NV_ERR_INVALID_PARAMETER` — `os.c:2377`, `nvfp->handles == NULL`. RM \
                 populates `handles[0]` only in its OWN export (`os.c:2291`), so CUDA's \
                 export registers no RM object on that file. ⇒ **USE THE OTHER DIRECTION**: \
                 RM allocates and exports, `cuMemImportFromShareableHandle` imports — \
                 measured below as CS_REVERSE."
            );
        }
    }

    // ★★★★★ **w755w — THE REVERSE LEG, WHICH IS THE DIRECTION THAT CAN WORK.**
    //
    // RM allocates a small vidmem object, exports it to a control fd **we** open, and CUDA
    // imports and maps it. If this yields a device pointer, the single store can be given one
    // — and the walk kernel (`KfWin { base, len }`, dereferenced at **GPGA offsets**) walks
    // the guest's tables **in place**, which is what deletes the CPU walk that CUT A refuses.
    //
    // ⊘ Run unconditionally, whichever way the forward leg went: they are different
    // questions, and the forward leg's `NO` says nothing about this one.
    let store = match conn.reserve_gpga_probe(64 << 20) {
        Ok(h) => h,
        Err(e) => {
            println!("CS_REVERSE=UNMEASURED:rm-alloc:{e:?}");
            return 0;
        }
    };
    let ctl = match kayfabe_linux_raw::CharDevice::openat(&dev, c"nvidiactl") {
        Ok(c) => c,
        Err(e) => {
            println!("CS_REVERSE=UNMEASURED:open-ctl:{e:?}");
            return 0;
        }
    };
    if let Err(e) = conn.export_object_to_fd(store, ctl.fd_number()) {
        // ⊘ UNMEASURED, not NO: nothing about CUDA's import was tested. w755v's lesson,
        // applied to the leg written because of it.
        println!("CS_REVERSE=RM-EXPORT-REFUSED {e:?}");
        println!(
            "CS_REVERSE_RESULT=UNMEASURED:RM would not export to our ctl fd, so CUDA's \
             import was never reached"
        );
        return 0;
    }
    println!(
        "CS_REVERSE=RM-EXPORT-OK object={store:#x} fd={}",
        ctl.fd_number()
    );
    match cuda.import_and_map(0, ctl.fd_number(), 64 << 20) {
        Ok(ptr) => {
            println!("CS_REVERSE=CUDA-IMPORT-OK dptr={ptr:#x}");
            println!(
                "CS_REVERSE_RESULT=YES: an RM-owned device allocation IS addressable by CUDA. \
                 ⇒ the single store can carry a device pointer, the walk kernel can be \
                 pointed at it at its own GPGA offsets, and the staged image + the CPU walk \
                 CUT A refuses both go away."
            );
        }
        Err(e) => {
            println!("CS_REVERSE=CUDA-IMPORT-REFUSED {e}");
            println!(
                "CS_REVERSE_RESULT=NO: RM exported and CUDA would not import it — read the rc \
                 above. ⊘ A precondition rc (NOT_INITIALIZED, INVALID_VALUE on the osHandle \
                 cast) is UNMEASURED, not NO."
            );
        }
    }
    0
}
