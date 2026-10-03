//! ★★★★★ **THE CUDA DRIVER API, `dlopen`ed — AND THE ONLY PLACE A DEVICE OR HOST ADDRESS EXISTS IN
//! THIS CRATE** (`v3-sec-rawaddr`, 2026-10-04; audit S1-03/S1-04; `THE_CONSTRAINTS.md` §13;
//! `OWNER_RULINGS.md` §R).
//!
//! # The perimeter, and its two tiers
//!
//! This file is the audit perimeter of `kf-cuda` (§R: *"this file can violate memory safety"*).
//! Everything that constructs, stores or opens a device address, a host address, or a driver handle
//! lives in the inline module [`raw`] below — **every `unsafe` block of the crate is in it**, and
//! every field of every address-carrying type is private to it. Its two child files are perimeter
//! too, one tier out:
//!
//! - `driver_unsafe/walk_gpu_unsafe.rs` — the walker's GPU half (`WalkGpu`, `DeviceImage`);
//! - `driver_unsafe/display_gpu_unsafe.rs` — the display plane's (`DisplayGpu`, `Composer`,
//!   `ConsoleFrame`).
//!
//! The children call `raw`'s validating methods and nothing else: Rust's privacy (a child module
//! cannot see a sibling's private fields or struct literals) means a child **cannot build** a
//! [`raw`] range around an address of its choosing — the compiler enforces it, not a convention.
//! The children contain no `unsafe` at all.
//!
//! Outside the perimeter (`walk.rs`, `display.rs`, every other crate) no address is held, built or
//! passed: only the opaque handles and offsets into objects named by handle.
//!
//! # The boundary rule
//!
//! Every function reachable from safe code that leads into a driver call validates ALL of its own
//! inputs — offset plus length with `checked_add`, the range inside the real allocation, context
//! identity of every range AND of the stream, kernel argument kinds and capacities, alignment,
//! lifetime (by type) and GPU-flight state (owned here, never promised by a caller). A precondition
//! left to a caller would be *"unsafe code declared as safe"* (§R) and there is none.
//!
//! # Why the driver API and not the runtime API
//!
//! `libcudart` is a second shared object, absent on a machine that has only the driver, and it owns
//! a context lifecycle we do not want. The driver API is what `libcuda.so.1` — the file the
//! **driver package** installs — exports. ⊘ A **musl static-pie** binary cannot `dlopen` at all
//! (`dlerror()` = *"Dynamic loading not supported"*, 2026-09-14, locally, no GPU), so this crate is
//! only ever loaded by a dynamically linked process (QEMU, the harness binaries).
//!
//! ⚠ **Every required symbol is resolved by name and a missing one is a named refusal**, never a
//! null call.

#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

pub(crate) mod display_gpu_unsafe;
#[cfg(test)]
mod tests_unsafe;
pub(crate) mod walk_gpu_unsafe;

pub use raw::CompletionFd;
pub use raw::{KF_ARGS_LAYOUT, KF_WIN_LAYOUT, StructLayout};

/// A CUDA driver-API status. `0` is `CUDA_SUCCESS`.
pub type CUresult = core::ffi::c_int;

/// Why CUDA could not be brought up, or what it (or this crate's perimeter) refused.
///
/// ⊘ No variant carries an address: an error text naming a device pointer or a host address would
/// hand it to safe code as a string (design §1 R1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CudaError {
    /// `libcuda.so.1` could not be loaded. ⊘ Carries `dlerror()` verbatim, because
    /// *"Dynamic loading not supported"* (a musl build) and *"cannot open shared object
    /// file"* (no driver installed) are **different diagnoses**.
    NoLibrary {
        /// What was tried.
        soname: String,
        /// `dlerror()`, verbatim.
        dlerror: String,
    },
    /// A symbol the binding requires is absent from the library that did load.
    MissingSymbol(&'static str),
    /// A driver call refused, or the perimeter refused an input by name.
    Refused {
        /// Which call, or which check.
        what: &'static str,
        /// Its `CUresult` (`0` for a perimeter refusal).
        code: CUresult,
        /// `cuGetErrorName`, or the refusal's reason.
        name: String,
    },
}

impl core::fmt::Display for CudaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CudaError::NoLibrary { soname, dlerror } => {
                write!(f, "could not load {soname}: {dlerror}")
            }
            CudaError::MissingSymbol(s) => write!(f, "libcuda has no symbol {s}"),
            CudaError::Refused { what, code, name } => {
                write!(f, "{what} refused: {code} ({name})")
            }
        }
    }
}

/// A perimeter refusal, by name.
pub(crate) fn refused(what: &'static str, name: String) -> CudaError {
    CudaError::Refused {
        what,
        code: 0,
        name,
    }
}

/// How many device allocations, completion descriptors and other driver resources the perimeter
/// LEAKED because the context could not be drained before their release (failure policy F1:
/// freeing after a failed drain races queued work; closing a descriptor a queued host function will
/// write lets a reused fd number receive it). A count, never an address.
#[must_use]
pub fn driver_leaks() -> u64 {
    raw::leaks()
}

/// ═══ THE RAW TIER ═════════════════════════════════════════════════════════════════════════════
///
/// Every `unsafe` block of `kf-cuda`. Every address-carrying field is private to this module; the
/// child files reach memory only through the methods below, each of which validates its inputs.
#[allow(clippy::module_name_repetitions)]
mod raw {
    use super::{CUresult, CudaError, refused};
    use crate::abi::{KF_MAX_PDB, KfFormat};
    use core::ffi::{c_char, c_int, c_uint, c_void};
    use kf_linux_raw::{Backing, CachePolicy, HostPageSize, HostProt, MappedRegion, StaticSpan};
    use std::ffi::CString;
    use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    /// `CUDA_SUCCESS`.
    const CUDA_SUCCESS: CUresult = 0;
    /// `CUDA_ERROR_NOT_READY` — `cuEventQuery`'s "not yet", which is an answer and not a failure.
    const CUDA_ERROR_NOT_READY: CUresult = 600;
    /// A device address, as the driver spells it. ⊘ Private to this module: nothing outside it can
    /// name the type, let alone hold one.
    type DevAddr = u64;

    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *const c_char;
        fn eventfd(initval: c_uint, flags: c_int) -> c_int;
        fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
        fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
        fn poll(fds: *mut PollFd, nfds: core::ffi::c_ulong, timeout: c_int) -> c_int;
    }

    /// `struct pollfd`.
    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: i16,
        revents: i16,
    }
    /// `POLLIN`.
    const POLLIN: i16 = 0x1;
    /// `EFD_NONBLOCK | EFD_CLOEXEC` (the same values on x86-64 and aarch64).
    const EFD_NONBLOCK_CLOEXEC: c_int = 0o4000 | 0o2_000_000;
    const RTLD_NOW: c_int = 2;
    const RTLD_GLOBAL: c_int = 0x100;
    /// The soname the **driver package** installs (never `libcuda.so`, which only a toolkit has).
    const LIBCUDA_SONAME: &str = "libcuda.so.1";
    /// `CU_STREAM_CAPTURE_MODE_THREAD_LOCAL`.
    const CU_STREAM_CAPTURE_MODE_THREAD_LOCAL: c_uint = 1;
    /// `CU_STREAM_CAPTURE_STATUS_ACTIVE`.
    const CU_STREAM_CAPTURE_STATUS_ACTIVE: c_uint = 1;
    /// `CU_MEM_LOCATION_TYPE_DEVICE`.
    const CU_MEM_LOCATION_TYPE_DEVICE: u32 = 0x1;
    /// `CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR`.
    const CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR: u32 = 0x1;
    /// The architectural maximum of threads per block on every part kayfabe supports.
    pub(in crate::driver_unsafe) const MAX_BLOCK: u32 = 1024;

    // ═══ The binding ═════════════════════════════════════════════════════════════════════════

    /// The subset of the driver API the walker and the display plane need, resolved once.
    ///
    /// ⊘ A struct of function pointers and not an `extern` block: an `extern` block is a link-time
    /// dependency, and every build of this workspace would then need `libcuda`. No `Debug`.
    #[allow(non_snake_case)]
    struct Cuda {
        _handle: *mut c_void,
        cuInit: unsafe extern "C" fn(c_uint) -> CUresult,
        cuDeviceGet: unsafe extern "C" fn(*mut c_int, c_int) -> CUresult,
        cuDeviceGetCount: unsafe extern "C" fn(*mut c_int) -> CUresult,
        cuDeviceGetName: unsafe extern "C" fn(*mut c_char, c_int, c_int) -> CUresult,
        /// ★ Select the device by the host GPU's PCI address, never by ordinal
        /// (V3_MULTI_GPU_AUDIT §2 blocker 1).
        cuDeviceGetByPCIBusId: unsafe extern "C" fn(*mut c_int, *const c_char) -> CUresult,
        cuCtxCreate: unsafe extern "C" fn(*mut *mut c_void, c_uint, c_int) -> CUresult,
        cuCtxDestroy: unsafe extern "C" fn(*mut c_void) -> CUresult,
        cuCtxSynchronize: unsafe extern "C" fn() -> CUresult,
        cuCtxSetCurrent: unsafe extern "C" fn(*mut c_void) -> CUresult,
        cuModuleLoadData: unsafe extern "C" fn(*mut *mut c_void, *const c_void) -> CUresult,
        cuModuleGetFunction:
            unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *const c_char) -> CUresult,
        cuMemAlloc: unsafe extern "C" fn(*mut DevAddr, usize) -> CUresult,
        cuMemFree: unsafe extern "C" fn(DevAddr) -> CUresult,
        cuMemsetD8: unsafe extern "C" fn(DevAddr, u8, usize) -> CUresult,
        cuMemcpyHtoD: unsafe extern "C" fn(DevAddr, *const c_void, usize) -> CUresult,
        cuMemcpyDtoH: unsafe extern "C" fn(*mut c_void, DevAddr, usize) -> CUresult,
        cuLaunchKernel: unsafe extern "C" fn(
            *mut c_void,
            c_uint,
            c_uint,
            c_uint,
            c_uint,
            c_uint,
            c_uint,
            c_uint,
            *mut c_void,
            *mut *mut c_void,
            *mut *mut c_void,
        ) -> CUresult,
        cuGetErrorName: Option<unsafe extern "C" fn(CUresult, *mut *const c_char) -> CUresult>,
        // ★ P4 — the asynchronous walk: a stream of launches ending in a host function that
        // writes an eventfd. Required: every driver since CUDA 10 exports them.
        cuStreamCreate: unsafe extern "C" fn(*mut *mut c_void, c_uint) -> CUresult,
        cuStreamDestroy: unsafe extern "C" fn(*mut c_void) -> CUresult,
        cuEventCreate: unsafe extern "C" fn(*mut *mut c_void, c_uint) -> CUresult,
        cuEventDestroy: unsafe extern "C" fn(*mut c_void) -> CUresult,
        cuEventRecord: unsafe extern "C" fn(*mut c_void, *mut c_void) -> CUresult,
        cuEventQuery: unsafe extern "C" fn(*mut c_void) -> CUresult,
        cuEventElapsedTime: unsafe extern "C" fn(*mut f32, *mut c_void, *mut c_void) -> CUresult,
        cuLaunchHostFunc:
            unsafe extern "C" fn(*mut c_void, extern "C" fn(*mut c_void), *mut c_void) -> CUresult,
        cuMemAllocHost: unsafe extern "C" fn(*mut *mut c_void, usize) -> CUresult,
        cuMemHostGetDevicePointer:
            unsafe extern "C" fn(*mut DevAddr, *mut c_void, c_uint) -> CUresult,
        cuMemcpyDtoHAsync:
            unsafe extern "C" fn(*mut c_void, DevAddr, usize, *mut c_void) -> CUresult,
        cuMemcpyDtoDAsync: unsafe extern "C" fn(DevAddr, DevAddr, usize, *mut c_void) -> CUresult,
        cuMemsetD8Async: unsafe extern "C" fn(DevAddr, u8, usize, *mut c_void) -> CUresult,
        // ★ P4b — the walk as ONE CUDA graph. `Option`: a driver without graphs (pre-11.4)
        // degrades the walker to per-launch submission, visibly, rather than losing it.
        cuStreamBeginCapture: Option<unsafe extern "C" fn(*mut c_void, c_uint) -> CUresult>,
        cuStreamEndCapture: Option<unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> CUresult>,
        cuStreamGetCaptureInfo: Option<
            unsafe extern "C" fn(
                *mut c_void,
                *mut c_uint,
                *mut u64,
                *mut *mut c_void,
                *mut *const *mut c_void,
                *mut usize,
            ) -> CUresult,
        >,
        cuGraphInstantiateWithFlags:
            Option<unsafe extern "C" fn(*mut *mut c_void, *mut c_void, u64) -> CUresult>,
        cuGraphLaunch: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> CUresult>,
        cuGraphExecKernelNodeSetParams: Option<
            unsafe extern "C" fn(*mut c_void, *mut c_void, *const KernelNodeParams) -> CUresult,
        >,
        cuGraphExecDestroy: Option<unsafe extern "C" fn(*mut c_void) -> CUresult>,
        cuGraphDestroy: Option<unsafe extern "C" fn(*mut c_void) -> CUresult>,
        cuGraphUpload: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> CUresult>,
        cuEventRecordWithFlags:
            Option<unsafe extern "C" fn(*mut c_void, *mut c_void, c_uint) -> CUresult>,
        // ★ w755w — the store is exported BY RM and imported BY CUDA. `Option`: absent is an
        // ANSWER about this driver, never a load failure of the walker.
        cuMemImportFromShareableHandle:
            Option<unsafe extern "C" fn(*mut u64, *mut c_void, c_uint) -> CUresult>,
        cuMemAddressReserve:
            Option<unsafe extern "C" fn(*mut u64, usize, usize, u64, u64) -> CUresult>,
        cuMemMap: Option<unsafe extern "C" fn(u64, usize, usize, u64, u64) -> CUresult>,
        cuMemSetAccess: Option<unsafe extern "C" fn(u64, usize, *const c_void, usize) -> CUresult>,
        cuMemUnmap: Option<unsafe extern "C" fn(u64, usize) -> CUresult>,
        cuMemAddressFree: Option<unsafe extern "C" fn(u64, usize) -> CUresult>,
        cuMemRelease: Option<unsafe extern "C" fn(u64) -> CUresult>,
        // ★ v3-sec-rawaddr: the console frame's page-lock registration (`console_pages`).
        cuMemHostRegister: Option<unsafe extern "C" fn(*mut c_void, usize, c_uint) -> CUresult>,
    }

    // SAFETY: every field is a code pointer into a library loaded `RTLD_GLOBAL` for the life of
    // the process. Nothing here is interior-mutable and nothing is freed.
    unsafe impl Send for Cuda {}
    // SAFETY: as above — the struct is immutable after construction.
    unsafe impl Sync for Cuda {}

    /// `dlsym`, returning NULL rather than refusing. ⊘ ONE place: every lookup, required or
    /// optional, shares exactly this `unsafe` (design §7: `sym!`'s own lookup and the closure that
    /// duplicated it were folded here).
    fn sym_or_null(handle: *mut c_void, name: &str) -> *mut c_void {
        let Ok(n) = CString::new(name) else {
            return core::ptr::null_mut();
        };
        // SAFETY: `handle` is a live library handle and `n` is NUL-terminated.
        unsafe { dlsym(handle, n.as_ptr()) }
    }

    impl Cuda {
        /// `dlopen` the driver and resolve every symbol.
        fn open() -> Result<Cuda, CudaError> {
            let soname = LIBCUDA_SONAME;
            let c = CString::new(soname).map_err(|_| CudaError::NoLibrary {
                soname: soname.to_string(),
                dlerror: "the soname contains a NUL".to_string(),
            })?;
            // ⊘ `RTLD_GLOBAL` because the driver's own lazy paths resolve against the global
            // scope; `RTLD_NOW` so a missing symbol is refused HERE rather than at a later call.
            // SAFETY: `c` is a valid NUL-terminated C string that outlives the call.
            let handle = unsafe {
                let _ = dlerror(); // clear any stale error
                dlopen(c.as_ptr(), RTLD_NOW | RTLD_GLOBAL)
            };
            if handle.is_null() {
                // SAFETY: `dlerror` returns either NULL or a NUL-terminated string owned by libdl.
                let e = unsafe {
                    let p = dlerror();
                    if p.is_null() {
                        "dlopen returned NULL and dlerror said nothing".to_string()
                    } else {
                        core::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
                    }
                };
                return Err(CudaError::NoLibrary {
                    soname: soname.to_string(),
                    dlerror: e,
                });
            }
            // ⚠ `_v2` is not decoration: the unsuffixed entry points are the 32-bit-pointer ABI,
            // so every size-carrying entry point is asked for by its versioned name.
            macro_rules! opt {
                ($name:literal) => {{
                    let raw = sym_or_null(handle, $name);
                    if raw.is_null() {
                        None
                    } else {
                        // SAFETY: the driver's ABI for this symbol; `raw` is non-NULL and came
                        // from `dlsym` on a live handle.
                        Some(unsafe { core::mem::transmute(raw) })
                    }
                }};
            }
            // A required symbol: `opt!`, refused by name when absent. ⊘ No second transmute.
            macro_rules! sym {
                ($name:literal) => {{
                    match opt!($name) {
                        Some(f) => f,
                        None => return Err(CudaError::MissingSymbol($name)),
                    }
                }};
            }
            Ok(Cuda {
                _handle: handle,
                cuInit: sym!("cuInit"),
                cuDeviceGet: sym!("cuDeviceGet"),
                cuDeviceGetCount: sym!("cuDeviceGetCount"),
                cuDeviceGetName: sym!("cuDeviceGetName"),
                cuDeviceGetByPCIBusId: sym!("cuDeviceGetByPCIBusId"),
                cuCtxCreate: sym!("cuCtxCreate_v2"),
                cuCtxDestroy: sym!("cuCtxDestroy_v2"),
                cuCtxSynchronize: sym!("cuCtxSynchronize"),
                cuCtxSetCurrent: sym!("cuCtxSetCurrent"),
                cuModuleLoadData: sym!("cuModuleLoadData"),
                cuModuleGetFunction: sym!("cuModuleGetFunction"),
                cuMemAlloc: sym!("cuMemAlloc_v2"),
                cuMemFree: sym!("cuMemFree_v2"),
                cuMemsetD8: sym!("cuMemsetD8_v2"),
                cuMemcpyHtoD: sym!("cuMemcpyHtoD_v2"),
                cuMemcpyDtoH: sym!("cuMemcpyDtoH_v2"),
                cuLaunchKernel: sym!("cuLaunchKernel"),
                cuGetErrorName: opt!("cuGetErrorName"),
                cuStreamCreate: sym!("cuStreamCreate"),
                cuStreamDestroy: sym!("cuStreamDestroy_v2"),
                cuEventCreate: sym!("cuEventCreate"),
                cuEventDestroy: sym!("cuEventDestroy_v2"),
                cuEventRecord: sym!("cuEventRecord"),
                cuEventQuery: sym!("cuEventQuery"),
                cuEventElapsedTime: sym!("cuEventElapsedTime"),
                cuLaunchHostFunc: sym!("cuLaunchHostFunc"),
                cuMemAllocHost: sym!("cuMemAllocHost_v2"),
                cuMemHostGetDevicePointer: sym!("cuMemHostGetDevicePointer_v2"),
                cuMemcpyDtoHAsync: sym!("cuMemcpyDtoHAsync_v2"),
                cuMemcpyDtoDAsync: sym!("cuMemcpyDtoDAsync_v2"),
                cuMemsetD8Async: sym!("cuMemsetD8Async"),
                // ⚠ `cuStreamBeginCapture_v2` takes the capture mode; `_v2` of the setter takes
                // the CUDA 12 struct (a strict superset of v1's, so the fallback reads a prefix).
                cuStreamBeginCapture: opt!("cuStreamBeginCapture_v2"),
                cuStreamEndCapture: opt!("cuStreamEndCapture"),
                cuStreamGetCaptureInfo: opt!("cuStreamGetCaptureInfo_v2"),
                cuGraphInstantiateWithFlags: opt!("cuGraphInstantiateWithFlags"),
                cuGraphLaunch: opt!("cuGraphLaunch"),
                cuGraphExecKernelNodeSetParams: match opt!("cuGraphExecKernelNodeSetParams_v2") {
                    Some(f) => Some(f),
                    None => opt!("cuGraphExecKernelNodeSetParams"),
                },
                cuGraphExecDestroy: opt!("cuGraphExecDestroy"),
                cuGraphDestroy: opt!("cuGraphDestroy"),
                cuGraphUpload: opt!("cuGraphUpload"),
                cuEventRecordWithFlags: opt!("cuEventRecordWithFlags"),
                cuMemImportFromShareableHandle: opt!("cuMemImportFromShareableHandle"),
                cuMemAddressReserve: opt!("cuMemAddressReserve"),
                cuMemMap: opt!("cuMemMap"),
                cuMemSetAccess: opt!("cuMemSetAccess"),
                cuMemUnmap: opt!("cuMemUnmap"),
                cuMemAddressFree: opt!("cuMemAddressFree"),
                cuMemRelease: opt!("cuMemRelease"),
                cuMemHostRegister: opt!("cuMemHostRegister_v2"),
            })
        }

        /// Turn a `CUresult` into a refusal that names the call.
        fn check(&self, what: &'static str, r: CUresult) -> Result<(), CudaError> {
            if r == CUDA_SUCCESS {
                return Ok(());
            }
            let mut p: *const c_char = core::ptr::null();
            let name = match self.cuGetErrorName {
                // SAFETY: `p` is a valid out-pointer; the driver writes a static string into it.
                Some(f) if unsafe { f(r, &raw mut p) } == CUDA_SUCCESS && !p.is_null() => {
                    // SAFETY: the driver guarantees a NUL-terminated static string.
                    unsafe { core::ffi::CStr::from_ptr(p) }
                        .to_string_lossy()
                        .into_owned()
                }
                _ => "unnamed".to_string(),
            };
            Err(CudaError::Refused {
                what,
                code: r,
                name,
            })
        }

        fn has_graph_api(&self) -> bool {
            self.cuStreamBeginCapture.is_some()
                && self.cuStreamEndCapture.is_some()
                && self.cuStreamGetCaptureInfo.is_some()
                && self.cuGraphInstantiateWithFlags.is_some()
                && self.cuGraphLaunch.is_some()
                && self.cuGraphExecKernelNodeSetParams.is_some()
                && self.cuGraphExecDestroy.is_some()
                && self.cuGraphDestroy.is_some()
                && self.cuGraphUpload.is_some()
                && self.cuEventRecordWithFlags.is_some()
        }
    }

    fn need<T: Copy>(f: Option<T>, name: &'static str) -> Result<T, CudaError> {
        f.ok_or(CudaError::MissingSymbol(name))
    }

    /// A process-wide counter that names contexts. ⊘ Never the `CUcontext` value, which is a
    /// pointer into libcuda's heap.
    static NEXT_CTX: AtomicU64 = AtomicU64::new(1);
    /// Failure policy F1: resources leaked because their context could not be drained.
    static LEAKS: AtomicU64 = AtomicU64::new(0);

    pub(super) fn leaks() -> u64 {
        LEAKS.load(Ordering::Relaxed)
    }

    // ═══ Contexts ════════════════════════════════════════════════════════════════════════════

    struct CtxInner {
        cu: Cuda,
        raw: usize,
        device: c_int,
        id: u64,
        name: String,
        sync_calls: AtomicU64,
    }

    impl Drop for CtxInner {
        fn drop(&mut self) {
            // SAFETY: `raw` came from this binding's own `cuCtxCreate_v2` and is destroyed exactly
            // once: `CtxInner` is reachable only through `Arc`s, and this is the last one. Every
            // object of the context (allocations, streams, events, modules, the pinned stage, the
            // console registrations) holds a `Ctx` clone, so none outlives the context.
            unsafe { (self.cu.cuCtxDestroy)(self.raw as *mut c_void) };
        }
    }

    /// ★ A CUDA context. Every object made in it holds a clone, so the context is destroyed only
    /// when the last of them drops — never under a live allocation or stream.
    pub(in crate::driver_unsafe) struct Ctx(Arc<CtxInner>);

    /// ★ PROOF that every operation queued in context `ctx_id` before it was minted has completed.
    /// Minted only by [`Ctx::drain`] and by [`Event::query`] answering *done*; consumed by
    /// [`PinnedStage::end_flight`], the only way a walk's stage returns to `Idle`.
    pub(in crate::driver_unsafe) struct Drained {
        ctx_id: u64,
    }

    impl Ctx {
        /// `cuInit`, the device (by PCI address, or ordinal 0 for single-GPU harnesses), its name,
        /// `cuCtxCreate` (current on this thread).
        pub(in crate::driver_unsafe) fn create(bdf: Option<&str>) -> Result<Ctx, CudaError> {
            let cu = Cuda::open()?;
            // SAFETY: `cuInit` takes an integer and returns a status; no pointers.
            cu.check("cuInit", unsafe { (cu.cuInit)(0) })?;
            let mut n: c_int = 0;
            // SAFETY: one live, aligned out-pointer for the whole call.
            cu.check("cuDeviceGetCount", unsafe {
                (cu.cuDeviceGetCount)(&raw mut n)
            })?;
            if n < 1 {
                return Err(refused(
                    "cuDeviceGetCount",
                    "the driver loaded and reports ZERO devices — a fact about this host, not \
                     about CUDA"
                        .to_string(),
                ));
            }
            let mut device: c_int = 0;
            match bdf {
                None => {
                    // SAFETY: one live out-pointer, no aliasing.
                    cu.check("cuDeviceGet", unsafe {
                        (cu.cuDeviceGet)(&raw mut device, 0)
                    })?;
                }
                Some(bdf) => {
                    let c = CString::new(bdf).map_err(|_| {
                        refused(
                            "cuDeviceGetByPCIBusId",
                            format!("the PCI bus id {bdf:?} contains a NUL"),
                        )
                    })?;
                    // SAFETY: one live out-pointer and a NUL-terminated string outliving the call.
                    cu.check("cuDeviceGetByPCIBusId", unsafe {
                        (cu.cuDeviceGetByPCIBusId)(&raw mut device, c.as_ptr())
                    })?;
                }
            }
            let mut buf = [0u8; 128];
            // SAFETY: the buffer is live for the call and its length is passed as the bound the
            // driver must honour. The bytes are then read by `CStr::from_bytes_until_nul`, which is
            // bounded by the array even if the driver wrote no NUL.
            let r = unsafe { (cu.cuDeviceGetName)(buf.as_mut_ptr().cast::<c_char>(), 128, device) };
            let name = if r == CUDA_SUCCESS {
                core::ffi::CStr::from_bytes_until_nul(&buf).map_or_else(
                    |_| "<unterminated>".to_string(),
                    |c| c.to_string_lossy().into_owned(),
                )
            } else {
                "<unnamed>".to_string()
            };
            let mut ctx: *mut c_void = core::ptr::null_mut();
            // SAFETY: one live out-pointer; the driver writes an opaque handle we never deref.
            cu.check("cuCtxCreate_v2", unsafe {
                (cu.cuCtxCreate)(&raw mut ctx, 0, device)
            })?;
            Ok(Ctx(Arc::new(CtxInner {
                cu,
                raw: ctx as usize,
                device,
                id: NEXT_CTX.fetch_add(1, Ordering::Relaxed),
                name,
                sync_calls: AtomicU64::new(0),
            })))
        }

        fn cu(&self) -> &Cuda {
            &self.0.cu
        }

        fn share(&self) -> Ctx {
            Ctx(Arc::clone(&self.0))
        }

        /// This context's process-local identity (a counter, never the `CUcontext` value).
        pub(in crate::driver_unsafe) fn id(&self) -> u64 {
            self.0.id
        }

        /// The device's name (`cuDeviceGetName`).
        pub(in crate::driver_unsafe) fn device_name(&self) -> &str {
            &self.0.name
        }

        /// Whether every graph entry point the walk's one-call submission needs resolved.
        pub(in crate::driver_unsafe) fn has_graph_api(&self) -> bool {
            self.0.cu.has_graph_api()
        }

        /// ★ `cuCtxSetCurrent` — bind this context to the calling thread. A context is current
        /// PER THREAD (`[measured w731, RTX 3060, 580.159.04]` a worker thread without it got
        /// `CUDA_ERROR_INVALID_CONTEXT` 2 115 times).
        pub(in crate::driver_unsafe) fn make_current(&self) -> Result<(), CudaError> {
            // SAFETY: `raw` came from this library's own `cuCtxCreate_v2` and is alive (this `Ctx`
            // holds the `Arc`). `cuCtxSetCurrent` affects the calling thread only.
            self.cu().check("cuCtxSetCurrent", unsafe {
                (self.0.cu.cuCtxSetCurrent)(self.0.raw as *mut c_void)
            })
        }

        /// ★ Make this context current and wait for everything queued in it (`cuCtxSynchronize`):
        /// the one way to mint a [`Drained`] for the whole context. Counted ([`Ctx::sync_calls`]):
        /// a walk must make none (gate 8, `tests/walk_never_synchronizes.rs`).
        pub(in crate::driver_unsafe) fn drain(&self) -> Result<Drained, CudaError> {
            self.make_current()?;
            self.0.sync_calls.fetch_add(1, Ordering::Relaxed);
            // SAFETY: no arguments; it waits on the context current on this thread (made so above).
            self.cu().check("cuCtxSynchronize", unsafe {
                (self.0.cu.cuCtxSynchronize)()
            })?;
            Ok(Drained { ctx_id: self.0.id })
        }

        /// How many `cuCtxSynchronize` this context has made — gate 8's falsifier.
        pub(in crate::driver_unsafe) fn sync_calls(&self) -> u64 {
            self.0.sync_calls.load(Ordering::Relaxed)
        }
    }

    // ═══ V1 — ranges ════════════════════════════════════════════════════════════════════════

    /// ★★★ **V1, pure: `[off, off+n)` inside a `len`-byte allocation**, `n ≥ 1`, overflow checked
    /// before the bound. An exact fit (`off + n == len`) is inside.
    pub(in crate::driver_unsafe) fn sub_range(len: u64, off: u64, n: u64) -> Option<u64> {
        let end = off.checked_add(n)?;
        (n >= 1 && end <= len).then_some(off)
    }

    /// ★ V1b, pure: every range of an async operation belongs to the stream's context.
    pub(in crate::driver_unsafe) fn ctx_matches(stream_ctx: u64, ranges: &[u64]) -> bool {
        ranges.iter().all(|r| *r == stream_ctx)
    }

    /// What keeps a range's memory alive while a launch (or a graph) can still use it.
    enum Keep {
        Mem(#[allow(dead_code)] Arc<Alloc>),
        Ctx(#[allow(dead_code)] Ctx),
    }

    #[derive(Clone, Copy)]
    enum KeepRef<'a> {
        Mem(&'a Arc<Alloc>),
        Ctx(&'a Ctx),
    }

    impl KeepRef<'_> {
        fn keep(self) -> Keep {
            match self {
                KeepRef::Mem(a) => Keep::Mem(Arc::clone(a)),
                KeepRef::Ctx(c) => Keep::Ctx(c.share()),
            }
        }
    }

    /// ★★ **A validated device range** `[addr, addr+len)`, `len ≥ 1`, inside one live allocation
    /// (or one pinned-stage region) of context `ctx_id`, borrowing what it lies in.
    ///
    /// Minted ONLY by [`DevMem::range`], [`DevMem::whole`], [`DevRange::sub`] and the pinned
    /// stage; its fields are private to this module, so no other code — the perimeter's children
    /// included — can make one. `Copy` because it owns nothing; no `Debug`, no `Hash`.
    #[derive(Clone, Copy)]
    pub(in crate::driver_unsafe) struct DevRange<'a> {
        addr: DevAddr,
        len: u64,
        ctx_id: u64,
        keep: KeepRef<'a>,
    }

    impl<'a> DevRange<'a> {
        /// Bytes.
        pub(in crate::driver_unsafe) fn len(&self) -> u64 {
            self.len
        }

        /// ★ V1 on a range: `[off, off+n)` of THIS range.
        pub(in crate::driver_unsafe) fn sub(
            &self,
            off: u64,
            n: u64,
        ) -> Result<DevRange<'a>, CudaError> {
            let o = sub_range(self.len, off, n).ok_or_else(|| {
                refused(
                    "DevRange::sub (V1)",
                    format!("[{off:#x}, +{n:#x}) leaves a {:#x}-byte range", self.len),
                )
            })?;
            Ok(DevRange {
                addr: self.addr + o,
                len: n,
                ctx_id: self.ctx_id,
                keep: self.keep,
            })
        }

        fn same_ctx(&self, stream: &Stream, what: &'static str) -> Result<(), CudaError> {
            if !ctx_matches(stream.ctx.id(), &[self.ctx_id]) {
                return Err(refused(
                    what,
                    "the range and the stream belong to different CUDA contexts (V1b)".to_string(),
                ));
            }
            Ok(())
        }

        /// ★ V1b: `cuMemcpyDtoDAsync` of this whole range into `dst`, on `stream`. Refused unless
        /// both ranges and the stream share one context, the lengths are equal, and the ranges are
        /// disjoint (the driver's copy is undefined on overlap).
        pub(in crate::driver_unsafe) fn copy_async(
            &self,
            stream: &Stream,
            dst: &DevRange<'_>,
        ) -> Result<(), CudaError> {
            let what = "cuMemcpyDtoDAsync";
            self.same_ctx(stream, what)?;
            dst.same_ctx(stream, what)?;
            if dst.len != self.len {
                return Err(refused(
                    what,
                    format!("{:#x} bytes into a {:#x}-byte range", self.len, dst.len),
                ));
            }
            if !(self.addr + self.len <= dst.addr || dst.addr + dst.len <= self.addr) {
                return Err(refused(
                    what,
                    "the source and destination overlap".to_string(),
                ));
            }
            let n = usize::try_from(self.len).map_err(|_| refused(what, "length".into()))?;
            // SAFETY: both ranges are live allocations of this context (`DevRange`'s invariant;
            // they borrow their owners and `DevMem`'s drop drains before freeing), each exactly `n`
            // bytes, disjoint (checked above); `stream` is a live stream of the same context.
            stream.cu().check(what, unsafe {
                (stream.cu().cuMemcpyDtoDAsync)(dst.addr, self.addr, n, stream.raw as *mut c_void)
            })
        }

        /// ★ V1b: `cuMemsetD8Async` of this whole range to `v`, on `stream`.
        pub(in crate::driver_unsafe) fn fill_async(
            &self,
            stream: &Stream,
            v: u8,
        ) -> Result<(), CudaError> {
            let what = "cuMemsetD8Async";
            self.same_ctx(stream, what)?;
            let n = usize::try_from(self.len).map_err(|_| refused(what, "length".into()))?;
            // SAFETY: `[addr, addr+n)` is a live allocation of this context (`DevRange`); `stream`
            // is a live stream of the same context.
            stream.cu().check(what, unsafe {
                (stream.cu().cuMemsetD8Async)(self.addr, v, n, stream.raw as *mut c_void)
            })
        }

        /// ★★ V1b + V11's copy: this whole range into the console frame `dst`, on `stream`
        /// (`cuMemcpyDtoHAsync`). Refused unless the range, the frame's registration and the
        /// stream share one context and the range fits the frame.
        pub(in crate::driver_unsafe) fn copy_to_console(
            &self,
            stream: &Stream,
            dst: &ConsoleDst,
        ) -> Result<(), CudaError> {
            let what = "cuMemcpyDtoHAsync (console frame)";
            self.same_ctx(stream, what)?;
            if dst.ctx_id != self.ctx_id {
                return Err(refused(
                    what,
                    "the frame is registered in another CUDA context".to_string(),
                ));
            }
            let n = usize::try_from(self.len).map_err(|_| refused(what, "length".into()))?;
            if n > dst.span.len() {
                return Err(refused(
                    what,
                    format!("{n:#x} bytes do not fit a {:#x}-byte frame", dst.span.len()),
                ));
            }
            // SAFETY: destination — `dst.span` is a `StaticSpan` (memory that is never unmapped)
            // page-locked in this context by `console_pages`, and `n <= span.len()` (checked
            // above), so the copy writes only `[ptr, ptr+n)` of it, as `HostSpan::as_ptr`'s
            // contract allows for a registered span. No Rust reference into it exists. Source —
            // `[addr, addr+n)` is a live allocation of this context (`DevRange`).
            stream.cu().check(what, unsafe {
                (stream.cu().cuMemcpyDtoHAsync)(
                    dst.span.host_span().as_ptr().cast::<c_void>(),
                    self.addr,
                    n,
                    stream.raw as *mut c_void,
                )
            })
        }
    }

    // ═══ Device memory ══════════════════════════════════════════════════════════════════════

    enum AllocKind {
        /// `cuMemAlloc`.
        Alloc,
        /// An RM export imported and mapped (`cuMemImportFromShareableHandle` + `cuMemMap`).
        Import {
            map: Option<VaMapping>,
            res: Option<VaReservation>,
        },
    }

    struct Alloc {
        ctx: Ctx,
        addr: DevAddr,
        len: u64,
        kind: AllocKind,
    }

    /// ★ F1, pure: what an owner does with a resource after trying to drain its context.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::driver_unsafe) enum Disposition {
        /// The drain succeeded: nothing queued can still reach the resource; release it.
        Release,
        /// The drain failed: queued work may still reach it. Leak it, and count it.
        Leak,
    }

    /// ★ F1: a failed drain is never "freed anyway".
    pub(in crate::driver_unsafe) fn drop_disposition<T, E>(drain: &Result<T, E>) -> Disposition {
        if drain.is_ok() {
            Disposition::Release
        } else {
            Disposition::Leak
        }
    }

    impl Drop for Alloc {
        fn drop(&mut self) {
            let drained = self.ctx.drain();
            match (drop_disposition(&drained), &mut self.kind) {
                (Disposition::Release, AllocKind::Alloc) => {
                    // SAFETY: `addr` is this allocation's base from `cuMemAlloc` in `ctx`, freed
                    // exactly once (this is the last `Arc` of it); the drain above returned, so no
                    // queued operation can still touch it.
                    unsafe { (self.ctx.cu().cuMemFree)(self.addr) };
                }
                (Disposition::Release, AllocKind::Import { map, res }) => {
                    // The guards release in order: unmap, then the address range.
                    drop(map.take());
                    drop(res.take());
                }
                (Disposition::Leak, AllocKind::Alloc) => {
                    LEAKS.fetch_add(1, Ordering::Relaxed);
                }
                (Disposition::Leak, AllocKind::Import { map, res }) => {
                    core::mem::forget(map.take());
                    core::mem::forget(res.take());
                    LEAKS.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// ★ A device allocation of this crate's: allocated zeroed, or an RM export imported. Not
    /// `Copy`, not `Clone` (a perimeter-internal [`DevMem::share`] keeps it alive for a graph or a
    /// retained window; the memory is released when the last share drops, after a drain).
    pub(in crate::driver_unsafe) struct DevMem {
        a: Arc<Alloc>,
    }

    /// The import's access descriptor (`CUmemAccessDesc`, 12 bytes, transcribed from `cuda.h`).
    #[repr(C)]
    struct AccessDesc {
        location_type: c_uint,
        location_id: c_int,
        flags: c_uint,
    }

    impl DevMem {
        /// `cuMemAlloc_v2` of exactly `len ≥ 1` bytes, then a synchronous zero-fill of all of
        /// them (the kernels read their own state; uninitialised memory would make a first walk
        /// depend on a previous tenant).
        pub(in crate::driver_unsafe) fn alloc_zeroed(
            ctx: &Ctx,
            len: u64,
        ) -> Result<DevMem, CudaError> {
            let what = "cuMemAlloc_v2";
            if len == 0 {
                return Err(refused(what, "a zero-length allocation".to_string()));
            }
            let n = usize::try_from(len).map_err(|_| refused(what, format!("{len:#x} bytes")))?;
            ctx.make_current()?;
            let mut p: DevAddr = 0;
            // SAFETY: one live out-pointer; `n` is a length the driver owns entirely.
            let r = unsafe { (ctx.cu().cuMemAlloc)(&raw mut p, n) };
            ctx.cu().check(what, r)?;
            let m = DevMem {
                a: Arc::new(Alloc {
                    ctx: ctx.share(),
                    addr: p,
                    len,
                    kind: AllocKind::Alloc,
                }),
            };
            m.fill(0, len, 0)?;
            Ok(m)
        }

        /// ★★ **V2 — import an RM-exported object** (`fd`, borrowed for the call) and map `len`
        /// bytes of it, read-write for this context's device. `len ≥ 1`; a negative fd cannot be
        /// expressed (`BorrowedFd`). Every step that fails releases the steps before it (the
        /// guards drop in reverse), and no error text names an address.
        ///
        /// ⚠ CUDA cannot report the size of an imported RM object: that `len` is no larger than
        /// the object is enforced by `cuMemMap`, which refuses an `offset + size` past the
        /// allocation — UNVERIFIED on the target drivers (design §2.6 Res-1, hardware row H1, a
        /// merge blocker). Callers pass RM's own allocation length.
        pub(in crate::driver_unsafe) fn import(
            ctx: &Ctx,
            fd: BorrowedFd<'_>,
            len: u64,
        ) -> Result<DevMem, CudaError> {
            let what = "cuMemImportFromShareableHandle + cuMemMap";
            if len == 0 {
                return Err(refused(what, "a zero-length import".to_string()));
            }
            let n = usize::try_from(len).map_err(|_| refused(what, format!("{len:#x} bytes")))?;
            let cu = ctx.cu();
            let import = need(
                cu.cuMemImportFromShareableHandle,
                "cuMemImportFromShareableHandle",
            )?;
            let reserve = need(cu.cuMemAddressReserve, "cuMemAddressReserve")?;
            let map = need(cu.cuMemMap, "cuMemMap")?;
            let set_access = need(cu.cuMemSetAccess, "cuMemSetAccess")?;
            need(cu.cuMemUnmap, "cuMemUnmap")?;
            need(cu.cuMemAddressFree, "cuMemAddressFree")?;
            need(cu.cuMemRelease, "cuMemRelease")?;
            ctx.make_current()?;
            let raw_fd = fd.as_raw_fd();
            let mut handle: u64 = 0;
            // SAFETY: `handle` is a live local. For `POSIX_FILE_DESCRIPTOR` the `osHandle` is the
            // fd NUMBER cast to a pointer (never dereferenced); `fd` is borrowed, so it is open for
            // the whole call, and non-negative by `BorrowedFd`'s own invariant.
            cu.check("cuMemImportFromShareableHandle", unsafe {
                import(
                    &raw mut handle,
                    raw_fd as usize as *mut c_void,
                    CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR,
                )
            })?;
            let handle = ImportHandle {
                ctx: ctx.share(),
                handle,
            };
            let mut addr: u64 = 0;
            // SAFETY: `addr` is a live local; alignment 0 lets CUDA choose.
            cu.check("cuMemAddressReserve", unsafe {
                reserve(&raw mut addr, n, 0, 0, 0)
            })?;
            let res = VaReservation {
                ctx: ctx.share(),
                addr,
                len: n,
            };
            // SAFETY: `addr` is a reservation of exactly `n` bytes made above and `handle` a live
            // imported handle; the driver refuses a size past the imported object (Res-1).
            cu.check("cuMemMap", unsafe { map(addr, n, 0, handle.handle, 0) })?;
            let mapping = VaMapping {
                ctx: ctx.share(),
                addr,
                len: n,
            };
            let desc = AccessDesc {
                location_type: CU_MEM_LOCATION_TYPE_DEVICE,
                location_id: ctx.0.device,
                flags: 3, // CU_MEM_ACCESS_FLAGS_PROT_READWRITE
            };
            // SAFETY: one live `AccessDesc` for the mapped range of `n` bytes made above.
            cu.check("cuMemSetAccess", unsafe {
                set_access(addr, n, (&raw const desc).cast::<c_void>(), 1)
            })?;
            // The mapping keeps the memory; the import handle is released now (its guard drops).
            drop(handle);
            Ok(DevMem {
                a: Arc::new(Alloc {
                    ctx: ctx.share(),
                    addr,
                    len,
                    kind: AllocKind::Import {
                        map: Some(mapping),
                        res: Some(res),
                    },
                }),
            })
        }

        /// Bytes.
        pub(in crate::driver_unsafe) fn len(&self) -> u64 {
            self.a.len
        }

        /// The context it was made in.
        pub(in crate::driver_unsafe) fn ctx_id(&self) -> u64 {
            self.a.ctx.id()
        }

        /// A second owner of the same memory (refcounted; never exposed to safe code).
        pub(in crate::driver_unsafe) fn share(&self) -> DevMem {
            DevMem {
                a: Arc::clone(&self.a),
            }
        }

        /// ★★★ **V1 — `[off, off+n)` of this allocation**, refused by name unless wholly inside.
        pub(in crate::driver_unsafe) fn range(
            &self,
            off: u64,
            n: u64,
        ) -> Result<DevRange<'_>, CudaError> {
            let o = sub_range(self.a.len, off, n).ok_or_else(|| {
                refused(
                    "DevMem::range (V1)",
                    format!(
                        "[{off:#x}, +{n:#x}) leaves the {:#x}-byte allocation",
                        self.a.len
                    ),
                )
            })?;
            Ok(DevRange {
                addr: self.a.addr + o,
                len: n,
                ctx_id: self.a.ctx.id(),
                keep: KeepRef::Mem(&self.a),
            })
        }

        /// The whole allocation as a range (never empty: every constructor refuses `len == 0`).
        pub(in crate::driver_unsafe) fn whole(&self) -> DevRange<'_> {
            DevRange {
                addr: self.a.addr,
                len: self.a.len,
                ctx_id: self.a.ctx.id(),
                keep: KeepRef::Mem(&self.a),
            }
        }

        /// `cuMemcpyHtoD_v2` of `bytes` to `[off, off+len)` (V1). Synchronous, on the legacy
        /// stream — which the walker's blocking stream is ordered behind.
        pub(in crate::driver_unsafe) fn write(
            &self,
            off: u64,
            bytes: &[u8],
        ) -> Result<(), CudaError> {
            if bytes.is_empty() {
                return Ok(());
            }
            let r = self.range(off, bytes.len() as u64)?;
            self.a.ctx.make_current()?;
            // SAFETY: destination — `r` is inside this live allocation (V1 above) and exactly
            // `bytes.len()` long; source — `bytes` is a live slice of that length.
            self.a.ctx.cu().check("cuMemcpyHtoD_v2", unsafe {
                (self.a.ctx.cu().cuMemcpyHtoD)(r.addr, bytes.as_ptr().cast::<c_void>(), bytes.len())
            })
        }

        /// `cuMemcpyDtoH_v2` of `[off, off+buf.len())` into `buf` (V1). Synchronous.
        pub(in crate::driver_unsafe) fn read(
            &self,
            off: u64,
            buf: &mut [u8],
        ) -> Result<(), CudaError> {
            if buf.is_empty() {
                return Ok(());
            }
            let r = self.range(off, buf.len() as u64)?;
            self.a.ctx.make_current()?;
            // SAFETY: destination — `buf` is a live, exclusively borrowed slice and the byte count
            // is its own length; source — `r` is inside this live allocation (V1).
            self.a.ctx.cu().check("cuMemcpyDtoH_v2", unsafe {
                (self.a.ctx.cu().cuMemcpyDtoH)(buf.as_mut_ptr().cast::<c_void>(), r.addr, buf.len())
            })
        }

        /// `cuMemsetD8_v2` of `[off, off+n)` to `v` (V1). Synchronous.
        pub(in crate::driver_unsafe) fn fill(
            &self,
            off: u64,
            n: u64,
            v: u8,
        ) -> Result<(), CudaError> {
            if n == 0 {
                return Ok(());
            }
            let r = self.range(off, n)?;
            let count =
                usize::try_from(n).map_err(|_| refused("cuMemsetD8_v2", "length".into()))?;
            self.a.ctx.make_current()?;
            // SAFETY: `[r.addr, r.addr+count)` is inside this live allocation (V1 above).
            self.a.ctx.cu().check("cuMemsetD8_v2", unsafe {
                (self.a.ctx.cu().cuMemsetD8)(r.addr, v, count)
            })
        }
    }

    /// The imported RM object's handle; released (`cuMemRelease`) when the import is done with it.
    struct ImportHandle {
        ctx: Ctx,
        handle: u64,
    }

    impl Drop for ImportHandle {
        fn drop(&mut self) {
            if let Some(f) = self.ctx.cu().cuMemRelease {
                // SAFETY: `handle` came from `cuMemImportFromShareableHandle` in this context and is
                // released exactly once (this guard is its only owner and is not `Clone`). A live
                // mapping of it keeps the memory, as the VMM API specifies.
                unsafe { f(self.handle) };
            }
        }
    }

    /// An address reservation (`cuMemAddressReserve`); freed on drop (`cuMemAddressFree`).
    struct VaReservation {
        ctx: Ctx,
        addr: u64,
        len: usize,
    }

    impl Drop for VaReservation {
        fn drop(&mut self) {
            if let Some(f) = self.ctx.cu().cuMemAddressFree {
                // SAFETY: `[addr, addr+len)` is exactly the reservation `cuMemAddressReserve`
                // returned in this context; this guard is its only owner. Its mapping, if any, is
                // a separate guard that drops (unmaps) first.
                unsafe { f(self.addr, self.len) };
            }
        }
    }

    /// A mapping of an imported object into a reservation (`cuMemMap`); unmapped on drop.
    struct VaMapping {
        ctx: Ctx,
        addr: u64,
        len: usize,
    }

    impl Drop for VaMapping {
        fn drop(&mut self) {
            if let Some(f) = self.ctx.cu().cuMemUnmap {
                // SAFETY: `[addr, addr+len)` is exactly the range `cuMemMap` mapped in this
                // context; this guard is its only owner, and its owner drained the context first.
                unsafe { f(self.addr, self.len) };
            }
        }
    }

    // ═══ Streams and events ═════════════════════════════════════════════════════════════════

    /// ★ A blocking CUDA stream (ordered against the legacy stream, so a synchronous copy issued
    /// before a walk is complete before the walk reads it). Keeps every completion descriptor it
    /// was asked to signal alive until its drop drains the context (F1).
    pub(in crate::driver_unsafe) struct Stream {
        ctx: Ctx,
        raw: usize,
        capturing: AtomicBool,
        signals: Mutex<Vec<Arc<OwnedFd>>>,
    }

    impl Stream {
        /// `cuStreamCreate(flags = 0)`.
        pub(in crate::driver_unsafe) fn create(ctx: &Ctx) -> Result<Stream, CudaError> {
            ctx.make_current()?;
            let mut h: *mut c_void = core::ptr::null_mut();
            // SAFETY: one live out-pointer; the driver writes an opaque handle.
            ctx.cu().check("cuStreamCreate", unsafe {
                (ctx.cu().cuStreamCreate)(&raw mut h, 0)
            })?;
            Ok(Stream {
                ctx: ctx.share(),
                raw: h as usize,
                capturing: AtomicBool::new(false),
                signals: Mutex::new(Vec::new()),
            })
        }

        fn cu(&self) -> &Cuda {
            self.ctx.cu()
        }

        /// ★★★ `cuLaunchHostFunc(stream, signal, fd)` — once everything queued before it has
        /// completed, a driver thread writes `1` to `fd`. The stream keeps the descriptor alive
        /// (an `Arc`) until its own drop has drained the context, so a queued — or, in a graph,
        /// replayed — signal never writes into a closed or reused fd number.
        pub(in crate::driver_unsafe) fn host_signal(
            &self,
            fd: &CompletionFd,
        ) -> Result<(), CudaError> {
            {
                let mut s = self
                    .signals
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !s.iter().any(|k| Arc::ptr_eq(k, &fd.fd)) {
                    s.push(Arc::clone(&fd.fd));
                }
            }
            // SAFETY: `raw` is this binding's live stream. The user datum is the fd NUMBER cast to
            // a pointer, never dereferenced by `completion_hostfn`; the descriptor is kept open by
            // the `Arc` this stream now holds, released only after a successful drain (Drop).
            self.cu().check("cuLaunchHostFunc", unsafe {
                (self.cu().cuLaunchHostFunc)(
                    self.raw as *mut c_void,
                    completion_hostfn,
                    usize::try_from(fd.fd.as_raw_fd()).unwrap_or(usize::MAX) as *mut c_void,
                )
            })
        }

        /// `cuStreamBeginCapture_v2(THREAD_LOCAL)`: every operation queued from here to
        /// [`Stream::end_capture`] is recorded into a graph, not executed.
        pub(in crate::driver_unsafe) fn begin_capture(&self) -> Result<(), CudaError> {
            let f = need(self.cu().cuStreamBeginCapture, "cuStreamBeginCapture_v2")?;
            // SAFETY: `raw` is this binding's live stream; the mode is a documented enumerator.
            self.cu().check("cuStreamBeginCapture_v2", unsafe {
                f(self.raw as *mut c_void, CU_STREAM_CAPTURE_MODE_THREAD_LOCAL)
            })?;
            self.capturing.store(true, Ordering::Release);
            Ok(())
        }

        fn is_capturing(&self) -> bool {
            self.capturing.load(Ordering::Acquire)
        }

        /// The node the capture most recently added (a linear capture has exactly one leaf).
        fn capture_leaf(&self) -> Result<usize, CudaError> {
            let f = need(
                self.cu().cuStreamGetCaptureInfo,
                "cuStreamGetCaptureInfo_v2",
            )?;
            let mut status: c_uint = 0;
            let mut id: u64 = 0;
            let mut g: *mut c_void = core::ptr::null_mut();
            let mut deps: *const *mut c_void = core::ptr::null();
            let mut n: usize = 0;
            // SAFETY: `raw` is our stream; five live out-pointers. `deps` points at driver-owned
            // storage valid until the next capture call on this stream, and is read once below.
            let r = unsafe {
                f(
                    self.raw as *mut c_void,
                    &raw mut status,
                    &raw mut id,
                    &raw mut g,
                    &raw mut deps,
                    &raw mut n,
                )
            };
            self.cu().check("cuStreamGetCaptureInfo_v2", r)?;
            if status != CU_STREAM_CAPTURE_STATUS_ACTIVE || n != 1 || deps.is_null() {
                return Err(refused(
                    "cuStreamGetCaptureInfo_v2",
                    format!(
                        "expected an ACTIVE linear capture with exactly one leaf; status={status} \
                         leaves={n}"
                    ),
                ));
            }
            // SAFETY: `n == 1` and `deps` is non-NULL, so `deps[0]` is a valid element.
            let node = unsafe { *deps };
            Ok(node as usize)
        }

        /// ★ End the capture and instantiate it. ⚠ Must be called on every path out of a capture,
        /// failing ones included; the stream leaves capture mode either way. `nodes` are the
        /// launches recorded during the capture ([`Kernel::launch`]'s `Some`), kept by the graph so
        /// a per-walk argument change is one setter per node, and so every allocation they name
        /// stays alive for the graph's life.
        pub(in crate::driver_unsafe) fn end_capture(
            &self,
            nodes: Vec<Captured>,
        ) -> Result<GraphExec, CudaError> {
            let end = need(self.cu().cuStreamEndCapture, "cuStreamEndCapture")?;
            let inst = need(
                self.cu().cuGraphInstantiateWithFlags,
                "cuGraphInstantiateWithFlags",
            )?;
            let mut g: *mut c_void = core::ptr::null_mut();
            // SAFETY: `raw` is our stream; one live out-pointer.
            let r = unsafe { end(self.raw as *mut c_void, &raw mut g) };
            self.capturing.store(false, Ordering::Release);
            let mut exec = GraphExec {
                ctx: self.ctx.share(),
                graph: g as usize,
                exec: 0,
                nodes,
                baked: None,
                updates: 0,
            };
            self.cu().check("cuStreamEndCapture", r)?;
            let mut e: *mut c_void = core::ptr::null_mut();
            // SAFETY: `graph` came from the capture above; one live out-pointer.
            self.cu().check("cuGraphInstantiateWithFlags", unsafe {
                inst(&raw mut e, exec.graph as *mut c_void, 0)
            })?;
            exec.exec = e as usize;
            Ok(exec)
        }
    }

    impl Drop for Stream {
        fn drop(&mut self) {
            let drained = self.ctx.drain();
            let signals = core::mem::take(
                &mut *self
                    .signals
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            if drop_disposition(&drained) == Disposition::Leak {
                // F1: a host function may still be queued; never close the descriptor it names.
                for s in signals {
                    core::mem::forget(s);
                }
                LEAKS.fetch_add(1, Ordering::Relaxed);
            }
            // SAFETY: `raw` came from `Stream::create` and is destroyed exactly once, here.
            unsafe { (self.ctx.cu().cuStreamDestroy)(self.raw as *mut c_void) };
        }
    }

    /// A CUDA event (timing enabled).
    pub(in crate::driver_unsafe) struct Event {
        ctx: Ctx,
        raw: usize,
    }

    impl Event {
        /// `cuEventCreate(flags = 0)`.
        pub(in crate::driver_unsafe) fn create(ctx: &Ctx) -> Result<Event, CudaError> {
            ctx.make_current()?;
            let mut h: *mut c_void = core::ptr::null_mut();
            // SAFETY: one live out-pointer.
            ctx.cu().check("cuEventCreate", unsafe {
                (ctx.cu().cuEventCreate)(&raw mut h, 0)
            })?;
            Ok(Event {
                ctx: ctx.share(),
                raw: h as usize,
            })
        }

        /// Record on `stream` — under capture as an EXTERNAL record, which becomes a graph node (a
        /// plain record during capture only expresses a dependency).
        pub(in crate::driver_unsafe) fn record(&self, stream: &Stream) -> Result<(), CudaError> {
            if stream.ctx.id() != self.ctx.id() {
                return Err(refused(
                    "cuEventRecord",
                    "the event and the stream belong to different CUDA contexts".to_string(),
                ));
            }
            if stream.is_capturing() {
                let f = need(
                    self.ctx.cu().cuEventRecordWithFlags,
                    "cuEventRecordWithFlags",
                )?;
                // SAFETY: both handles came from this binding; `1` is `CU_EVENT_RECORD_EXTERNAL`.
                self.ctx.cu().check("cuEventRecordWithFlags", unsafe {
                    f(self.raw as *mut c_void, stream.raw as *mut c_void, 1)
                })
            } else {
                // SAFETY: both handles came from this binding and are live.
                self.ctx.cu().check("cuEventRecord", unsafe {
                    (self.ctx.cu().cuEventRecord)(
                        self.raw as *mut c_void,
                        stream.raw as *mut c_void,
                    )
                })
            }
        }

        /// `cuEventQuery` — never blocks. `Ok(Some(proof))`: everything recorded before the event
        /// completed; `Ok(None)`: not yet; `Err`: the stream failed.
        pub(in crate::driver_unsafe) fn query(&self) -> Result<Option<Drained>, CudaError> {
            // SAFETY: `raw` came from this binding and is live.
            let r = unsafe { (self.ctx.cu().cuEventQuery)(self.raw as *mut c_void) };
            if r == CUDA_ERROR_NOT_READY {
                return Ok(None);
            }
            self.ctx.cu().check("cuEventQuery", r)?;
            Ok(Some(Drained {
                ctx_id: self.ctx.id(),
            }))
        }

        /// `cuEventElapsedTime(self, end)` in microseconds. Both must have completed.
        pub(in crate::driver_unsafe) fn elapsed_us(&self, end: &Event) -> Result<u64, CudaError> {
            let mut ms: f32 = 0.0;
            // SAFETY: one live out-pointer; both handles came from this binding.
            self.ctx.cu().check("cuEventElapsedTime", unsafe {
                (self.ctx.cu().cuEventElapsedTime)(
                    &raw mut ms,
                    self.raw as *mut c_void,
                    end.raw as *mut c_void,
                )
            })?;
            Ok((f64::from(ms) * 1000.0) as u64)
        }
    }

    impl Drop for Event {
        fn drop(&mut self) {
            // SAFETY: `raw` came from `Event::create` and is destroyed exactly once, here.
            unsafe { (self.ctx.cu().cuEventDestroy)(self.raw as *mut c_void) };
        }
    }

    // ═══ Modules, kernels and V3 ════════════════════════════════════════════════════════════

    /// Which committed PTX to load. ⊘ An enum, not bytes: no other program can reach the JIT.
    #[derive(Clone, Copy)]
    pub(in crate::driver_unsafe) enum Ptx {
        /// `cuda/walk/kf_walk.ptx` ([`crate::walk::WALK_PTX`]).
        Walk,
        /// `cuda/display/kf_scanout.ptx` ([`crate::display::SCANOUT_PTX`]).
        Scanout,
    }

    /// A loaded module (reclaimed by `cuCtxDestroy`; holds the context).
    pub(in crate::driver_unsafe) struct Module {
        ctx: Ctx,
        raw: usize,
    }

    impl Module {
        /// `cuModuleLoadData` over the committed PTX plus a NUL — **the PTX JIT runs here**.
        pub(in crate::driver_unsafe) fn load(ctx: &Ctx, ptx: Ptx) -> Result<Module, CudaError> {
            let bytes = match ptx {
                Ptx::Walk => crate::walk::WALK_PTX,
                Ptx::Scanout => crate::display::SCANOUT_PTX,
            };
            let mut image = bytes.to_vec();
            image.push(0);
            ctx.make_current()?;
            let mut m: *mut c_void = core::ptr::null_mut();
            // SAFETY: `image` is live for the call and NUL-terminated (pushed above), which is the
            // driver's documented requirement; `m` is a live out-pointer.
            ctx.cu().check("cuModuleLoadData", unsafe {
                (ctx.cu().cuModuleLoadData)(&raw mut m, image.as_ptr().cast::<c_void>())
            })?;
            Ok(Module {
                ctx: ctx.share(),
                raw: m as usize,
            })
        }

        /// ★ The entry `name`, with the argument signature [`KERNEL_SIGS`] pins for it. A name the
        /// table does not know is refused before the driver is asked.
        pub(in crate::driver_unsafe) fn kernel(
            &self,
            name: &'static str,
        ) -> Result<Kernel, CudaError> {
            let sig = sig_of(name).ok_or(CudaError::MissingSymbol(name))?;
            let c = CString::new(name).map_err(|_| CudaError::MissingSymbol(name))?;
            let mut f: *mut c_void = core::ptr::null_mut();
            // SAFETY: `raw` is this binding's live module, `c` is NUL-terminated and live for the
            // call, and `f` is a live out-pointer.
            let r = unsafe {
                (self.ctx.cu().cuModuleGetFunction)(&raw mut f, self.raw as *mut c_void, c.as_ptr())
            };
            self.ctx
                .cu()
                .check("cuModuleGetFunction", r)
                .map_err(|e| match e {
                    CudaError::Refused { code, .. } => CudaError::Refused {
                        what: "cuModuleGetFunction",
                        code,
                        name: format!("the committed PTX has no entry `{name}`"),
                    },
                    other => other,
                })?;
            Ok(Kernel {
                ctx: self.ctx.share(),
                raw: f as usize,
                name,
                sig,
            })
        }
    }

    /// ★ One parameter kind of a kernel, as its PTX declares it (pinned by
    /// `tests_unsafe.rs::the_kernel_table_is_the_ptx`, T4).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::driver_unsafe) enum ArgKind {
        /// The by-value `KfArgs` (`.param .align 8 .b8 X[264]`), only as an [`ArgBlock`].
        Block,
        /// A device pointer (`.param .u64`) to at least `min` bytes.
        Ptr {
            /// Bytes the kernel may address from it.
            min: u64,
        },
        /// A `.u32` that is the CAPACITY of pointer parameter `of`, in `elem`-byte elements —
        /// only an [`Arg::CapOf`] of that very range, so a capacity is never larger than its
        /// buffer.
        Cap {
            /// The pointer parameter it bounds.
            of: usize,
            /// Bytes per element.
            elem: u64,
        },
        /// A plain `.u32` scalar.
        U32,
        /// A plain `.s32` scalar.
        I32,
    }

    /// Bytes of `KfArgs` (`kf_walk.ptx:59`, `.b8 …_param_0[264]`).
    pub(in crate::driver_unsafe) const KF_ARGS_BYTES: usize = 264;

    /// The walk's frontier capacity and the leaf phase's staging capacity (`kf_walk.cu`
    /// `KF_MAX_FRONTIER`, `KF_MAX_SCRATCH`), and the sizes of its two device structs — pinned to
    /// the `.cu` by T6 (`walk_gpu_unsafe.rs`).
    pub(in crate::driver_unsafe) const KF_MAX_FRONTIER: u64 = 131_072;
    pub(in crate::driver_unsafe) const KF_MAX_SCRATCH: u64 = 4 << 20;
    pub(in crate::driver_unsafe) const KF_ENT_BYTES: u64 = 40;
    pub(in crate::driver_unsafe) const KF_SUM_BYTES: u64 = 64;
    pub(in crate::driver_unsafe) const KF_RUN_BYTES: u64 = 32;
    const F: u64 = KF_MAX_FRONTIER;

    const P_ENTS: ArgKind = ArgKind::Ptr {
        min: F * KF_ENT_BYTES,
    };
    const P_WORDS: ArgKind = ArgKind::Ptr { min: F * 4 };
    const P_ONE: ArgKind = ArgKind::Ptr { min: 4 };
    const P_SUMS: ArgKind = ArgKind::Ptr {
        min: F * KF_SUM_BYTES,
    };
    const P_HEADS: ArgKind = ArgKind::Ptr { min: F };
    const P_PDBBASE: ArgKind = ArgKind::Ptr {
        min: KF_MAX_PDB as u64 * 4,
    };
    const P_RUNSTAGE: ArgKind = ArgKind::Ptr {
        min: KF_MAX_SCRATCH * KF_RUN_BYTES,
    };
    const B: ArgKind = ArgKind::Block;
    const U: ArgKind = ArgKind::U32;
    const I: ArgKind = ArgKind::I32;

    /// ★★★ **Every kernel this crate launches, and its argument signature** — the PTX's own
    /// declaration (T4) plus, per pointer, the bytes the kernel's loops can address (bounded by
    /// the `.cu`'s compile-time constants), and per capacity, the buffer it bounds. V3 checks every
    /// launch against this table.
    pub(in crate::driver_unsafe) static KERNEL_SIGS: &[(&str, &[ArgKind])] = &[
        ("_Z15kf_begin_kernel6KfArgs", &[B]),
        ("_Z14kf_walk_kernel6KfArgs", &[B]),
        ("_Z13kf_diff_slots6KfArgs", &[B]),
        ("_Z12kf_diff_emit6KfArgs", &[B]),
        ("_Z16kf_commit_kernel6KfArgs", &[B]),
        // (KfArgs, KfEnt *fr, uint *nfr, uint *used): fr[t], t < npdb <= 64; *nfr; used[0..2].
        (
            "_Z11kf_par_seed6KfArgsP5KfEntPjS2_",
            &[B, P_ENTS, P_ONE, ArgKind::Ptr { min: 8 }],
        ),
        // (KfArgs, level, in, nin, stage, stagecap, used, start, cnt)
        (
            "_Z13kf_par_expand6KfArgsjPK5KfEntPKjPS0_jPjS6_S6_",
            &[
                B,
                U,
                P_ENTS,
                P_ONE,
                ArgKind::Ptr { min: KF_ENT_BYTES },
                ArgKind::Cap {
                    of: 4,
                    elem: KF_ENT_BYTES,
                },
                P_ONE,
                P_WORDS,
                P_WORDS,
            ],
        ),
        // (in, nin, out, total)
        (
            "_Z11kf_par_scanPKjS0_PjS1_",
            &[P_WORDS, P_ONE, P_WORDS, P_ONE],
        ),
        // (KfArgs, stage, start, cnt, off, nin, dst, cap)
        (
            "_Z14kf_par_compact6KfArgsPK5KfEntPKjS4_S4_S4_PS0_j",
            &[
                B,
                P_ENTS,
                P_WORDS,
                P_WORDS,
                P_WORDS,
                P_ONE,
                ArgKind::Ptr { min: KF_ENT_BYTES },
                ArgKind::Cap {
                    of: 6,
                    elem: KF_ENT_BYTES,
                },
            ],
        ),
        // (KfArgs, task, nt, runstage, stagecap, used, sum)
        (
            "_Z11kf_par_leaf6KfArgsPK5KfEntPKjP8KfMapRunjPjP5KfSum",
            &[
                B,
                P_ENTS,
                P_ONE,
                P_RUNSTAGE,
                ArgKind::Cap {
                    of: 3,
                    elem: KF_RUN_BYTES,
                },
                P_ONE,
                P_SUMS,
            ],
        ),
        // (KfArgs, task, sum, nt, contrib, head)
        (
            "_Z12kf_par_heads6KfArgsPK5KfEntPK5KfSumPKjPjPh",
            &[B, P_ENTS, P_SUMS, P_ONE, P_WORDS, P_HEADS],
        ),
        // (KfArgs, pdbbase)
        ("_Z12kf_par_bases6KfArgsPj", &[B, P_PDBBASE]),
        // (KfArgs, task, sum, nt, off, head, pdbbase, runstage)
        (
            "_Z11kf_par_emit6KfArgsPK5KfEntPK5KfSumPKjS7_PKhS7_PK8KfMapRun",
            &[
                B, P_ENTS, P_SUMS, P_ONE, P_WORDS, P_HEADS, P_PDBBASE, P_RUNSTAGE,
            ],
        ),
        // (KfArgs, task, sum, nt, off, head, pdbbase)
        (
            "_Z11kf_par_join6KfArgsPK5KfEntPK5KfSumPKjS7_PKhS7_",
            &[B, P_ENTS, P_SUMS, P_ONE, P_WORDS, P_HEADS, P_PDBBASE],
        ),
        // kf_compose (display.rs): src, dst, 12 × u32, 4 × s32. Its extents are V10's.
        (
            "kf_compose",
            &[
                ArgKind::Ptr { min: 4 },
                ArgKind::Ptr { min: 4 },
                U,
                U,
                U,
                U,
                U,
                U,
                U,
                U,
                U,
                U,
                U,
                U,
                I,
                I,
                I,
                I,
            ],
        ),
    ];

    fn sig_of(name: &str) -> Option<&'static [ArgKind]> {
        KERNEL_SIGS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, s)| *s)
    }

    /// ★ One kernel argument. Pointers are only [`DevRange`]s; a capacity only the length of one.
    pub(in crate::driver_unsafe) enum Arg<'a> {
        /// The by-value `KfArgs`.
        Block(&'a ArgBlock),
        /// A device pointer.
        Ptr(DevRange<'a>),
        /// `range.len / elem` as a `.u32`.
        CapOf(DevRange<'a>, u64),
        /// A `.u32` scalar.
        U32(u32),
        /// A `.s32` scalar.
        I32(i32),
    }

    /// What V3 checks about an argument: its kind and, for a pointer or a capacity, its range.
    /// ⊘ Its `Debug` (used in V3's refusals) names lengths and contexts, never the start.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(in crate::driver_unsafe) enum ArgDesc {
        /// An [`ArgBlock`] of context `ctx`.
        Block {
            /// Its context.
            ctx: u64,
        },
        /// A range.
        Ptr {
            /// Its start (compared, never exposed).
            addr: u64,
            /// Its bytes.
            len: u64,
            /// Its context.
            ctx: u64,
        },
        /// A capacity of a range.
        Cap {
            /// The range's start.
            addr: u64,
            /// The range's bytes.
            len: u64,
            /// Its context.
            ctx: u64,
            /// Bytes per element.
            elem: u64,
        },
        /// A `.u32` scalar.
        U32,
        /// A `.s32` scalar.
        I32,
    }

    impl core::fmt::Debug for ArgDesc {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                ArgDesc::Block { ctx } => write!(f, "Block(ctx {ctx})"),
                ArgDesc::Ptr { len, ctx, .. } => write!(f, "Ptr({len:#x} bytes, ctx {ctx})"),
                ArgDesc::Cap { len, ctx, elem, .. } => {
                    write!(f, "Cap(of {len:#x} bytes / {elem}, ctx {ctx})")
                }
                ArgDesc::U32 => write!(f, "U32"),
                ArgDesc::I32 => write!(f, "I32"),
            }
        }
    }

    impl Arg<'_> {
        fn desc(&self) -> ArgDesc {
            match self {
                Arg::Block(b) => ArgDesc::Block { ctx: b.ctx_id },
                Arg::Ptr(r) => ArgDesc::Ptr {
                    addr: r.addr,
                    len: r.len,
                    ctx: r.ctx_id,
                },
                Arg::CapOf(r, elem) => ArgDesc::Cap {
                    addr: r.addr,
                    len: r.len,
                    ctx: r.ctx_id,
                    elem: *elem,
                },
                Arg::U32(_) => ArgDesc::U32,
                Arg::I32(_) => ArgDesc::I32,
            }
        }

        fn bytes(&self) -> Vec<u8> {
            match self {
                Arg::Block(b) => b.bytes.to_vec(),
                Arg::Ptr(r) => r.addr.to_le_bytes().to_vec(),
                // `check_args` refused any quotient that does not fit a `u32` before any bytes are made.
                Arg::CapOf(r, elem) => u32::try_from(r.len / (*elem).max(1))
                    .unwrap_or(0)
                    .to_le_bytes()
                    .to_vec(),
                Arg::U32(v) => v.to_le_bytes().to_vec(),
                Arg::I32(v) => v.to_le_bytes().to_vec(),
            }
        }
    }

    /// ★★★ **V3, pure** — a launch of a kernel of context `ctx` with signature `sig`:
    /// the argument count and every kind match; every range, block and the stream belong to the
    /// kernel's context; every pointer reaches the bytes the kernel can address; every capacity is
    /// a capacity OF the range it bounds (same start, same length) and fits a `u32`;
    /// `grid ≥ 1` and `1 ≤ block ≤ 1024` (the block limit waived only for the failed-launch probe).
    pub(in crate::driver_unsafe) fn check_args(
        ctx: u64,
        stream_ctx: u64,
        sig: &[ArgKind],
        args: &[ArgDesc],
        grid: u32,
        block: u32,
        waive_block_limit: bool,
    ) -> Result<(), String> {
        if stream_ctx != ctx {
            return Err("the stream belongs to another CUDA context".to_string());
        }
        if args.len() != sig.len() {
            return Err(format!(
                "{} arguments for a {}-parameter kernel",
                args.len(),
                sig.len()
            ));
        }
        if grid == 0 || block == 0 || (block > MAX_BLOCK && !waive_block_limit) {
            return Err(format!("grid {grid} x block {block}"));
        }
        for (i, (k, a)) in sig.iter().zip(args).enumerate() {
            let ok = match (*k, *a) {
                (ArgKind::Block, ArgDesc::Block { ctx: c }) => c == ctx,
                (ArgKind::Ptr { min }, ArgDesc::Ptr { len, ctx: c, .. }) => c == ctx && len >= min,
                (
                    ArgKind::Cap { of, elem },
                    ArgDesc::Cap {
                        addr,
                        len,
                        ctx: c,
                        elem: e,
                    },
                ) => {
                    let paired = matches!(
                        args.get(of),
                        Some(ArgDesc::Ptr { addr: pa, len: pl, ctx: pc }) if *pa == addr && *pl == len && *pc == c
                    );
                    c == ctx && e == elem && paired && u32::try_from(len / elem.max(1)).is_ok()
                }
                (ArgKind::U32, ArgDesc::U32) | (ArgKind::I32, ArgDesc::I32) => true,
                _ => false,
            };
            if !ok {
                return Err(format!("argument {i}: {a:?} does not satisfy {k:?}"));
            }
        }
        Ok(())
    }

    /// A kernel entry (held with its context; reclaimed by `cuCtxDestroy`). No `Debug`: its
    /// handle is a pointer into libcuda's heap.
    pub(in crate::driver_unsafe) struct Kernel {
        ctx: Ctx,
        raw: usize,
        name: &'static str,
        sig: &'static [ArgKind],
    }

    /// `CUDA_KERNEL_NODE_PARAMS_v2` (`cuda.h`, CUDA 12); v1 is its first seven fields.
    #[repr(C)]
    struct KernelNodeParams {
        func: *mut c_void,
        grid_x: c_uint,
        grid_y: c_uint,
        grid_z: c_uint,
        block_x: c_uint,
        block_y: c_uint,
        block_z: c_uint,
        shmem: c_uint,
        kernel_params: *mut *mut c_void,
        extra: *mut *mut c_void,
        kern: *mut c_void,
        ctx: *mut c_void,
    }

    /// ★ A launch recorded during a stream capture: its graph node, its encoded arguments, and
    /// what keeps every range it names alive while the graph exists. Perimeter-internal.
    pub(in crate::driver_unsafe) struct Captured {
        node: usize,
        kernel: usize,
        name: &'static str,
        sig: &'static [ArgKind],
        grid: u32,
        block: u32,
        shm: u32,
        params: Vec<Vec<u8>>,
        _keep: Vec<Keep>,
    }

    /// A launch's encoded parameters, what keeps its ranges alive, and the driver's answer.
    type Launched = (Vec<Vec<u8>>, Vec<Keep>, CUresult);

    impl Kernel {
        fn launch_checked(
            &self,
            stream: &Stream,
            grid: u32,
            block: u32,
            shm: u32,
            args: &[Arg<'_>],
            waive: bool,
        ) -> Result<Launched, CudaError> {
            let descs: Vec<ArgDesc> = args.iter().map(Arg::desc).collect();
            check_args(
                self.ctx.id(),
                stream.ctx.id(),
                self.sig,
                &descs,
                grid,
                block,
                waive,
            )
            .map_err(|e| refused("cuLaunchKernel (V3)", format!("{}: {e}", self.name)))?;
            let capturing = stream.is_capturing();
            let mut keep = Vec::new();
            for a in args {
                match a {
                    // ★ V4: a launch that reads the pinned stage puts it in flight BEFORE it is
                    // queued (not under capture: a recorded launch runs only when the graph does).
                    Arg::Block(b) => {
                        if !capturing {
                            b.stage.begin()?;
                        }
                        keep.push(Keep::Ctx(b.ctx.share()));
                        keep.extend(b.keep.iter().map(Keep::clone_of));
                    }
                    Arg::Ptr(r) | Arg::CapOf(r, _) => keep.push(r.keep.keep()),
                    Arg::U32(_) | Arg::I32(_) => {}
                }
            }
            let mut params: Vec<Vec<u8>> = args.iter().map(Arg::bytes).collect();
            let mut p: Vec<*mut c_void> = params
                .iter_mut()
                .map(|b| b.as_mut_ptr().cast::<c_void>())
                .collect();
            // SAFETY: `raw` is a live function of this context (V3 checked the stream's context);
            // every element of `p` points into a live `Vec` of `params`, one per by-value
            // parameter — V3 checked the count and each kind against the PTX-pinned signature, so
            // the driver reads exactly the bytes each `Vec` holds (8 for a pointer, 4 for a
            // scalar, 264 for the block) and copies them during the call. Every pointer argument
            // is a `DevRange` reaching the bytes the kernel's own bounds can address (V3's `min`),
            // and every capacity is one of its own range.
            let r = unsafe {
                (self.ctx.cu().cuLaunchKernel)(
                    self.raw as *mut c_void,
                    grid,
                    1,
                    1,
                    block,
                    1,
                    1,
                    shm,
                    stream.raw as *mut c_void,
                    p.as_mut_ptr(),
                    core::ptr::null_mut(),
                )
            };
            Ok((params, keep, r))
        }

        /// ★★ **V3 — launch on `stream`.** Refused by name unless every argument satisfies the
        /// kernel's pinned signature. Under a stream capture, returns the recorded node.
        pub(in crate::driver_unsafe) fn launch(
            &self,
            stream: &Stream,
            grid: u32,
            block: u32,
            shm: u32,
            args: &[Arg<'_>],
        ) -> Result<Option<Captured>, CudaError> {
            let (params, keep, r) = self.launch_checked(stream, grid, block, shm, args, false)?;
            self.ctx
                .cu()
                .check("cuLaunchKernel", r)
                .map_err(|e| match e {
                    CudaError::Refused { code, name, .. } => CudaError::Refused {
                        what: "cuLaunchKernel",
                        code,
                        name: format!("{}: {name}", self.name),
                    },
                    other => other,
                })?;
            if !stream.is_capturing() {
                return Ok(None);
            }
            Ok(Some(Captured {
                node: stream.capture_leaf()?,
                kernel: self.raw,
                name: self.name,
                sig: self.sig,
                grid,
                block,
                shm,
                params,
                _keep: keep,
            }))
        }

        /// ★★ **A launch that must FAIL** (THE_CONSTRAINTS §w724d probe (b)): one block of 2048
        /// threads, above the architectural maximum on every part, so the DRIVER refuses at launch.
        /// The block limit is the only check waived; everything else is V3. `Err` = refused as
        /// intended; `Ok(())` = it unexpectedly launched (a finding).
        pub(in crate::driver_unsafe) fn probe_oversized_block(
            &self,
            stream: &Stream,
            args: &[Arg<'_>],
        ) -> Result<(), CudaError> {
            let (_params, _keep, r) = self.launch_checked(stream, 1, 2048, 0, args, true)?;
            self.ctx
                .cu()
                .check("cuLaunchKernel (deliberately malformed)", r)
        }
    }

    impl Keep {
        fn clone_of(k: &Keep) -> Keep {
            match k {
                Keep::Mem(a) => Keep::Mem(Arc::clone(a)),
                Keep::Ctx(c) => Keep::Ctx(c.share()),
            }
        }
    }

    /// ★ An instantiated graph of a captured walk. Keeps every allocation its nodes name and the
    /// block its by-value arguments currently carry, so nothing the graph can replay is freed.
    pub(in crate::driver_unsafe) struct GraphExec {
        ctx: Ctx,
        graph: usize,
        exec: usize,
        nodes: Vec<Captured>,
        baked: Option<ArgBlock>,
        updates: u64,
    }

    impl GraphExec {
        /// `cuGraphUpload` — pay the first launch's upload now, at bring-up.
        pub(in crate::driver_unsafe) fn upload(&self, stream: &Stream) -> Result<(), CudaError> {
            let f = need(self.ctx.cu().cuGraphUpload, "cuGraphUpload")?;
            if stream.ctx.id() != self.ctx.id() {
                return Err(refused("cuGraphUpload", "another context's stream".into()));
            }
            // SAFETY: both handles came from this binding and are live.
            self.ctx.cu().check("cuGraphUpload", unsafe {
                f(self.exec as *mut c_void, stream.raw as *mut c_void)
            })
        }

        /// How many launches had to rewrite the nodes' arguments.
        pub(in crate::driver_unsafe) fn updates(&self) -> u64 {
            self.updates
        }

        /// ★★ **Replay the walk with `block` as every node's by-value `KfArgs`** (and the named
        /// kernels' grids from `regrid`) — ONE `cuGraphLaunch`, plus one setter per node only when
        /// the block or a grid changed. V3 again for every rewritten node (the block's context; a
        /// grid ≥ 1). Puts the block's pinned stage in flight before the launch is queued.
        pub(in crate::driver_unsafe) fn launch(
            &mut self,
            stream: &Stream,
            block: &ArgBlock,
            regrid: &[(&'static str, u32)],
        ) -> Result<(), CudaError> {
            let launch = need(self.ctx.cu().cuGraphLaunch, "cuGraphLaunch")?;
            let set = need(
                self.ctx.cu().cuGraphExecKernelNodeSetParams,
                "cuGraphExecKernelNodeSetParams",
            )?;
            if stream.ctx.id() != self.ctx.id() || block.ctx_id != self.ctx.id() {
                return Err(refused(
                    "cuGraphLaunch (V3)",
                    "the stream or the block belongs to another CUDA context".to_string(),
                ));
            }
            let grid_of = |n: &Captured| {
                regrid
                    .iter()
                    .find(|(k, _)| *k == n.name)
                    .map_or(n.grid, |(_, g)| *g)
            };
            if regrid.iter().any(|(_, g)| *g == 0) {
                return Err(refused("cuGraphLaunch (V3)", "a zero grid".to_string()));
            }
            let stale = self.baked.as_ref().is_none_or(|b| b.bytes != block.bytes)
                || self.nodes.iter().any(|n| grid_of(n) != n.grid);
            if stale {
                // Unknown until every node took the new values: a failure part-way forces the next
                // launch to rewrite them all.
                self.baked = None;
                for i in 0..self.nodes.len() {
                    let grid = grid_of(&self.nodes[i]);
                    let n = &mut self.nodes[i];
                    if n.sig.first() == Some(&ArgKind::Block) {
                        n.params[0] = block.bytes.to_vec();
                    }
                    n.grid = grid;
                    let mut p: Vec<*mut c_void> = n
                        .params
                        .iter_mut()
                        .map(|b| b.as_mut_ptr().cast::<c_void>())
                        .collect();
                    let kp = KernelNodeParams {
                        func: n.kernel as *mut c_void,
                        grid_x: n.grid,
                        grid_y: 1,
                        grid_z: 1,
                        block_x: n.block,
                        block_y: 1,
                        block_z: 1,
                        shmem: n.shm,
                        kernel_params: p.as_mut_ptr(),
                        extra: core::ptr::null_mut(),
                        kern: core::ptr::null_mut(),
                        ctx: core::ptr::null_mut(),
                    };
                    // SAFETY: `exec`/`node` came from this binding and `node` belongs to the graph
                    // `exec` was instantiated from. Every element of `p` points into a live `Vec`
                    // of `n.params`, one per parameter of the node's kernel — validated by V3 when
                    // it was captured; the only bytes replaced are parameter 0 by a block of this
                    // context (checked above), the same kind and size. `kp` is fully initialised;
                    // the driver copies everything during the call.
                    let r = unsafe {
                        set(
                            self.exec as *mut c_void,
                            n.node as *mut c_void,
                            &raw const kp,
                        )
                    };
                    self.ctx.cu().check("cuGraphExecKernelNodeSetParams", r)?;
                }
                self.baked = Some(block.share());
                self.updates += 1;
            }
            block.stage.begin()?;
            // SAFETY: both handles came from this binding and are live; every allocation the graph
            // names is kept alive by `self.nodes` and `self.baked`.
            self.ctx.cu().check("cuGraphLaunch", unsafe {
                launch(self.exec as *mut c_void, stream.raw as *mut c_void)
            })
        }
    }

    impl Drop for GraphExec {
        fn drop(&mut self) {
            let drained = self.ctx.drain();
            if drop_disposition(&drained) == Disposition::Leak {
                // F1: a replay may still be queued; the graph and everything it names stay.
                core::mem::forget(core::mem::take(&mut self.nodes));
                core::mem::forget(self.baked.take());
                LEAKS.fetch_add(1, Ordering::Relaxed);
                return;
            }
            if let (Some(f), true) = (self.ctx.cu().cuGraphExecDestroy, self.exec != 0) {
                // SAFETY: `exec` came from `cuGraphInstantiateWithFlags` and is destroyed once.
                unsafe { f(self.exec as *mut c_void) };
            }
            if let (Some(f), true) = (self.ctx.cu().cuGraphDestroy, self.graph != 0) {
                // SAFETY: `graph` came from the capture and is destroyed once, after its exec.
                unsafe { f(self.graph as *mut c_void) };
            }
        }
    }

    // ═══ The pinned stage and V4 ════════════════════════════════════════════════════════════

    /// ★ A region of the walker's pinned stage.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in crate::driver_unsafe) enum Region {
        /// The root list (`KF_MAX_PDB` × u64), host-written.
        Pdbs,
        /// The slot list (`KF_MAX_PDB` × u32), host-written.
        Slots,
        /// The verdict (`KfAck`), host-written.
        Ack,
        /// One verdict byte per previous report run, host-written.
        AckCode,
        /// The report header, written by the kernels only.
        Hdr,
        /// The report's `PdbEntry` array, written by the kernels only.
        Rpdb,
        /// The report's run array, written by the kernels only.
        Rrun,
        /// The capacity layout (`KfLayout`), host-written.
        Lay,
    }

    impl Region {
        /// Every region, in stage order.
        pub(in crate::driver_unsafe) const ALL: [Region; 8] = [
            Region::Pdbs,
            Region::Slots,
            Region::Ack,
            Region::AckCode,
            Region::Hdr,
            Region::Rpdb,
            Region::Rrun,
            Region::Lay,
        ];

        fn index(self) -> usize {
            self as usize
        }

        /// ★ V4: the host may write only the regions the kernels READ (and never one they write).
        pub(in crate::driver_unsafe) fn host_writable(self) -> bool {
            matches!(
                self,
                Region::Pdbs | Region::Slots | Region::Ack | Region::AckCode | Region::Lay
            )
        }
    }

    /// ★ V4, pure: the stage's offsets from its regions' lengths — each region at the next 64-byte
    /// boundary after the previous one, so they are monotonic, aligned and disjoint by
    /// construction; `None` on an empty region or an overflow.
    pub(in crate::driver_unsafe) fn stage_layout(
        lens: &[usize; 8],
    ) -> Option<([(usize, usize); 8], usize)> {
        let mut at = [(0usize, 0usize); 8];
        let mut end = 0usize;
        for (i, &len) in lens.iter().enumerate() {
            if len == 0 {
                return None;
            }
            let off = end.checked_next_multiple_of(64)?;
            end = off.checked_add(len)?;
            at[i] = (off, len);
        }
        Some((at, end))
    }

    /// ★ The walk's GPU-flight state (V4). (`InFlight` is the walker's own word for it.)
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[allow(clippy::enum_variant_names)]
    pub(in crate::driver_unsafe) enum Flight {
        /// Nothing queued reads or writes the stage: the host may.
        Idle,
        /// Work that reads or writes the stage may be running.
        InFlight,
        /// A completion query failed: nothing about the stage is known any more. Final.
        Poisoned(String),
    }

    /// What happens to the flight state.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(in crate::driver_unsafe) enum FlightEvent {
        /// The host wants to read or write the stage.
        Access,
        /// A launch that uses the stage is about to be queued.
        Begin,
        /// A proof of completion arrived.
        Drained,
        /// A completion query failed.
        Error(String),
    }

    impl Flight {
        /// ★★ V4, pure: the one transition function. Only [`FlightEvent::Drained`] returns to
        /// `Idle`; an error poisons, and poisoned is final.
        pub(in crate::driver_unsafe) fn step(&self, ev: FlightEvent) -> Result<Flight, String> {
            match (self, ev) {
                (Flight::Poisoned(w), FlightEvent::Error(_))
                | (Flight::Poisoned(w), FlightEvent::Drained) => Ok(Flight::Poisoned(w.clone())),
                (Flight::Poisoned(w), _) => Err(format!("the walker is poisoned: {w}")),
                (_, FlightEvent::Error(w)) => Ok(Flight::Poisoned(w)),
                (Flight::Idle, FlightEvent::Access) => Ok(Flight::Idle),
                (Flight::InFlight, FlightEvent::Access) => Err(
                    "a walk is in flight: the pinned stage belongs to the GPU until its \
                     completion is observed"
                        .to_string(),
                ),
                (Flight::Idle | Flight::InFlight, FlightEvent::Begin) => Ok(Flight::InFlight),
                (Flight::Idle | Flight::InFlight, FlightEvent::Drained) => Ok(Flight::Idle),
            }
        }
    }

    /// The stage's flight state, shared with every [`ArgBlock`] minted over it.
    struct StageShared {
        ctx_id: u64,
        flight: Mutex<Flight>,
    }

    impl StageShared {
        fn apply(&self, ev: FlightEvent) -> Result<(), CudaError> {
            let mut f = self
                .flight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let next = f
                .step(ev)
                .map_err(|e| refused("the pinned stage (V4)", e))?;
            *f = next;
            Ok(())
        }

        fn begin(&self) -> Result<(), CudaError> {
            self.apply(FlightEvent::Begin)
        }
    }

    /// ★★ **The walker's page-locked stage** — the root and slot lists, the verdict, the layout
    /// the kernels read in place, and the report they write straight into host memory. Its device
    /// address is reachable only from `encode_kf_args`; its host bytes only through V4.
    pub(in crate::driver_unsafe) struct PinnedStage {
        ctx: Ctx,
        host: usize,
        dev: DevAddr,
        at: [(usize, usize); 8],
        shared: Arc<StageShared>,
    }

    impl PinnedStage {
        /// `cuMemAllocHost` of the regions' layout, zero-filled (the commit node's first read of
        /// "the previous report" must find no magic), and its one device address.
        pub(in crate::driver_unsafe) fn new(
            ctx: &Ctx,
            lens: &[usize; 8],
        ) -> Result<PinnedStage, CudaError> {
            let what = "cuMemAllocHost_v2 (the walk's pinned stage)";
            let (at, total) =
                stage_layout(lens).ok_or_else(|| refused(what, format!("{lens:?}")))?;
            ctx.make_current()?;
            let mut p: *mut c_void = core::ptr::null_mut();
            // SAFETY: one live out-pointer; `total` is owned by the driver entirely.
            ctx.cu().check(what, unsafe {
                (ctx.cu().cuMemAllocHost)(&raw mut p, total)
            })?;
            let mut d: DevAddr = 0;
            // SAFETY: one live out-pointer; `p` is the base of the live `cuMemAllocHost` allocation
            // just made in this context, which is what the call requires.
            ctx.cu().check("cuMemHostGetDevicePointer_v2", unsafe {
                (ctx.cu().cuMemHostGetDevicePointer)(&raw mut d, p, 0)
            })?;
            let mut s = PinnedStage {
                ctx: ctx.share(),
                host: p as usize,
                dev: d,
                at,
                shared: Arc::new(StageShared {
                    ctx_id: ctx.id(),
                    flight: Mutex::new(Flight::Idle),
                }),
            };
            for r in Region::ALL {
                let zero = vec![0u8; s.at[r.index()].1];
                s.copy_in(r, 0, &zero)?;
            }
            Ok(s)
        }

        /// A region's length.
        pub(in crate::driver_unsafe) fn region_len(&self, r: Region) -> usize {
            self.at[r.index()].1
        }

        fn span(
            &self,
            r: Region,
            off: usize,
            n: usize,
            what: &'static str,
        ) -> Result<usize, CudaError> {
            let (base, len) = self.at[r.index()];
            let end = off.checked_add(n);
            if end.is_none_or(|e| e > len) {
                return Err(refused(
                    what,
                    format!("[{off:#x}, +{n:#x}) leaves the {len:#x}-byte {r:?} region"),
                ));
            }
            Ok(base + off)
        }

        fn copy_in(&mut self, r: Region, off: usize, bytes: &[u8]) -> Result<(), CudaError> {
            let at = self.span(r, off, bytes.len(), "PinnedStage::write (V4)")?;
            // SAFETY: `[at, at+len)` is inside region `r`, which `stage_layout` placed inside the
            // live `cuMemAllocHost` allocation (`span` above checked the bound with overflow first);
            // `bytes` is a distinct Rust slice. No queued work reads the range: the callers are
            // `new` (nothing queued yet) and `write`, which refuses unless the flight is `Idle`.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    (self.host + at) as *mut u8,
                    bytes.len(),
                );
            }
            Ok(())
        }

        /// ★ V4: write `bytes` at `off` of a HOST-WRITABLE region, only while `Idle`.
        pub(in crate::driver_unsafe) fn write(
            &mut self,
            r: Region,
            off: usize,
            bytes: &[u8],
        ) -> Result<(), CudaError> {
            if !r.host_writable() {
                return Err(refused(
                    "PinnedStage::write (V4)",
                    format!("the {r:?} region is written by the kernels only"),
                ));
            }
            self.shared.apply(FlightEvent::Access)?;
            self.copy_in(r, off, bytes)
        }

        /// ★ V4: `n` bytes at `off` of a region, only while `Idle`.
        pub(in crate::driver_unsafe) fn read(
            &self,
            r: Region,
            off: usize,
            n: usize,
        ) -> Result<Vec<u8>, CudaError> {
            self.shared.apply(FlightEvent::Access)?;
            let at = self.span(r, off, n, "PinnedStage::read (V4)")?;
            let mut out = vec![0u8; n];
            // SAFETY: `[at, at+n)` is inside the live pinned allocation (`span` checked it); `out`
            // is a fresh local of exactly `n` bytes, so the regions cannot overlap. The flight is
            // `Idle` (checked above): the DMA that wrote the range completed before the proof that
            // returned the stage to `Idle`.
            unsafe {
                core::ptr::copy_nonoverlapping((self.host + at) as *const u8, out.as_mut_ptr(), n)
            };
            Ok(out)
        }

        /// ★ V4: the only way back to `Idle` — a proof of completion from THIS context.
        pub(in crate::driver_unsafe) fn end_flight(&self, d: Drained) -> Result<(), CudaError> {
            if d.ctx_id != self.shared.ctx_id {
                return Err(refused(
                    "PinnedStage::end_flight (V4)",
                    "a completion proof from another CUDA context".to_string(),
                ));
            }
            self.shared.apply(FlightEvent::Drained)
        }

        /// ★ F2: a failed completion query. Final.
        pub(in crate::driver_unsafe) fn poison(&self, why: String) {
            let _ = self.shared.apply(FlightEvent::Error(why));
        }

        /// The flight state.
        pub(in crate::driver_unsafe) fn flight(&self) -> Flight {
            self.shared
                .flight
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }

        fn device_range(&self, r: Region) -> DevRange<'_> {
            let (off, len) = self.at[r.index()];
            DevRange {
                addr: self.dev + off as u64,
                len: len as u64,
                ctx_id: self.ctx.id(),
                keep: KeepRef::Ctx(&self.ctx),
            }
        }
    }

    // ═══ The launch argument block ══════════════════════════════════════════════════════════

    /// ★★★ **The kernels' by-value `KfArgs`, encoded here and nowhere else** (§R(b): launch
    /// arguments are perimeter material). Opaque: its bytes contain device addresses. Holds the
    /// context, the stage's flight state, and what keeps the window it names alive.
    pub(in crate::driver_unsafe) struct ArgBlock {
        bytes: [u8; KF_ARGS_BYTES],
        ctx_id: u64,
        ctx: Ctx,
        stage: Arc<StageShared>,
        keep: Vec<Keep>,
    }

    impl ArgBlock {
        fn share(&self) -> ArgBlock {
            ArgBlock {
                bytes: self.bytes,
                ctx_id: self.ctx_id,
                ctx: self.ctx.share(),
                stage: Arc::clone(&self.stage),
                keep: self.keep.iter().map(Keep::clone_of).collect(),
            }
        }
    }

    /// The walker's device buffers a `KfArgs` names (the stage's regions come from the stage).
    pub(in crate::driver_unsafe) struct KfArgsRanges<'a> {
        /// The GPGA window walked (`None`: the empty window, base 0 length 0).
        pub win: Option<DevRange<'a>>,
        /// `KfDev`.
        pub dev: DevRange<'a>,
        /// The walk table.
        pub walk: DevRange<'a>,
        /// The committed placements.
        pub com: DevRange<'a>,
        /// `KfSlot` per slot.
        pub slot: DevRange<'a>,
        /// The run stage the diff reuses as scratch.
        pub scratch: DevRange<'a>,
        /// The diff's integer scratch.
        pub iscratch: DevRange<'a>,
        /// The pinned stage.
        pub stage: &'a PinnedStage,
    }

    /// `struct KfWin` (`kf_walk.cu`): mirrored here, where its pointer field lives.
    #[repr(C)]
    struct KfWin {
        base: u64,
        len: u64,
        span: u64,
    }

    /// `struct KfArgs` (`kf_walk.cu`), by value — 264 bytes, pinned to the `.cu` by
    /// `tests/walk_abi_matches_the_cu.rs` through [`KF_ARGS_LAYOUT`] and to the PTX's own `.param`
    /// declaration. ⊘ Never instantiated: it exists for `size_of`/`offset_of!`, and the encoder
    /// writes each field at its offset into a zeroed buffer, so padding is zero by construction.
    #[repr(C)]
    #[allow(dead_code)]
    struct KfArgs {
        win: KfWin,
        fmt: KfFormat,
        dev: u64,
        walk: u64,
        com: u64,
        slot: u64,
        pdbs: u64,
        slots: u64,
        npdb: u32,
        key_perm: u32,
        ack: u64,
        ack_code: u64,
        scratch: u64,
        iscratch: u64,
        hdr: u64,
        rpdb: u64,
        rrun: u64,
        lay: u64,
    }

    const _: () = assert!(core::mem::size_of::<KfArgs>() == KF_ARGS_BYTES);

    /// A struct's layout as `(size, [(field, offset)])` — for the ABI differential only.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct StructLayout {
        /// `size_of`.
        pub size: usize,
        /// `offset_of!` per field, in declaration order.
        pub fields: &'static [(&'static str, usize)],
    }

    macro_rules! layout_of {
        ($t:ty { $($f:ident),+ $(,)? }) => {
            StructLayout {
                size: core::mem::size_of::<$t>(),
                fields: &[$((stringify!($f), core::mem::offset_of!($t, $f))),+],
            }
        };
    }

    /// `KfArgs`' layout (re-exported as `kf_cuda::abi::KF_ARGS_LAYOUT`).
    pub const KF_ARGS_LAYOUT: StructLayout = layout_of!(KfArgs {
        win,
        fmt,
        dev,
        walk,
        com,
        slot,
        pdbs,
        slots,
        npdb,
        key_perm,
        ack,
        ack_code,
        scratch,
        iscratch,
        hdr,
        rpdb,
        rrun,
        lay
    });
    /// `KfWin`'s layout (re-exported as `kf_cuda::abi::KF_WIN_LAYOUT`).
    pub const KF_WIN_LAYOUT: StructLayout = layout_of!(KfWin { base, len, span });

    /// ★★★ **Encode `KfArgs`** over ranges of ONE context — the only producer of the kernels'
    /// by-value parameter. Every pointer field is a [`DevRange`] or a stage region of that context;
    /// a range of another context is refused by name. `win.span == win.len` (the single store is
    /// the whole of guest vidmem and all of it is mapped).
    pub(in crate::driver_unsafe) fn encode_kf_args(
        r: &KfArgsRanges<'_>,
        fmt: &KfFormat,
        npdb: u32,
        key_perm: u32,
    ) -> Result<ArgBlock, CudaError> {
        let ctx = r.stage.ctx.id();
        let ranges = [r.dev, r.walk, r.com, r.slot, r.scratch, r.iscratch];
        if ranges.iter().chain(r.win.iter()).any(|x| x.ctx_id != ctx) {
            return Err(refused(
                "encode_kf_args",
                "a range of another CUDA context".to_string(),
            ));
        }
        let st = r.stage;
        let mut b = [0u8; KF_ARGS_BYTES];
        let mut put = |off: usize, v: &[u8]| b[off..off + v.len()].copy_from_slice(v);
        let o = |f: &str| {
            KF_ARGS_LAYOUT
                .fields
                .iter()
                .find(|(n, _)| *n == f)
                .map_or(0, |(_, o)| *o)
        };
        let w = |f: &str| {
            KF_WIN_LAYOUT
                .fields
                .iter()
                .find(|(n, _)| *n == f)
                .map_or(0, |(_, o)| *o)
        };
        let (wb, wl) = r.win.map_or((0, 0), |x| (x.addr, x.len));
        put(o("win") + w("base"), &wb.to_le_bytes());
        put(o("win") + w("len"), &wl.to_le_bytes());
        put(o("win") + w("span"), &wl.to_le_bytes());
        put(o("fmt"), &fmt.encode());
        put(o("dev"), &r.dev.addr.to_le_bytes());
        put(o("walk"), &r.walk.addr.to_le_bytes());
        put(o("com"), &r.com.addr.to_le_bytes());
        put(o("slot"), &r.slot.addr.to_le_bytes());
        put(o("pdbs"), &st.device_range(Region::Pdbs).addr.to_le_bytes());
        put(
            o("slots"),
            &st.device_range(Region::Slots).addr.to_le_bytes(),
        );
        put(o("npdb"), &npdb.to_le_bytes());
        put(o("key_perm"), &key_perm.to_le_bytes());
        put(o("ack"), &st.device_range(Region::Ack).addr.to_le_bytes());
        put(
            o("ack_code"),
            &st.device_range(Region::AckCode).addr.to_le_bytes(),
        );
        put(o("scratch"), &r.scratch.addr.to_le_bytes());
        put(o("iscratch"), &r.iscratch.addr.to_le_bytes());
        put(o("hdr"), &st.device_range(Region::Hdr).addr.to_le_bytes());
        put(o("rpdb"), &st.device_range(Region::Rpdb).addr.to_le_bytes());
        put(o("rrun"), &st.device_range(Region::Rrun).addr.to_le_bytes());
        put(o("lay"), &st.device_range(Region::Lay).addr.to_le_bytes());
        let keep = ranges
            .iter()
            .chain(r.win.iter())
            .map(|x| x.keep.keep())
            .collect();
        Ok(ArgBlock {
            bytes: b,
            ctx_id: ctx,
            ctx: st.ctx.share(),
            stage: Arc::clone(&st.shared),
            keep,
        })
    }

    // ═══ The console frame's pages ══════════════════════════════════════════════════════════

    /// ★ A console frame's page-locked destination: a [`StaticSpan`] registered with context
    /// `ctx_id`. Minted only by [`console_pages`].
    pub(in crate::driver_unsafe) struct ConsoleDst {
        span: StaticSpan,
        ctx_id: u64,
    }

    impl ConsoleDst {
        /// The frame's span (opaque).
        pub(in crate::driver_unsafe) fn span(&self) -> StaticSpan {
            self.span
        }

        /// The context the pages are registered with.
        pub(in crate::driver_unsafe) fn ctx_id(&self) -> u64 {
            self.ctx_id
        }
    }

    /// ★★★ **Map, page-lock, and only then leak, `bytes` of console frame** (design D6/D7, V12's
    /// mechanism). The mapping is made HERE, private anonymous and owned, so its
    /// [`MappedRegion::static_span`] cannot be refused; it is registered with the driver while
    /// still owned (on a refusal it drops — unmapped, nothing registered); and it is leaked only
    /// after the registration succeeded, so a registered page is never unmapped. `bytes` must be
    /// a whole number of host pages (the caller rounds).
    pub(in crate::driver_unsafe) fn console_pages(
        ctx: &Ctx,
        bytes: u64,
    ) -> Result<(&'static MappedRegion, ConsoleDst), CudaError> {
        let what = "cuMemHostRegister_v2 (console frame)";
        let register = need(ctx.cu().cuMemHostRegister, "cuMemHostRegister_v2")?;
        let page = HostPageSize::query();
        let region = MappedRegion::map(
            Backing::PrivateAnonymous,
            bytes,
            HostProt::ReadWrite,
            CachePolicy::WriteBack,
            page,
        )
        .map_err(|e| refused(what, format!("the frame mapping: {e}")))?;
        let span = region.host_span();
        ctx.make_current()?;
        // SAFETY: `span` is the whole of `region`, a live private-anonymous mapping this function
        // owns; the driver pins exactly `[ptr, ptr+len)` of it, as `HostSpan::as_ptr`'s contract
        // allows. On success the region is leaked below before this function returns, so the
        // registered pages are never unmapped; on failure nothing was registered and the region
        // is dropped (unmapped) by the `?`.
        ctx.cu().check(what, unsafe {
            register(span.as_ptr().cast::<c_void>(), span.len(), 0)
        })?;
        let pages: &'static MappedRegion = Box::leak(Box::new(region));
        let span = pages
            .static_span()
            .expect("console_pages maps owned private anonymous memory, which is always eligible");
        Ok((
            pages,
            ConsoleDst {
                span,
                ctx_id: ctx.id(),
            },
        ))
    }

    // ═══ The completion fd ══════════════════════════════════════════════════════════════════

    /// ★★★ **A walk's (or a scanout copy's) completion fd** — an
    /// `eventfd(EFD_NONBLOCK|EFD_CLOEXEC)` a CUDA host function writes once everything queued
    /// before it has completed. Put it in the worker's `epoll` set (`AsFd`); [`CompletionFd::drain`]
    /// consumes the wake.
    #[derive(Debug)]
    pub struct CompletionFd {
        fd: Arc<OwnedFd>,
    }

    impl CompletionFd {
        /// A fresh non-blocking eventfd.
        ///
        /// # Errors
        /// [`CudaError::Refused`] naming `eventfd`.
        pub fn new() -> Result<CompletionFd, CudaError> {
            // SAFETY: `eventfd` takes two integers and returns an fd or -1.
            let fd = unsafe { eventfd(0, EFD_NONBLOCK_CLOEXEC) };
            if fd < 0 {
                return Err(CudaError::Refused {
                    what: "eventfd (the completion fd)",
                    code: fd,
                    name: "eventfd(2) refused".to_string(),
                });
            }
            // SAFETY: `fd` was just returned by `eventfd`, is open, and is owned by nothing else.
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            Ok(CompletionFd { fd: Arc::new(fd) })
        }

        /// Signal it once, as a queued host function does (a harness multiplexing other work).
        pub fn signal(&self) {
            completion_hostfn(
                usize::try_from(self.fd.as_raw_fd()).unwrap_or(usize::MAX) as *mut c_void
            );
        }

        /// Consume every pending signal; `0` when none was pending. **Never blocks.**
        #[must_use]
        pub fn drain(&self) -> u64 {
            let mut v: u64 = 0;
            // SAFETY: `v` is a live 8-byte buffer; the fd is non-blocking and open (owned here).
            let n = unsafe { read(self.fd.as_raw_fd(), (&raw mut v).cast::<c_void>(), 8) };
            if n == 8 { v } else { 0 }
        }

        /// `poll(2)` for readability, up to `timeout_ms`. ⚠ **Blocks the calling thread** —
        /// harnesses and the self-test only, never a worker (§35).
        #[must_use]
        pub fn wait_readable(&self, timeout_ms: i32) -> bool {
            let mut p = PollFd {
                fd: self.fd.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            };
            // SAFETY: one live `pollfd`, count 1.
            let r = unsafe { poll(&raw mut p, 1, timeout_ms) };
            r > 0 && (p.revents & POLLIN) != 0
        }
    }

    impl AsFd for CompletionFd {
        fn as_fd(&self) -> BorrowedFd<'_> {
            self.fd.as_fd()
        }
    }

    /// ★ The host function every walk and scanout copy ends with: `write(fd, 1)`, `user` being
    /// the fd NUMBER. It dereferences nothing it is handed and makes no CUDA call.
    extern "C" fn completion_hostfn(user: *mut c_void) {
        let fd = c_int::try_from(user as usize).unwrap_or(-1);
        let one: u64 = 1;
        // SAFETY: `one` is a live 8-byte value; `write` on a bad fd returns -1 and touches nothing.
        let _ = unsafe { write(fd, (&raw const one).cast::<c_void>(), 8) };
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// ★ The completion path WITHOUT a GPU: the exact function the driver calls makes the fd
        /// readable and drains to exactly one signal.
        #[test]
        fn the_host_function_signals_the_fd_and_drain_consumes_it() {
            let fd = CompletionFd::new().expect("eventfd");
            assert_eq!(fd.drain(), 0, "a fresh fd has nothing pending");
            assert!(!fd.wait_readable(0), "and is not readable");
            completion_hostfn(usize::try_from(fd.fd.as_raw_fd()).unwrap() as *mut c_void);
            assert!(
                fd.wait_readable(0),
                "the host function must make the fd readable"
            );
            assert_eq!(fd.drain(), 1, "exactly one signal per walk");
            assert_eq!(fd.drain(), 0, "drain consumed it");
        }

        /// ★ T18 (raw tier): the encoder's layout is the compiler's, and the block is the PTX's
        /// size — the only test that looks at the addresses' container.
        #[test]
        fn the_kfargs_layout_is_the_compilers() {
            assert_eq!(KF_ARGS_LAYOUT.size, KF_ARGS_BYTES);
            assert_eq!(KF_WIN_LAYOUT.size, 24);
            let names: Vec<&str> = KF_ARGS_LAYOUT.fields.iter().map(|(n, _)| *n).collect();
            assert_eq!(names.len(), 18);
            assert!(KF_ARGS_LAYOUT.fields.windows(2).all(|w| w[0].1 < w[1].1));
        }
    }
}
