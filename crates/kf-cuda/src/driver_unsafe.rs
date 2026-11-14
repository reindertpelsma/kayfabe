//! ★★★★★ **THE CUDA DRIVER API, `dlopen`ed** — no build-time link, no `libcudart`, no CUDA
//! toolkit anywhere in this tree's build.
//!
//! # ⊘⊘⊘ THE PREMISE THIS WHOLE MODULE EXISTS FOR, AND IT IS MEASURED
//!
//! `[measured 2026-09-14, locally, no GPU]` a **musl static-pie** Rust binary's `dlopen`
//! returns `NULL` with `dlerror()` = **`"Dynamic loading not supported"`** — for
//! `libcuda.so.1`, for `libc.so.6` and for `libm.so.6` alike. ⇒ the refusal is **musl's**,
//! not `libcuda`'s, and it arrives **before any question about CUDA is asked**. Every other
//! isolate in this tree is exactly that binary (`kayfabe-isolate-host/build.rs` refuses a
//! non-static image by name), so **no other isolate could ever load CUDA**, whatever it did
//! about privilege ordering.
//!
//! ⊘ `THE_CONSTRAINTS.md` §w724d states the blocker as *"`libcuda` is a glibc shared object,
//! and musl static binaries do not support `dlopen` at all"*. Both halves are true and the
//! second is the load-bearing one: it is not *"the wrong kind of shared object"*, it is
//! *"there is no dynamic linker in this process"*. ⇒ the fix has to be a **different build**,
//! which is what §w724d prescribes.
//!
//! # Why the driver API and not the runtime API
//!
//! `libcudart` is a second shared object, it is not present on a machine that has only the
//! driver installed, and it owns a context lifecycle we do not want. The driver API is what
//! `libcuda.so.1` — the file the **driver package** installs, beside `nvidia-smi` — exports.
//! ⇒ a bench box provisioned with nothing but the NVIDIA driver can run this. That is the
//! same reason the PTX is JITted rather than shipped as a cubin.
//!
//! ⚠ **Every symbol is resolved by name and a missing one is a named refusal**, never a null
//! call. A partially-resolved binding that runs until it reaches the symbol nobody checked is
//! the shape this tree keeps paying for.

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::sync::atomic::{AtomicBool, Ordering};
use std::cell::RefCell;
use std::ffi::CString;
use std::sync::Arc;

// ════════════════════════════════════════════════════════════════════════════════════════════
// ★★★★★ 2026-11-13 (security review 2, D-01/D-02/D-04; owner rule "every safe-to-unsafe boundary
// validates"): THE PERIMETER CHECKS ITS OWN INPUTS. A safe caller in this crate is assumed to be
// wrong some day; what it can then cause is a PANIC (as the standard library does), never a
// driver call on a dead handle, an out-of-allocation device range, or a short parameter array.
//
// The three mechanisms, all in this file:
//  1. LIFETIME. Every handle this binding mints carries the liveness token (`Life`) of the context
//     it was created in (and a graph's own token for graph nodes). Handles are not `Copy`; every
//     destroy CONSUMES its handle; `ctx_destroy` ends the context's token, so any child handle
//     used afterwards panics before libcuda sees it (D-04).
//  2. DEVICE MEMORY. No safe function here takes a device address. Allocations are `DevBuf`s
//     (address + length, minted only by `mem_alloc_zeroed`, `import_and_map` and
//     `pinned_device_view`); copies and sets take a `DevSpan`, a borrow of a `DevBuf` whose range
//     was checked (`checked_add`) against the allocation when the span was made (D-02, S1-04).
//  3. LAUNCHES. `module_load` parses the `.entry` parameter lists of the PTX it is handed; a
//     `Func` carries its entry's parameter sizes; every launch asserts the count and every size
//     before `cuLaunchKernel` / `cuGraphExecKernelNodeSetParams` (D-01).
// ════════════════════════════════════════════════════════════════════════════════════════════

/// ★ A liveness token: `true` while the context (or graph) it names exists. Shared by every
/// handle minted in that context; ended exactly once, by the destroy that consumes the owner.
#[derive(Debug)]
pub(crate) struct Life(AtomicBool);

impl Life {
    fn new() -> Arc<Life> {
        Arc::new(Life(AtomicBool::new(true)))
    }
    fn dead() -> Arc<Life> {
        Arc::new(Life(AtomicBool::new(false)))
    }
    fn alive(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    fn end(&self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Panic unless `l` is alive. ⊘ A panic and not an `Err`: a dead handle in a safe caller is a
/// bug in this process, and the standard library's answer to a bug at a safe boundary is a panic.
#[track_caller]
fn live(l: &Life, what: &str) {
    assert!(
        l.alive(),
        "{what}: a CUDA handle was used after its context (or graph) was destroyed; refused \
         before the driver saw it"
    );
}

thread_local! {
    /// ★ This thread's CUDA context stack, mirrored: the driver's current context is PER THREAD
    /// (`ctx_set_current`'s doc), so the context a new handle belongs to is the top of the
    /// calling thread's stack. Only this binding changes the driver's stack (`ctx_create` pushes,
    /// `ctx_set_current` replaces the top, `ctx_destroy` pops the destroyed context when it is the
    /// top), so the mirror is exact.
    static CTX_STACK: RefCell<Vec<(usize, Arc<Life>)>> = const { RefCell::new(Vec::new()) };
}

/// The liveness token of the context current on this thread, which a handle created now joins.
///
/// # Errors
/// Refused by name when no context of this binding is current on the thread.
///
/// # Panics
/// When the current context was destroyed.
fn current_life(what: &'static str) -> Result<Arc<Life>, CudaError> {
    let top = CTX_STACK.with(|s| s.borrow().last().map(|(_, l)| l.clone()));
    let Some(l) = top else {
        return Err(CudaError::Refused {
            what,
            code: 0,
            name: "no context of this binding is current on this thread (make_current first)"
                .to_string(),
        });
    };
    live(&l, what);
    Ok(l)
}

/// A CUDA driver-API status. `0` is `CUDA_SUCCESS`.
pub type CUresult = c_int;
/// `CUDA_SUCCESS`.
pub const CUDA_SUCCESS: CUresult = 0;

/// A device-memory address, as the driver spells it.
pub type CUdeviceptr = u64;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
    // ★ P4 (w826, `V3_P4_PORT_MAP.md` §2.1(d)) — the walk's completion fd. libc, which every
    // glibc process already links; no new build dependency.
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
/// `EFD_NONBLOCK | EFD_CLOEXEC` (`<sys/eventfd.h>`; the same values on x86-64 and aarch64).
const EFD_NONBLOCK_CLOEXEC: c_int = 0o4000 | 0o2_000_000;
/// `CUDA_ERROR_NOT_READY` — `cuEventQuery`'s "not yet", which is an answer and not a failure.
pub const CUDA_ERROR_NOT_READY: CUresult = 600;

const RTLD_NOW: c_int = 2;
const RTLD_GLOBAL: c_int = 0x100;

/// Why CUDA could not be brought up, or what it refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CudaError {
    /// `libcuda.so.1` could not be loaded. ⊘ Carries `dlerror()` verbatim, because
    /// *"Dynamic loading not supported"* (a musl build) and *"cannot open shared object
    /// file"* (no driver installed) are **different diagnoses** and a collapsed message
    /// would send a reader to the wrong one.
    NoLibrary {
        /// What was tried.
        soname: String,
        /// `dlerror()`, verbatim.
        dlerror: String,
    },
    /// A symbol the binding requires is absent from the library that did load.
    MissingSymbol(&'static str),
    /// A driver call refused.
    Refused {
        /// Which call.
        what: &'static str,
        /// Its `CUresult`.
        code: CUresult,
        /// `cuGetErrorName`, if the library could give one.
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

/// The subset of the driver API the walk kernel needs, resolved once.
///
/// ⊘ It is a **struct of function pointers and not a set of `extern` declarations**, because
/// an `extern` block is a link-time dependency: it would make every build of this workspace
/// require `libcuda`, including the ones on machines with no NVIDIA driver at all. The whole
/// point is that the dependency is discovered at run time, in one process, and refused by
/// name everywhere else.
#[allow(non_snake_case)]
pub struct Cuda {
    _handle: *mut c_void,
    pub(crate) cuInit: unsafe extern "C" fn(c_uint) -> CUresult,
    pub(crate) cuDeviceGet: unsafe extern "C" fn(*mut c_int, c_int) -> CUresult,
    pub(crate) cuDeviceGetCount: unsafe extern "C" fn(*mut c_int) -> CUresult,
    pub(crate) cuDeviceGetName: unsafe extern "C" fn(*mut c_char, c_int, c_int) -> CUresult,
    /// ★ `cuDeviceGetByPCIBusId(CUdevice*, const char*)` — select the device by the host GPU's
    /// PCI address, never by ordinal (V3_MULTI_GPU_AUDIT §2 blocker 1: ordinals are
    /// fastest-first and `CUDA_VISIBLE_DEVICES` reorders them). Present since CUDA 4.1, so a
    /// missing symbol refuses the binding by name.
    pub(crate) cuDeviceGetByPCIBusId: unsafe extern "C" fn(*mut c_int, *const c_char) -> CUresult,
    /// ★ 2026-11-14 (security review 3 F-7 / S1-05): `cuDeviceGetAttribute(int*, attr, dev)`,
    /// for the one question the display and walk contexts must answer about the host: can a
    /// kernel reach PAGEABLE host memory (HMM / ATS)? `Option`: absent is an answer, not a failure.
    pub(crate) cuDeviceGetAttribute:
        Option<unsafe extern "C" fn(*mut c_int, c_int, c_int) -> CUresult>,
    pub(crate) cuCtxCreate: unsafe extern "C" fn(*mut *mut c_void, c_uint, c_int) -> CUresult,
    pub(crate) cuCtxDestroy: unsafe extern "C" fn(*mut c_void) -> CUresult,
    pub(crate) cuCtxSynchronize: unsafe extern "C" fn() -> CUresult,
    pub(crate) cuCtxSetCurrent: unsafe extern "C" fn(*mut c_void) -> CUresult,
    pub(crate) cuModuleLoadData: unsafe extern "C" fn(*mut *mut c_void, *const c_void) -> CUresult,
    pub(crate) cuModuleGetFunction:
        unsafe extern "C" fn(*mut *mut c_void, *mut c_void, *const c_char) -> CUresult,
    pub(crate) cuMemAlloc: unsafe extern "C" fn(*mut CUdeviceptr, usize) -> CUresult,
    pub(crate) cuMemFree: unsafe extern "C" fn(CUdeviceptr) -> CUresult,
    pub(crate) cuMemsetD8: unsafe extern "C" fn(CUdeviceptr, u8, usize) -> CUresult,
    pub(crate) cuMemcpyHtoD: unsafe extern "C" fn(CUdeviceptr, *const c_void, usize) -> CUresult,
    pub(crate) cuMemcpyDtoH: unsafe extern "C" fn(*mut c_void, CUdeviceptr, usize) -> CUresult,
    pub(crate) cuLaunchKernel: unsafe extern "C" fn(
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
    // ★★★★★ **P4 — THE ASYNCHRONOUS WALK** (`V3_P4_PORT_MAP.md` §2.1(d), Q6). A walk is a
    // stream of launches ending in a host function that writes an eventfd; the submitting
    // thread never waits on the GPU. Required, not optional: every driver since CUDA 10 exports
    // them, and a walker that cannot complete asynchronously is one v3 refuses (§5, §41).
    pub(crate) cuStreamCreate: unsafe extern "C" fn(*mut *mut c_void, c_uint) -> CUresult,
    pub(crate) cuStreamDestroy: unsafe extern "C" fn(*mut c_void) -> CUresult,
    pub(crate) cuEventCreate: unsafe extern "C" fn(*mut *mut c_void, c_uint) -> CUresult,
    pub(crate) cuEventDestroy: unsafe extern "C" fn(*mut c_void) -> CUresult,
    pub(crate) cuEventRecord: unsafe extern "C" fn(*mut c_void, *mut c_void) -> CUresult,
    pub(crate) cuEventQuery: unsafe extern "C" fn(*mut c_void) -> CUresult,
    pub(crate) cuEventElapsedTime:
        unsafe extern "C" fn(*mut f32, *mut c_void, *mut c_void) -> CUresult,
    pub(crate) cuLaunchHostFunc:
        unsafe extern "C" fn(*mut c_void, extern "C" fn(*mut c_void), *mut c_void) -> CUresult,
    pub(crate) cuMemAllocHost: unsafe extern "C" fn(*mut *mut c_void, usize) -> CUresult,
    pub(crate) cuMemFreeHost: unsafe extern "C" fn(*mut c_void) -> CUresult,
    pub(crate) cuMemHostGetDevicePointer:
        unsafe extern "C" fn(*mut CUdeviceptr, *mut c_void, c_uint) -> CUresult,
    pub(crate) cuMemcpyDtoHAsync:
        unsafe extern "C" fn(*mut c_void, CUdeviceptr, usize, *mut c_void) -> CUresult,
    pub(crate) cuMemcpyDtoDAsync:
        unsafe extern "C" fn(CUdeviceptr, CUdeviceptr, usize, *mut c_void) -> CUresult,
    pub(crate) cuMemsetD8Async:
        unsafe extern "C" fn(CUdeviceptr, u8, usize, *mut c_void) -> CUresult,
    // ★★★★★ **P4b (w827) — THE WALK AS ONE CUDA GRAPH.** `[measured GA106 cc3caf1f]` the
    // walk's ~25 stream operations cost `submit_us p50=172` (ver2) / `212` (ver3) on the
    // submitting thread against a 50 µs budget: the cost is per-launch DRIVER time, not GPU
    // time. Captured once and replayed with one `cuGraphLaunch`, the per-walk cost is one call.
    // ⊘ `Option`, like the VMM set below and for the same reason: this binding is loaded on
    // every boot, and a driver without graphs (pre-11.4) must degrade the walker to per-launch
    // submission — VISIBLY, via `WalkKernel::submits_as_graph` — not lose it.
    pub(crate) cuStreamBeginCapture: Option<unsafe extern "C" fn(*mut c_void, c_uint) -> CUresult>,
    pub(crate) cuStreamEndCapture:
        Option<unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> CUresult>,
    pub(crate) cuStreamGetCaptureInfo: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *mut c_uint,
            *mut u64,
            *mut *mut c_void,
            *mut *const *mut c_void,
            *mut usize,
        ) -> CUresult,
    >,
    pub(crate) cuGraphInstantiateWithFlags:
        Option<unsafe extern "C" fn(*mut *mut c_void, *mut c_void, u64) -> CUresult>,
    pub(crate) cuGraphLaunch: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> CUresult>,
    pub(crate) cuGraphExecKernelNodeSetParams:
        Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *const KernelNodeParams) -> CUresult>,
    pub(crate) cuGraphExecDestroy: Option<unsafe extern "C" fn(*mut c_void) -> CUresult>,
    pub(crate) cuGraphDestroy: Option<unsafe extern "C" fn(*mut c_void) -> CUresult>,
    pub(crate) cuGraphUpload: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> CUresult>,
    pub(crate) cuEventRecordWithFlags:
        Option<unsafe extern "C" fn(*mut c_void, *mut c_void, c_uint) -> CUresult>,
    /// ★ How many times `cuCtxSynchronize` ran through this binding — the falsifier of
    /// `V3_P4_PORT_MAP.md` §3 row 2 (*"fails if any `cuCtxSynchronize` runs on the worker
    /// (count the calls)"*). A counter, not a promise: gate 8 reads it around its walks.
    ctx_sync_calls: core::sync::atomic::AtomicU64,
    // ⊘ 2026-11-13: the w755i export-side entry points (`cuMemGetAllocationGranularity`,
    // `cuMemCreate`, `cuMemExportToShareableHandle`, `cuMemRelease`) are no longer resolved: their
    // one user, `export_device_allocation`, was removed (see there).
    // ★★★★★ **w755w — THE IMPORT SIDE, which is the direction that can work.**
    //
    // `[measured w755v]` RM refuses to import CUDA's fd (`nvfp->handles == NULL`,
    // `os.c:2377`) because RM registers `handles[0]` only in its **own** export. So the
    // store is exported BY RM and imported BY CUDA, and these are the symbols for that half.
    //
    // ⊘ `osHandle` is a `void *` that, for `CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR`, is the
    // **fd itself** cast to a pointer — not a pointer to the fd. Getting that backwards
    // yields `INVALID_VALUE` and reads like a rejected handle.
    pub(crate) cuMemImportFromShareableHandle:
        Option<unsafe extern "C" fn(*mut u64, *mut c_void, c_uint) -> CUresult>,
    pub(crate) cuMemAddressReserve:
        Option<unsafe extern "C" fn(*mut u64, usize, usize, u64, u64) -> CUresult>,
    pub(crate) cuMemMap: Option<unsafe extern "C" fn(u64, usize, usize, u64, u64) -> CUresult>,
    pub(crate) cuMemSetAccess:
        Option<unsafe extern "C" fn(u64, usize, *const c_void, usize) -> CUresult>,
    // ★ 2026-11-14 (security review 2 D-15): the three undo steps of an import that failed half
    // way — `import_and_map` gives back what it took instead of leaking it into the context.
    pub(crate) cuMemUnmap: Option<unsafe extern "C" fn(u64, usize) -> CUresult>,
    pub(crate) cuMemAddressFree: Option<unsafe extern "C" fn(u64, usize) -> CUresult>,
    pub(crate) cuMemRelease: Option<unsafe extern "C" fn(u64) -> CUresult>,
    // ★ 2026-10-03 (display step 3, the broker relay): page-lock memory WE mapped (a sealed
    // memfd the broker also receives) so the display's async D2H copy can land in it.
    // ⊘ `Option`, like the VMM set: a driver without them loses the broker path BY NAME at its
    // first frame, never the walker at load.
    pub(crate) cuMemHostRegister:
        Option<unsafe extern "C" fn(*mut c_void, usize, c_uint) -> CUresult>,
    pub(crate) cuMemHostUnregister: Option<unsafe extern "C" fn(*mut c_void) -> CUresult>,
}

// SAFETY: every field is a code pointer into a library loaded `RTLD_GLOBAL` for the life of
// the process. Nothing here is interior-mutable and nothing is freed.
unsafe impl Send for Cuda {}
// SAFETY: as above — the struct is immutable after construction.
unsafe impl Sync for Cuda {}

/// The soname the **driver package** installs. ⊘ Not `libcuda.so`, which only a *toolkit*
/// (or a `-dev` package) provides: the whole point is that a box carrying nothing but the
/// NVIDIA driver can run this.
pub const LIBCUDA_SONAME: &str = "libcuda.so.1";

/// `dlsym`, returning NULL rather than refusing. ⊘ One place, so the NULL-tolerant and the
/// required lookups share exactly one `unsafe` between them.
fn sym_or_null(handle: *mut c_void, name: &str) -> *mut c_void {
    let Ok(n) = CString::new(name) else {
        return core::ptr::null_mut();
    };
    // SAFETY: `handle` is a live library handle and `n` is NUL-terminated.
    unsafe { dlsym(handle, n.as_ptr()) }
}

impl Cuda {
    /// `dlopen` the driver and resolve every symbol.
    ///
    /// # Errors
    /// [`CudaError::NoLibrary`] if the library will not load — carrying `dlerror()` verbatim,
    /// because *"Dynamic loading not supported"* and *"cannot open shared object file"* are
    /// different diagnoses. [`CudaError::MissingSymbol`] naming the first absent symbol.
    pub fn open() -> Result<Cuda, CudaError> {
        Self::open_soname(LIBCUDA_SONAME)
    }

    /// As [`Cuda::open`], for a caller that must name the library (a test, or a box that puts
    /// it somewhere unusual).
    ///
    /// # Errors
    /// As [`Cuda::open`].
    pub fn open_soname(soname: &str) -> Result<Cuda, CudaError> {
        // ★★★ Before `dlopen`: this thread, and every thread libcuda starts from it, makes its
        // CUDA calls without CAP_SYS_ADMIN in effect, so libcuda's channels are born USER
        // (`crate::posture`).
        crate::posture::cuda_thread()?;
        let c = CString::new(soname).map_err(|_| CudaError::NoLibrary {
            soname: soname.to_string(),
            dlerror: "the soname contains a NUL".to_string(),
        })?;
        // ⊘ `RTLD_GLOBAL` because the driver's own lazy paths resolve against the global
        // scope; `RTLD_NOW` because a lazy binding would move a missing symbol from here —
        // where it is a named refusal — to an arbitrary later call.
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

        // ⚠ `_v2` is not decoration. The driver keeps the ORIGINAL 32-bit-pointer entry
        // points under the unsuffixed names for binary compatibility, so resolving
        // `cuMemAlloc` (rather than `cuMemAlloc_v2`) on a 64-bit host silently truncates every
        // device pointer. The same is true of `cuCtxCreate`, `cuMemcpy*` and `cuMemFree`.
        // ⇒ every size-carrying entry point here is asked for by its versioned name.
        // ★ The NULL-tolerant twin of `sym!`. See the VMM fields: a symbol this driver does
        // not have must yield `None` rather than refusing the whole binding.
        //
        // ⊘ It resolves through `sym_or_null` rather than repeating `sym!`'s body. The first
        // draft duplicated the `dlsym` + `transmute` pair, which added TWO more `unsafe`
        // blocks doing what two existing ones already did — and the crate's relaxation
        // ratchet is what said so. Duplicated `unsafe` is duplicated review surface.
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
        macro_rules! sym {
            ($name:literal) => {{
                let n = CString::new($name).expect("a literal with no NUL");
                // SAFETY: `handle` is a live library handle and `n` is NUL-terminated.
                let p = unsafe { dlsym(handle, n.as_ptr()) };
                if p.is_null() {
                    return Err(CudaError::MissingSymbol($name));
                }
                // SAFETY: the driver's ABI for this symbol is the signature at the field.
                unsafe { core::mem::transmute(p) }
            }};
        }
        // ⊘ 2026-11-13: through `sym_or_null` (it repeated that function's `dlsym` in a closure).
        let err_name = sym_or_null(handle, "cuGetErrorName");

        Ok(Cuda {
            _handle: handle,
            cuInit: sym!("cuInit"),
            cuDeviceGet: sym!("cuDeviceGet"),
            cuDeviceGetCount: sym!("cuDeviceGetCount"),
            cuDeviceGetName: sym!("cuDeviceGetName"),
            cuDeviceGetByPCIBusId: sym!("cuDeviceGetByPCIBusId"),
            cuDeviceGetAttribute: opt!("cuDeviceGetAttribute"),
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
            cuStreamCreate: sym!("cuStreamCreate"),
            cuStreamDestroy: sym!("cuStreamDestroy_v2"),
            cuEventCreate: sym!("cuEventCreate"),
            cuEventDestroy: sym!("cuEventDestroy_v2"),
            cuEventRecord: sym!("cuEventRecord"),
            cuEventQuery: sym!("cuEventQuery"),
            cuEventElapsedTime: sym!("cuEventElapsedTime"),
            cuLaunchHostFunc: sym!("cuLaunchHostFunc"),
            cuMemAllocHost: sym!("cuMemAllocHost_v2"),
            cuMemFreeHost: sym!("cuMemFreeHost"),
            cuMemHostGetDevicePointer: sym!("cuMemHostGetDevicePointer_v2"),
            cuMemcpyDtoHAsync: sym!("cuMemcpyDtoHAsync_v2"),
            cuMemcpyDtoDAsync: sym!("cuMemcpyDtoDAsync_v2"),
            cuMemsetD8Async: sym!("cuMemsetD8Async"),
            // ⚠ Versioned names, as everywhere here. `cuStreamBeginCapture` (unsuffixed) is the
            // CUDA 10.0 entry with no mode argument; `_v2` takes the mode. `_v2` of the capture
            // query and of the kernel-node setter take the CUDA 12 structs (the setter's is a
            // strict SUPERSET of the v1 struct, so the unsuffixed fallback reads a valid prefix).
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
            ctx_sync_calls: core::sync::atomic::AtomicU64::new(0),
            // ⊘ Resolved with a NULL-tolerant lookup, unlike `sym!`, for the reason the field
            // docs give: absent is an ANSWER here, not a load failure.
            cuMemImportFromShareableHandle: opt!("cuMemImportFromShareableHandle"),
            cuMemAddressReserve: opt!("cuMemAddressReserve"),
            cuMemMap: opt!("cuMemMap"),
            cuMemSetAccess: opt!("cuMemSetAccess"),
            cuMemUnmap: opt!("cuMemUnmap"),
            cuMemAddressFree: opt!("cuMemAddressFree"),
            cuMemRelease: opt!("cuMemRelease"),
            // ⚠ `_v2`, as everywhere a size is carried (the unsuffixed entry is the 32-bit ABI)
            cuMemHostRegister: opt!("cuMemHostRegister_v2"),
            cuMemHostUnregister: opt!("cuMemHostUnregister"),
            cuGetErrorName: if err_name.is_null() {
                None
            } else {
                // SAFETY: the driver's ABI for `cuGetErrorName`.
                Some(unsafe { core::mem::transmute(err_name) })
            },
        })
    }

    /// Turn a `CUresult` into a refusal that names the call.
    pub(crate) fn check(&self, what: &'static str, r: CUresult) -> Result<(), CudaError> {
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
}

/// ═══ THE SAFE SURFACE ═══════════════════════════════════════════════════════════════════
///
/// ★★★ Everything above this line is the foreign ABI; everything below is the **only** way
/// the rest of this crate reaches it. `walk.rs`, `selftest.rs`, `synth.rs` and `abi.rs`
/// contain **no `unsafe` at all**, which is what makes gate B's *"read the audited surface in
/// one sitting"* true of this crate: the surface is this file.
impl Cuda {
    /// `cuInit(0)`.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    pub fn init(&self) -> Result<(), CudaError> {
        crate::posture::cuda_thread()?;
        // SAFETY: `cuInit` takes an integer and returns a status; it has no pointer
        // arguments and no aliasing obligations at all.
        self.check("cuInit", unsafe { (self.cuInit)(0) })
    }

    /// `cuDeviceGetCount`.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    pub fn device_count(&self) -> Result<i32, CudaError> {
        #[cfg(test)]
        if self._handle.is_null() {
            return Ok(1); // the fake binding (tests) cannot write the out-pointer
        }
        let mut n: c_int = 0;
        // SAFETY: `n` is a live, correctly-aligned `c_int` for the whole call, which is the
        // only obligation the driver's out-pointer carries.
        self.check("cuDeviceGetCount", unsafe {
            (self.cuDeviceGetCount)(&raw mut n)
        })?;
        Ok(n)
    }

    /// `cuDeviceGet` for ordinal `ord`.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    pub fn device_get(&self, ord: i32) -> Result<i32, CudaError> {
        let mut d: c_int = 0;
        // SAFETY: as `device_count` — one live out-pointer, no aliasing.
        self.check("cuDeviceGet", unsafe {
            (self.cuDeviceGet)(&raw mut d, ord)
        })?;
        Ok(d)
    }

    /// ★ `cuDeviceGetByPCIBusId` — the CUDA device at PCI address `bdf`
    /// (`[domain]:[bus]:[device].[function]`, hex, as `CardInfo::bdf` spells it).
    ///
    /// # Errors
    /// [`CudaError::Refused`] — including `CUDA_ERROR_INVALID_DEVICE` when this process's CUDA
    /// cannot see that GPU (e.g. `CUDA_VISIBLE_DEVICES` excludes it): refused, never an
    /// ordinal fallback.
    pub fn device_by_pci_bus_id(&self, bdf: &str) -> Result<i32, CudaError> {
        let c = CString::new(bdf).map_err(|_| CudaError::Refused {
            what: "cuDeviceGetByPCIBusId",
            code: 0,
            name: format!("the PCI bus id {bdf:?} contains a NUL"),
        })?;
        let mut d: c_int = 0;
        // SAFETY: one live out-pointer and a NUL-terminated string that outlives the call.
        self.check("cuDeviceGetByPCIBusId", unsafe {
            (self.cuDeviceGetByPCIBusId)(&raw mut d, c.as_ptr())
        })?;
        Ok(d)
    }

    /// ★★ F-7 / S1-05 (security review 3, 2026-11-14): whether kernels on device `dev` can
    /// dereference PAGEABLE host memory (`CU_DEVICE_ATTRIBUTE_PAGEABLE_MEMORY_ACCESS`, 88: HMM or
    /// ATS). Where they can, a kernel address that leaves its buffer may reach the VMM's own memory,
    /// not just a fault — which is why every kernel bound here is a host-memory-safety obligation
    /// (OWNER_RULINGS: "HMM is not refused"). `None` when the driver has no entry or refuses the
    /// question; the caller logs the answer once, at bring-up.
    #[must_use]
    pub fn pageable_memory_access(&self, dev: i32) -> Option<bool> {
        const CU_DEVICE_ATTRIBUTE_PAGEABLE_MEMORY_ACCESS: c_int = 88;
        let f = self.cuDeviceGetAttribute?;
        let mut v: c_int = 0;
        // SAFETY: one live out-pointer; the attribute is a fixed enumerant and `dev` an ordinal
        // the driver bounds itself (an invalid one is an error code, not memory access).
        let r = unsafe { f(&raw mut v, CU_DEVICE_ATTRIBUTE_PAGEABLE_MEMORY_ACCESS, dev) };
        (r == CUDA_SUCCESS).then_some(v != 0)
    }

    /// `cuDeviceGetName`. ⊘ Never fails the caller: a device whose name we cannot read is
    /// still a device, and refusing the bring-up over a diagnostic string would be the
    /// instrument deciding the experiment.
    pub fn device_name(&self, dev: i32) -> String {
        let mut buf = [0i8; 128];
        // SAFETY: the buffer is live for the call and its length is passed as the bound the
        // driver is required to honour; the driver NUL-terminates within it.
        let r = unsafe {
            (self.cuDeviceGetName)(
                buf.as_mut_ptr().cast::<c_char>(),
                c_int::try_from(buf.len()).unwrap_or(128),
                dev,
            )
        };
        if r != CUDA_SUCCESS {
            return "<unnamed>".to_string();
        }
        // SAFETY: the call above succeeded, so the driver wrote a NUL-terminated string
        // inside `buf`, which is still live and owned here.
        unsafe { core::ffi::CStr::from_ptr(buf.as_ptr().cast::<c_char>()) }
            .to_string_lossy()
            .into_owned()
    }

    /// `cuCtxCreate_v2`. The returned handle is opaque and is only ever handed back. ★ The new
    /// context becomes current on this thread (the driver pushes it), and so does its liveness
    /// token in this binding's mirror of the thread's stack.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    pub fn ctx_create(&self, dev: i32) -> Result<CtxHandle, CudaError> {
        crate::posture::cuda_thread()?;
        let mut ctx: *mut c_void = core::ptr::null_mut();
        // SAFETY: one live out-pointer; the driver writes an opaque handle we never
        // dereference.
        self.check("cuCtxCreate_v2", unsafe {
            (self.cuCtxCreate)(&raw mut ctx, 0, dev)
        })?;
        let life = Life::new();
        CTX_STACK.with(|s| s.borrow_mut().push((ctx as usize, life.clone())));
        Ok(CtxHandle {
            raw: ctx as usize,
            life,
        })
    }

    /// ★ 2026-11-14 (security review 2 D-15): [`Self::ctx_create`] behind a guard that DESTROYS
    /// the context when it drops — so a bring-up that fails after the context exists (a `?` on any
    /// later step) gives back the context and with it every allocation, stream, event and module
    /// made in it, instead of leaking them for the life of the process. [`CtxGuard::disarm`] hands
    /// the handle over once the owner holds everything.
    ///
    /// # Errors
    /// As [`Self::ctx_create`].
    pub(crate) fn ctx_create_guarded(&self, dev: i32) -> Result<CtxGuard<'_>, CudaError> {
        Ok(CtxGuard {
            cu: self,
            ctx: Some(self.ctx_create(dev)?),
        })
    }

    /// `cuCtxDestroy_v2`. ⊘ Infallible by design: it runs in a `Drop` and there is nobody
    /// left to tell. ★ CONSUMES the handle and ends the context's liveness token, so every
    /// stream, event, module, function, graph and allocation minted in it refuses (panics)
    /// from now on instead of reaching the driver. A null handle destroys nothing.
    pub fn ctx_destroy(&self, ctx: CtxHandle) {
        if !ctx.life.alive() {
            return;
        }
        ctx.life.end();
        // SAFETY: `ctx` came from `cuCtxCreate_v2` and is destroyed exactly once: the handle is
        // not `Copy`/`Clone`, it is consumed here, and its token was alive (ended just above).
        unsafe { (self.cuCtxDestroy)(ctx.raw as *mut c_void) };
        CTX_STACK.with(|s| {
            let mut s = s.borrow_mut();
            if s.last().is_some_and(|(_, l)| Arc::ptr_eq(l, &ctx.life)) {
                s.pop();
            }
        });
    }

    /// ★★★★★ `cuCtxSetCurrent` — **BIND THIS CONTEXT TO THE CALLING THREAD.**
    ///
    /// ⊘⊘⊘ **A CUDA context is CURRENT PER THREAD, and this cost a boot.**
    /// `[measured 2026-09-15, w731, RTX 3060, 580.159.04]` the scratchpad isolate brings CUDA
    /// up on its **startup** thread (before the sandbox, as §w724d requires), and
    /// `cuCtxCreate` makes the context current **only there**. A later request is served on a
    /// **worker** thread, which has no current context — so the first `cuMemAlloc` of the live
    /// walk shadow returned **`CUDA_ERROR_INVALID_CONTEXT` (201)**, 2 115 times, and the
    /// census came back `compared=0 skipped[isolate_refused=2115]`.
    ///
    /// ⚠ The selftest could not have caught it: `bring_up_and_prove` runs on the **same**
    /// thread as the bring-up, so `CUDA_WALK=OK` and both post-sandbox probes passed on the
    /// very boot where every cross-thread call refused.
    ///
    /// # Errors
    /// [`CudaError`].
    ///
    /// # Panics
    /// When `ctx` was destroyed (or is the null handle).
    pub fn ctx_set_current(&self, ctx: &CtxHandle) -> Result<(), CudaError> {
        crate::posture::cuda_thread()?;
        live(&ctx.life, "cuCtxSetCurrent");
        // SAFETY: `ctx` came from this library's own `cuCtxCreate_v2` and is alive (checked
        // above). `cuCtxSetCurrent` takes the handle and affects the calling thread only.
        self.check("cuCtxSetCurrent", unsafe {
            (self.cuCtxSetCurrent)(ctx.raw as *mut c_void)
        })?;
        // the driver REPLACES the top of this thread's stack (or pushes onto an empty one)
        CTX_STACK.with(|s| {
            let mut s = s.borrow_mut();
            s.pop();
            s.push((ctx.raw, ctx.life.clone()));
        });
        Ok(())
    }

    /// `cuCtxSynchronize`.
    ///
    /// # Errors
    /// [`CudaError::Refused`] — including a device-side fault raised by an earlier launch,
    /// which is where an illegal access surfaces.
    pub fn ctx_synchronize(&self) -> Result<(), CudaError> {
        self.ctx_sync_calls
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        // SAFETY: no arguments.
        self.check("cuCtxSynchronize", unsafe { (self.cuCtxSynchronize)() })
    }

    /// `cuModuleLoadData` over a NUL-terminated PTX image. **This is where the PTX JIT runs.**
    ///
    /// ★ D-01 (2026-11-13): the image's `.entry` parameter lists are parsed HERE ([`ptx_entries`])
    /// and kept with the module, so every [`Func`] knows its parameters' sizes and every launch
    /// can be checked against them. An image whose entry lists do not parse is refused by name:
    /// a launch this binding cannot check is a launch it does not make.
    ///
    /// # Errors
    /// [`CudaError::Refused`] — a driver too old for the PTX ISA version refuses here, by
    /// name, which is the diagnosis a reader needs; or the parameter lists did not parse; or no
    /// context of this binding is current.
    ///
    /// # Panics
    /// When the image has no terminating NUL (a bug in the caller, refused before the driver
    /// reads past the buffer).
    pub fn module_load(&self, ptx_with_nul: &[u8]) -> Result<ModuleHandle, CudaError> {
        assert!(
            ptx_with_nul.last() == Some(&0),
            "cuModuleLoadData reads until a NUL and the image handed to it has none; \
             passing it would read past the end of our own buffer"
        );
        let entries = ptx_entries(&ptx_with_nul[..ptx_with_nul.len() - 1]).map_err(|name| {
            CudaError::Refused {
                what: "cuModuleLoadData (PTX entry parameter lists)",
                code: 0,
                name,
            }
        })?;
        let life = current_life("cuModuleLoadData")?;
        let mut m: *mut c_void = core::ptr::null_mut();
        // SAFETY: the image is a live slice for the whole call and is NUL-terminated (checked
        // immediately above), which is the driver's documented requirement; `m` is a live
        // out-pointer.
        self.check("cuModuleLoadData", unsafe {
            (self.cuModuleLoadData)(&raw mut m, ptx_with_nul.as_ptr().cast::<c_void>())
        })?;
        Ok(ModuleHandle {
            raw: m as usize,
            life,
            entries,
        })
    }

    /// `cuModuleGetFunction`. ★ The [`Func`] carries the entry's parameter sizes as the module's
    /// PTX text declares them (D-01).
    ///
    /// # Errors
    /// [`CudaError::Refused`], renamed to say **which** entry point is absent — from the PTX
    /// text (refused before the driver is asked) or from the driver.
    ///
    /// # Panics
    /// When `m`'s context was destroyed.
    pub fn module_function(&self, m: &ModuleHandle, sym: &'static str) -> Result<Func, CudaError> {
        live(&m.life, "cuModuleGetFunction");
        let Some(entry) = m.entries.iter().find(|e| e.name == sym) else {
            return Err(CudaError::Refused {
                what: "cuModuleGetFunction",
                code: 0,
                name: format!("the module's PTX text declares no entry `{sym}`"),
            });
        };
        let c = CString::new(sym).map_err(|_| CudaError::MissingSymbol(sym))?;
        let mut f: *mut c_void = core::ptr::null_mut();
        // SAFETY: `m` is a live handle this binding produced (checked above), `c` is
        // NUL-terminated and live for the call, and `f` is a live out-pointer.
        let r = unsafe { (self.cuModuleGetFunction)(&raw mut f, m.raw as *mut c_void, c.as_ptr()) };
        self.check("cuModuleGetFunction", r).map_err(|e| match e {
            CudaError::Refused { code, .. } => CudaError::Refused {
                what: "cuModuleGetFunction",
                code,
                name: format!("the committed PTX has no entry `{sym}`"),
            },
            other => other,
        })?;
        Ok(Func {
            raw: f as usize,
            life: m.life.clone(),
            name: sym,
            params: entry.params.clone().into(),
        })
    }

    /// `cuMemAlloc_v2` followed by `cuMemsetD8_v2` to zero. ⊘ The zeroing is not hygiene: the
    /// kernel reads its own `KfDev` state, and uninitialised device memory would make the
    /// first refresh's behaviour a function of what the previous tenant left behind.
    ///
    /// ★ Returns a [`DevBuf`]: the address and its length, together, minted here and only here
    /// (and by the import and pinned-view constructors). ⊘ 2026-11-13: a failed zeroing frees the
    /// allocation instead of leaking it (D-15).
    ///
    /// # Errors
    /// [`CudaError::Refused`]; also refused when no context of this binding is current.
    pub fn mem_alloc_zeroed(&self, bytes: usize, what: &'static str) -> Result<DevBuf, CudaError> {
        let life = current_life(what)?;
        let mut p: CUdeviceptr = 0;
        // SAFETY: one live out-pointer; `bytes` is a length the driver owns entirely.
        self.check(what, unsafe { (self.cuMemAlloc)(&raw mut p, bytes) })?;
        let buf = DevBuf {
            ptr: p,
            len: bytes,
            kind: DevKind::Owned,
            life,
            owner: None,
        };
        // SAFETY: `p` is the allocation just returned and `bytes` is exactly its length.
        if let Err(e) = self.check(what, unsafe { (self.cuMemsetD8)(p, 0, bytes) }) {
            self.mem_free(buf);
            return Err(e);
        }
        Ok(buf)
    }

    /// `cuMemFree_v2`. ⊘ Infallible: it runs in a `Drop`. ★ CONSUMES the buffer, so it cannot be
    /// freed twice or used after; only an [`DevKind::Owned`] allocation of a LIVE context is
    /// handed to the driver (an import or a pinned view is the context's, reclaimed with it; an
    /// allocation of a destroyed context was already reclaimed by `cuCtxDestroy`).
    pub fn mem_free(&self, b: DevBuf) {
        if b.kind == DevKind::Owned && b.ptr != 0 && b.life.alive() {
            // SAFETY: `b` was minted by `mem_alloc_zeroed` in a context that is still alive, and
            // is consumed here: `DevBuf` is neither `Copy` nor `Clone`, so this is its only free.
            unsafe { (self.cuMemFree)(b.ptr) };
        }
    }

    /// `cuMemcpyHtoD_v2` from a host slice into the first `src.len()` bytes of `dst`.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    ///
    /// # Panics
    /// When `src` is longer than `dst`, or `dst`'s context was destroyed.
    pub fn memcpy_h2d(
        &self,
        dst: DevSpan<'_>,
        src: &[u8],
        what: &'static str,
    ) -> Result<(), CudaError> {
        dst.check(src.len(), what);
        // SAFETY: `src` is a live slice for the call and the byte count passed is its own
        // length, so the driver cannot read past it; `[dst.ptr, dst.ptr + src.len())` lies inside
        // a live device allocation (`DevSpan::check`, just above).
        self.check(what, unsafe {
            (self.cuMemcpyHtoD)(dst.ptr, src.as_ptr().cast::<c_void>(), src.len())
        })
    }

    /// `cuMemcpyDtoH_v2` of the first `dst.len()` bytes of `src` into a host slice.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    ///
    /// # Panics
    /// When `dst` is longer than `src`, or `src`'s context was destroyed.
    pub fn memcpy_d2h(
        &self,
        dst: &mut [u8],
        src: DevSpan<'_>,
        what: &'static str,
    ) -> Result<(), CudaError> {
        src.check(dst.len(), what);
        // SAFETY: `dst` is a live, exclusively-borrowed slice and the byte count passed is
        // its own length, so the driver cannot write past it; the device range lies inside a
        // live allocation (`DevSpan::check`, just above).
        self.check(what, unsafe {
            (self.cuMemcpyDtoH)(dst.as_mut_ptr().cast::<c_void>(), src.ptr, dst.len())
        })
    }

    /// `cuLaunchKernel` with **one** by-value parameter, whose bytes are `param`.
    ///
    /// ⚠ The `CUresult` is returned rather than checked, because [`Cuda::launch_raw`]'s one
    /// deliberate caller is the failed-launch probe, for which a refusal is the **expected**
    /// outcome and an error return would discard it.
    ///
    /// # Panics
    /// When `f` does not take exactly one parameter of `param.len()` bytes (D-01), or its
    /// context was destroyed.
    pub fn launch_raw(&self, f: &Func, grid: u32, block: u32, param: &mut [u8]) -> CUresult {
        f.check_params(&[param.len()], "cuLaunchKernel");
        let mut p: [*mut c_void; 1] = [param.as_mut_ptr().cast::<c_void>()];
        // SAFETY: `param` is a live, exclusively-borrowed buffer holding the kernel's
        // by-value argument, and `p` is a live array of one pointer into it. The driver reads
        // exactly the parameters the module declares: ONE, of `param.len()` bytes, which
        // `Func::check_params` asserted just above against the module's own PTX text.
        unsafe {
            (self.cuLaunchKernel)(
                f.raw as *mut c_void,
                grid,
                1,
                1,
                block,
                1,
                1,
                0,
                core::ptr::null_mut(),
                p.as_mut_ptr(),
                core::ptr::null_mut(),
            )
        }
    }

    /// ★ w826 — launch with SEVERAL by-value parameters and dynamic shared memory: the parallel
    /// walk's kernels take pointers and scalars beside `KfArgs`. Each element of `params` is
    /// one parameter's bytes, in declaration order.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    ///
    /// # Panics
    /// When `params` does not match `f`'s declared parameters in number and every size (D-01),
    /// or a handle's context was destroyed.
    #[allow(clippy::too_many_arguments)]
    pub fn launch_args(
        &self,
        stream: &StreamHandle,
        f: &Func,
        grid: u32,
        block: u32,
        shmem: u32,
        params: &mut [Vec<u8>],
        what: &'static str,
    ) -> Result<(), CudaError> {
        live(&stream.life, what);
        let sizes: Vec<usize> = params.iter().map(Vec::len).collect();
        f.check_params(&sizes, what);
        let mut p: Vec<*mut c_void> = params
            .iter_mut()
            .map(|b| b.as_mut_ptr().cast::<c_void>())
            .collect();
        // SAFETY: every element of `p` points into a live, exclusively-borrowed Vec in
        // `params`; `Func::check_params` asserted above that there is exactly one per parameter
        // of `f` and that each holds exactly that parameter's byte size, as the module's PTX
        // declares it. The driver copies them during the call; `stream` is alive.
        let r = unsafe {
            (self.cuLaunchKernel)(
                f.raw as *mut c_void,
                grid,
                1,
                1,
                block,
                1,
                1,
                shmem,
                stream.raw as *mut c_void,
                p.as_mut_ptr(),
                core::ptr::null_mut(),
            )
        };
        self.check(what, r)
    }

    /// `cuMemcpyDtoDAsync_v2` — device to device, ordered in `stream` with the kernels around
    /// it: the whole of `src` into the first `src.len()` bytes of `dst`. ⊘ Was the synchronous
    /// `cuMemcpyDtoD_v2` on the legacy stream until P4 moved the walk onto its own stream; a
    /// legacy-stream copy between two stream launches is ordered only by the blocking-stream
    /// rule, which is a property of how the stream was created and not something this call site
    /// should lean on.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    ///
    /// # Panics
    /// When `src` is longer than `dst`, or a handle's context was destroyed.
    pub fn memcpy_d2d_async(
        &self,
        stream: &StreamHandle,
        dst: DevSpan<'_>,
        src: DevSpan<'_>,
        what: &'static str,
    ) -> Result<(), CudaError> {
        live(&stream.life, what);
        src.check(src.len, what);
        dst.check(src.len, what);
        // SAFETY: both ranges lie inside live device allocations and the count is `src`'s
        // length, which `dst` holds (both checked above); the driver reads and writes device
        // memory only, and `stream` is a live handle this binding produced.
        self.check(what, unsafe {
            (self.cuMemcpyDtoDAsync)(dst.ptr, src.ptr, src.len, stream.raw as *mut c_void)
        })
    }

    /// `cuMemsetD8_v2` over the whole of `dst` (synchronous, the legacy stream).
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    ///
    /// # Panics
    /// When `dst`'s context was destroyed.
    pub fn memset_d8(&self, dst: DevSpan<'_>, v: u8, what: &'static str) -> Result<(), CudaError> {
        dst.check(dst.len, what);
        // SAFETY: `[dst.ptr, dst.ptr + dst.len)` lies inside a live device allocation
        // (`DevSpan` is minted only by a bounds-checked `DevBuf::span`; the context is checked
        // alive just above).
        self.check(what, unsafe { (self.cuMemsetD8)(dst.ptr, v, dst.len) })
    }

    /// `cuMemsetD8Async` over the whole of `dst`, in `stream`.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    ///
    /// # Panics
    /// When a handle's context was destroyed.
    pub fn memset_d8_async(
        &self,
        stream: &StreamHandle,
        dst: DevSpan<'_>,
        v: u8,
        what: &'static str,
    ) -> Result<(), CudaError> {
        live(&stream.life, what);
        dst.check(dst.len, what);
        // SAFETY: as `memset_d8`; `stream` is a live handle this binding produced.
        self.check(what, unsafe {
            (self.cuMemsetD8Async)(dst.ptr, v, dst.len, stream.raw as *mut c_void)
        })
    }

    /// How many `cuCtxSynchronize` calls this binding has made — see the field.
    #[must_use]
    pub fn ctx_sync_calls(&self) -> u64 {
        self.ctx_sync_calls
            .load(core::sync::atomic::Ordering::Relaxed)
    }

    /// `cuStreamCreate(flags = 0)` — ★ a **blocking** stream, deliberately.
    ///
    /// ⊘ Not `CU_STREAM_NON_BLOCKING`: a blocking stream is ordered against the legacy default
    /// stream, so a synchronous `cuMemcpyHtoD` a harness issues before a walk (writing the
    /// guest's tables, `WalkKernel::write_at`) is complete before the walk reads them, and a
    /// synchronous read-back after a collected walk sees what the walk saw. A non-blocking
    /// stream would make that ordering every caller's problem, silently.
    ///
    /// # Errors
    /// [`CudaError::Refused`]; also refused when no context of this binding is current.
    pub fn stream_create(&self) -> Result<StreamHandle, CudaError> {
        let life = current_life("cuStreamCreate")?;
        let mut h: *mut c_void = core::ptr::null_mut();
        // SAFETY: one live out-pointer; the driver writes an opaque handle.
        self.check("cuStreamCreate", unsafe {
            (self.cuStreamCreate)(&raw mut h, 0)
        })?;
        Ok(StreamHandle {
            raw: h as usize,
            life,
        })
    }

    /// `cuStreamDestroy_v2`. ⊘ Infallible: it runs in a `Drop`. ★ Consumes the handle; a stream
    /// of a destroyed context was reclaimed with it and is not handed to the driver.
    pub fn stream_destroy(&self, s: StreamHandle) {
        if s.raw != 0 && s.life.alive() {
            // SAFETY: `s` came from `stream_create` in a context that is alive and is consumed
            // here (not `Copy`/`Clone`), so it is destroyed exactly once.
            unsafe { (self.cuStreamDestroy)(s.raw as *mut c_void) };
        }
    }

    /// `cuEventCreate(flags = 0)` — timing enabled, so a walk's GPU time is measurable.
    ///
    /// # Errors
    /// [`CudaError::Refused`]; also refused when no context of this binding is current.
    pub fn event_create(&self) -> Result<EventHandle, CudaError> {
        let life = current_life("cuEventCreate")?;
        let mut h: *mut c_void = core::ptr::null_mut();
        // SAFETY: one live out-pointer.
        self.check("cuEventCreate", unsafe {
            (self.cuEventCreate)(&raw mut h, 0)
        })?;
        Ok(EventHandle {
            raw: h as usize,
            life,
        })
    }

    /// `cuEventDestroy_v2`. ⊘ Infallible: it runs in a `Drop`. ★ Consumes the handle.
    pub fn event_destroy(&self, e: EventHandle) {
        if e.raw != 0 && e.life.alive() {
            // SAFETY: `e` came from `event_create` in a live context and is consumed here.
            unsafe { (self.cuEventDestroy)(e.raw as *mut c_void) };
        }
    }

    /// `cuEventRecord(e, stream)`.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    ///
    /// # Panics
    /// When a handle's context was destroyed.
    pub fn event_record(&self, e: &EventHandle, s: &StreamHandle) -> Result<(), CudaError> {
        live(&e.life, "cuEventRecord");
        live(&s.life, "cuEventRecord");
        // SAFETY: both handles came from this binding and are alive (checked above).
        self.check("cuEventRecord", unsafe {
            (self.cuEventRecord)(e.raw as *mut c_void, s.raw as *mut c_void)
        })
    }

    /// `cuEventQuery` — **never blocks.** `Ok(true)`: the work before the record is done;
    /// `Ok(false)`: not yet ([`CUDA_ERROR_NOT_READY`]); `Err`: the stream failed (a device
    /// fault in an earlier launch surfaces here, by name).
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    ///
    /// # Panics
    /// When `e`'s context was destroyed.
    pub fn event_query(&self, e: &EventHandle) -> Result<bool, CudaError> {
        live(&e.life, "cuEventQuery");
        // SAFETY: `e` came from this binding and is alive.
        let r = unsafe { (self.cuEventQuery)(e.raw as *mut c_void) };
        if r == CUDA_ERROR_NOT_READY {
            return Ok(false);
        }
        self.check("cuEventQuery", r).map(|()| true)
    }

    /// `cuEventElapsedTime(start, end)`, in microseconds. Both must have completed.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    ///
    /// # Panics
    /// When a handle's context was destroyed.
    pub fn event_elapsed_us(
        &self,
        start: &EventHandle,
        end: &EventHandle,
    ) -> Result<u64, CudaError> {
        live(&start.life, "cuEventElapsedTime");
        live(&end.life, "cuEventElapsedTime");
        let mut ms: f32 = 0.0;
        // SAFETY: one live out-pointer; both handles came from this binding and are alive.
        self.check("cuEventElapsedTime", unsafe {
            (self.cuEventElapsedTime)(
                &raw mut ms,
                start.raw as *mut c_void,
                end.raw as *mut c_void,
            )
        })?;
        Ok((f64::from(ms) * 1000.0) as u64)
    }

    /// ★★★ `cuLaunchHostFunc(stream, signal, fd)` — once every earlier operation in `stream`
    /// has completed, a driver thread writes `1` to `fd`. **This is how a walk completes: a
    /// host event on an fd, never an inline wait** (owner rule; `THE_TRANSLATED_PLANE.md` §5,
    /// *"the walk is one more epoll entry"*).
    ///
    /// ⊘ The callback makes no CUDA call (the driver forbids it) — it is one `write(2)`.
    ///
    /// ★ D-07 (2026-11-13): the callback OWNS a reference to the fd (an `Arc` handed to the
    /// driver as the user datum and taken back by the callback), so dropping the
    /// [`CompletionFd`] while a signal is queued can no longer make the driver thread write to a
    /// closed — or reused — fd number. The fd closes when the last reference goes.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    ///
    /// # Panics
    /// When `s`'s context was destroyed.
    pub fn launch_host_signal(&self, s: &StreamHandle, fd: &CompletionFd) -> Result<(), CudaError> {
        live(&s.life, "cuLaunchHostFunc");
        let user = Arc::into_raw(fd.fd.clone()).cast_mut().cast::<c_void>();
        // SAFETY: `s` is alive; `user` is one strong `Arc<OwnedFd>` reference, which the driver
        // hands to `completion_hostfn` exactly once and which that function takes back.
        let r = unsafe { (self.cuLaunchHostFunc)(s.raw as *mut c_void, completion_hostfn, user) };
        if r != CUDA_SUCCESS {
            // SAFETY: the launch was refused, so the driver will never call the function: the
            // reference made by `Arc::into_raw` just above is still ours, and is reclaimed once.
            drop(unsafe { Arc::from_raw(user.cast_const().cast::<std::os::fd::OwnedFd>()) });
        }
        self.check("cuLaunchHostFunc", r)
    }

    /// `cuMemAllocHost_v2` — page-locked host memory an async copy can target.
    ///
    /// # Errors
    /// [`CudaError::Refused`]; also refused when no context of this binding is current.
    pub(crate) fn pinned_alloc(
        &self,
        len: usize,
        what: &'static str,
    ) -> Result<PinnedBuf, CudaError> {
        let ctx = current_life(what)?;
        #[cfg(test)]
        if self._handle.is_null() {
            // the fake binding (tests): real host memory, leaked, stands in for a pinned buffer
            let mem: &'static mut [u8] = Box::leak(vec![0u8; len.max(1)].into_boxed_slice());
            return Ok(PinnedBuf {
                ptr: mem.as_mut_ptr() as usize,
                len,
                ctx,
                own: Life::new(),
                dma: PinnedDma::default(),
            });
        }
        let mut p: *mut c_void = core::ptr::null_mut();
        // SAFETY: one live out-pointer; `len` is owned by the driver entirely.
        self.check(what, unsafe { (self.cuMemAllocHost)(&raw mut p, len) })?;
        Ok(PinnedBuf {
            ptr: p as usize,
            len,
            ctx,
            own: Life::new(),
            dma: PinnedDma::default(),
        })
    }

    /// `cuMemFreeHost` — give back a [`Self::pinned_alloc`] buffer. ★ Consumes it and ends its
    /// token, so a device view of it ([`Self::pinned_device_view`]) refuses from now on.
    ///
    /// ⊘⊘ 2026-11-14 (security review 2 D-05(b)), above the contract it replaces: this used to
    /// carry a CALLER-CONTRACT ("no queued copy may still target the buffer"). Now the binding
    /// knows ([`PinnedDma`]): while the last queued copy's event has not completed, or completion
    /// is unknowable, or a device view was lent to a kernel, the pages are NOT freed — the buffer
    /// is consumed and left to `cuCtxDestroy`, and the refusal says why. A DMA can no longer land
    /// in freed host pages.
    ///
    /// # Errors
    /// [`CudaError::Refused`]: the driver's, or "the GPU may still write it" (the buffer is then
    /// kept until the context dies).
    pub(crate) fn pinned_free(&self, buf: PinnedBuf, what: &'static str) -> Result<(), CudaError> {
        buf.own.end();
        if !buf.ctx.alive() {
            // the context's destruction already reclaimed it
            return Ok(());
        }
        self.settle(&buf, what)?;
        // SAFETY: `buf.ptr` is the base of a live `cuMemAllocHost` allocation of a live context,
        // consumed here, so no safe code can reach it afterwards; no GPU write to it can still be
        // pending (`settle`, just above).
        self.check(what, unsafe {
            (self.cuMemFreeHost)(buf.ptr as *mut c_void)
        })
    }

    /// ★ `cuMemHostRegister_v2` (flags 0: portable to no other context, not device-mapped) —
    /// page-lock `span`, a mapping this process owns, so an async copy can target it. The
    /// returned [`PinnedBuf`] names the same pages; [`Self::host_unregister`] gives it back.
    ///
    /// CALLER-CONTRACT: the mapping behind `span` must outlive the registration (`display::Frame`
    /// owns both and unregisters before it unmaps). `HostSpan` carries no lifetime (kf-linux-raw,
    /// review 2 A-01, not this crate's type). Otherwise: the pinned pages are unmapped under a
    /// registration.
    ///
    /// # Errors
    /// [`CudaError::MissingSymbol`] on a driver without the entry; [`CudaError::Refused`].
    pub(crate) fn host_register(
        &self,
        span: &kf_linux_raw::HostSpan,
        what: &'static str,
    ) -> Result<PinnedBuf, CudaError> {
        let f = self
            .cuMemHostRegister
            .ok_or(CudaError::MissingSymbol("cuMemHostRegister_v2"))?;
        let ctx = current_life(what)?;
        // SAFETY: the span is minted by a live mapping the caller keeps alive for as long as the
        // registration exists (CALLER-CONTRACT above); handing it to the driver as a page-lock
        // target is the use `HostSpan::as_ptr` permits, and no Rust reference into it is formed.
        let p = unsafe { span.as_ptr() };
        // SAFETY: `[p, p + len)` is the live mapping above; the driver pins those pages and
        // dereferences nothing on our behalf. Flags 0 request nothing beyond the pin.
        self.check(what, unsafe { f(p.cast::<c_void>(), span.len(), 0) })?;
        Ok(PinnedBuf {
            ptr: p as usize,
            len: span.len(),
            ctx,
            own: Life::new(),
            dma: PinnedDma::default(),
        })
    }

    /// ★ `cuMemHostUnregister` — undo [`Self::host_register`]. ★ 2026-11-14 (D-05(b)): refused,
    /// leaving the pages registered until the context dies, while the GPU may still write them
    /// (as [`Self::pinned_free`]); the caller then must keep the mapping (`display::Frame` does).
    ///
    /// # Errors
    /// [`CudaError::MissingSymbol`]; [`CudaError::Refused`], as [`Self::pinned_free`].
    pub(crate) fn host_unregister(
        &self,
        buf: PinnedBuf,
        what: &'static str,
    ) -> Result<(), CudaError> {
        let f = self
            .cuMemHostUnregister
            .ok_or(CudaError::MissingSymbol("cuMemHostUnregister"))?;
        buf.own.end();
        if !buf.ctx.alive() {
            return Ok(());
        }
        self.settle(&buf, what)?;
        // SAFETY: `buf.ptr` is the base of a live registration made by `host_register` in a live
        // context, consumed here, and no GPU write to it can still be pending (`settle`, just
        // above); the driver unpins and dereferences nothing.
        self.check(what, unsafe { f(buf.ptr as *mut c_void) })
    }

    /// ★ D-05(b): may `buf`'s pages go back? Only when no GPU write can still land in them: no
    /// device view was ever lent, and the last queued copy's event completed (it is destroyed
    /// then). Refuses by name otherwise; the caller keeps the pages until the context dies.
    fn settle(&self, buf: &PinnedBuf, what: &'static str) -> Result<(), CudaError> {
        let busy = |why: &str| {
            Err(CudaError::Refused {
                what,
                code: 0,
                name: format!(
                    "{why}: the GPU may still write these pinned pages, so they are kept until \
                     the context is destroyed (D-05(b))"
                ),
            })
        };
        if buf.dma.viewed.load(core::sync::atomic::Ordering::Acquire) {
            return busy("a device view of the buffer was lent to a kernel");
        }
        let mut fence = buf
            .dma
            .fence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match core::mem::take(&mut *fence) {
            Fence::Clear => Ok(()),
            Fence::Unknown => {
                *fence = Fence::Unknown;
                busy("a copy was queued whose completion could not be recorded")
            }
            Fence::After(ev) => match self.event_query(&ev) {
                Ok(true) => {
                    self.event_destroy(ev);
                    Ok(())
                }
                Ok(false) => {
                    *fence = Fence::After(ev);
                    busy("the last queued copy has not completed")
                }
                Err(e) => {
                    *fence = Fence::After(ev);
                    Err(e)
                }
            },
        }
    }

    /// `cuMemHostGetDevicePointer_v2` — a DEVICE view of `buf[off..off + len]`, so a kernel can
    /// read the pinned bytes directly (zero-copy) instead of through a copy node. ★ The view is a
    /// [`DevBuf`] of exactly `len` bytes; it refuses (panics) once `buf` is freed or its context
    /// destroyed.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    ///
    /// # Panics
    /// If `[off, off + len)` leaves `buf` (checked arithmetic), or `buf` was released.
    pub(crate) fn pinned_device_view(
        &self,
        buf: &PinnedBuf,
        off: usize,
        len: usize,
    ) -> Result<DevBuf, CudaError> {
        assert!(
            off.checked_add(len).is_some_and(|e| e <= buf.len),
            "pinned_device_view: [{off:#x}, +{len:#x}) leaves the {:#x}-byte pinned buffer",
            buf.len
        );
        live(&buf.ctx, "cuMemHostGetDevicePointer_v2");
        live(&buf.own, "cuMemHostGetDevicePointer_v2");
        // D-05(b): from now on only the context's destruction gives these pages back
        buf.dma
            .viewed
            .store(true, core::sync::atomic::Ordering::Release);
        let mut d: CUdeviceptr = 0;
        // SAFETY: one live out-pointer; `buf.ptr` is the base of a live `cuMemAllocHost`
        // allocation of a live context (checked above), which is what the call requires.
        self.check("cuMemHostGetDevicePointer_v2", unsafe {
            (self.cuMemHostGetDevicePointer)(&raw mut d, buf.ptr as *mut c_void, 0)
        })?;
        Ok(DevBuf {
            ptr: d.wrapping_add(off as u64),
            len,
            kind: DevKind::PinnedView,
            life: buf.ctx.clone(),
            owner: Some(buf.own.clone()),
        })
    }

    // ⊘ `memcpy_h2d_async` (no caller since P4b, "kept as the audited spelling") was removed
    // 2026-11-13: dead code in the audited file is review surface with no test reaching it.

    /// `cuMemcpyDtoHAsync_v2` of the whole of `src` into `buf[off..off + src.len()]`, in `stream`.
    ///
    /// ★ 2026-11-14 (D-05(b)): an event is recorded on `s` right after the copy and kept in `buf`
    /// ([`PinnedDma`]), so `buf` cannot be given back while the copy may still land. ⊘ Was a
    /// CALLER-CONTRACT ("do not free `buf` until the stream passed this copy").
    ///
    /// CALLER-CONTRACT: the caller does not READ the range until it observed the completion queued
    /// after this copy. Not validated here: `PinnedBuf::read` holds no binding to ask the fence
    /// with. Otherwise: a torn read of frame bytes — a wrong picture, never memory unsafety (the
    /// read is a raw-pointer copy out of live pages; freeing them is refused above).
    ///
    /// # Errors
    /// [`CudaError::Refused`]. ⚠ An error after the copy was queued (the fence event could not be
    /// made or recorded) leaves `buf` unfreeable until the context dies.
    ///
    /// # Panics
    /// If the range leaves `buf` (checked arithmetic), or a handle was released.
    pub(crate) fn memcpy_d2h_async(
        &self,
        s: &StreamHandle,
        buf: &PinnedBuf,
        off: usize,
        src: DevSpan<'_>,
        what: &'static str,
    ) -> Result<(), CudaError> {
        let n = src.len;
        assert!(
            off.checked_add(n).is_some_and(|e| e <= buf.len),
            "{what}: range leaves the pinned buffer"
        );
        live(&s.life, what);
        live(&buf.ctx, what);
        live(&buf.own, what);
        src.check(n, what);
        // SAFETY: `[off, off+n)` lies inside the live pinned allocation and the device range
        // inside a live device allocation (both checked above); `s` is alive. The pages cannot be
        // given back before this copy completed (`fence_after`, below; `settle`).
        self.check(what, unsafe {
            (self.cuMemcpyDtoHAsync)(
                (buf.ptr + off) as *mut c_void,
                src.ptr,
                n,
                s.raw as *mut c_void,
            )
        })?;
        self.fence_after(buf, s)
    }

    /// ★ D-05(b): record `buf`'s fence on `s`, after the copy just queued (its event is made once
    /// per buffer and re-recorded). A failure makes the buffer's completion unknowable.
    fn fence_after(&self, buf: &PinnedBuf, s: &StreamHandle) -> Result<(), CudaError> {
        let mut fence = buf
            .dma
            .fence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let ev = match core::mem::take(&mut *fence) {
            Fence::After(ev) => ev,
            Fence::Unknown => {
                *fence = Fence::Unknown;
                return Ok(());
            }
            Fence::Clear => match self.event_create() {
                Ok(ev) => ev,
                Err(e) => {
                    *fence = Fence::Unknown;
                    return Err(e);
                }
            },
        };
        match self.event_record(&ev, s) {
            Ok(()) => {
                *fence = Fence::After(ev);
                Ok(())
            }
            Err(e) => {
                self.event_destroy(ev);
                *fence = Fence::Unknown;
                Err(e)
            }
        }
    }

    /// ★ Whether every graph entry point the walk's one-call submission needs resolved.
    #[must_use]
    pub fn has_graph_api(&self) -> bool {
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

    fn graph_fn<T: Copy>(f: Option<T>, name: &'static str) -> Result<T, CudaError> {
        f.ok_or(CudaError::MissingSymbol(name))
    }

    /// `cuStreamBeginCapture_v2(s, THREAD_LOCAL)` — every operation queued on `s` from here to
    /// [`Cuda::stream_end_capture`] is RECORDED into a graph and not executed. ★ The returned
    /// [`Capture`] borrows `s` (the stream cannot be destroyed mid-capture) and names the graph
    /// the capture will become, so its nodes can be tied to that graph.
    ///
    /// # Errors
    /// [`CudaError`].
    ///
    /// # Panics
    /// When `s`'s context was destroyed.
    pub fn stream_begin_capture<'s>(&self, s: &'s StreamHandle) -> Result<Capture<'s>, CudaError> {
        let f = Self::graph_fn(self.cuStreamBeginCapture, "cuStreamBeginCapture_v2")?;
        live(&s.life, "cuStreamBeginCapture_v2");
        // SAFETY: `s` came from `stream_create` and is alive; the mode is a documented enumerator.
        self.check("cuStreamBeginCapture_v2", unsafe {
            f(s.raw as *mut c_void, CU_STREAM_CAPTURE_MODE_THREAD_LOCAL)
        })?;
        Ok(Capture {
            stream: s,
            graph: Life::new(),
        })
    }

    /// `cuStreamEndCapture` — the graph recorded since [`Cuda::stream_begin_capture`]. ⚠ Must be
    /// called on every path out of a capture, the failing ones included, or the stream stays in
    /// capture mode and every later submission on it is recorded instead of run. ★ Consumes the
    /// [`Capture`].
    ///
    /// # Errors
    /// [`CudaError`] (the capture was invalidated); the stream has left capture mode either way.
    pub fn stream_end_capture(&self, c: Capture<'_>) -> Result<GraphHandle, CudaError> {
        let f = Self::graph_fn(self.cuStreamEndCapture, "cuStreamEndCapture")?;
        live(&c.stream.life, "cuStreamEndCapture");
        let mut g: *mut c_void = core::ptr::null_mut();
        // SAFETY: the stream is alive and capturing (the `Capture` exists); one live out-pointer.
        let r = unsafe { f(c.stream.raw as *mut c_void, &raw mut g) };
        let graph = GraphHandle {
            raw: g as usize,
            life: c.stream.life.clone(),
            graph: c.graph,
        };
        if r != CUDA_SUCCESS {
            if !g.is_null() {
                self.graph_destroy(graph);
            }
            self.check("cuStreamEndCapture", r)?;
            unreachable!("check refuses a non-success code");
        }
        Ok(graph)
    }

    /// The node the capture `c` most recently added — i.e. the node for the operation just
    /// queued, on a linear (single-stream) capture. How a captured launch is named so its
    /// parameters can be updated in the instantiated graph without re-capturing. ★ The node is
    /// tied to the graph `c` becomes.
    ///
    /// # Errors
    /// [`CudaError`]; also refused by name when the stream is not capturing or the capture is not
    /// a single chain (more or fewer than one dependency).
    pub fn capture_leaf(&self, c: &Capture<'_>) -> Result<GraphNode, CudaError> {
        let f = Self::graph_fn(self.cuStreamGetCaptureInfo, "cuStreamGetCaptureInfo_v2")?;
        live(&c.stream.life, "cuStreamGetCaptureInfo_v2");
        let mut status: c_uint = 0;
        let mut id: u64 = 0;
        let mut g: *mut c_void = core::ptr::null_mut();
        let mut deps: *const *mut c_void = core::ptr::null();
        let mut n: usize = 0;
        // SAFETY: the stream is alive; five live out-pointers. `deps` points at driver-owned
        // storage valid until the next capture call on the stream, and is read (once) before any.
        let r = unsafe {
            f(
                c.stream.raw as *mut c_void,
                &raw mut status,
                &raw mut id,
                &raw mut g,
                &raw mut deps,
                &raw mut n,
            )
        };
        self.check("cuStreamGetCaptureInfo_v2", r)?;
        if status != CU_STREAM_CAPTURE_STATUS_ACTIVE || n != 1 || deps.is_null() {
            return Err(CudaError::Refused {
                what: "cuStreamGetCaptureInfo_v2",
                code: 0,
                name: format!(
                    "expected an ACTIVE linear capture with exactly one leaf; status={status} \
                     leaves={n}"
                ),
            });
        }
        // SAFETY: `n == 1` and `deps` is non-NULL, so `deps[0]` is a valid element.
        let node = unsafe { *deps };
        Ok(GraphNode {
            raw: node as usize,
            graph: c.graph.clone(),
        })
    }

    /// `cuGraphInstantiateWithFlags(g, 0)`.
    ///
    /// # Errors
    /// [`CudaError`].
    ///
    /// # Panics
    /// When `g` (or its context) was destroyed.
    pub fn graph_instantiate(&self, g: &GraphHandle) -> Result<GraphExecHandle, CudaError> {
        let f = Self::graph_fn(
            self.cuGraphInstantiateWithFlags,
            "cuGraphInstantiateWithFlags",
        )?;
        live(&g.life, "cuGraphInstantiateWithFlags");
        live(&g.graph, "cuGraphInstantiateWithFlags");
        let mut e: *mut c_void = core::ptr::null_mut();
        // SAFETY: `g` came from `stream_end_capture` and is alive; one live out-pointer.
        self.check("cuGraphInstantiateWithFlags", unsafe {
            f(&raw mut e, g.raw as *mut c_void, 0)
        })?;
        Ok(GraphExecHandle {
            raw: e as usize,
            life: g.life.clone(),
            graph: g.graph.clone(),
        })
    }

    /// `cuGraphLaunch(e, s)` — the whole walk, queued with ONE driver call.
    ///
    /// # Errors
    /// [`CudaError`].
    ///
    /// # Panics
    /// When a handle's context was destroyed.
    pub fn graph_launch(&self, e: &GraphExecHandle, s: &StreamHandle) -> Result<(), CudaError> {
        let f = Self::graph_fn(self.cuGraphLaunch, "cuGraphLaunch")?;
        live(&e.life, "cuGraphLaunch");
        live(&s.life, "cuGraphLaunch");
        // SAFETY: both handles came from this binding and are alive.
        self.check("cuGraphLaunch", unsafe {
            f(e.raw as *mut c_void, s.raw as *mut c_void)
        })
    }

    /// `cuGraphExecKernelNodeSetParams` — replace the launch parameters (grid, block, shared
    /// memory, every by-value argument) of kernel node `node` in `e`. Takes effect for the
    /// NEXT `cuGraphLaunch`; the driver copies `params` during the call.
    ///
    /// # Errors
    /// [`CudaError`] (e.g. `f` is not the node's function: the topology may not change).
    ///
    /// # Panics
    /// When `params` does not match `f`'s declared parameters (D-01); when `node` is not a node
    /// of the graph `e` was instantiated from, or that graph was destroyed; when a handle's
    /// context was destroyed.
    #[allow(clippy::too_many_arguments)]
    pub fn graph_exec_kernel_set(
        &self,
        e: &GraphExecHandle,
        node: &GraphNode,
        f: &Func,
        grid: u32,
        block: u32,
        shmem: u32,
        params: &mut [Vec<u8>],
        what: &'static str,
    ) -> Result<(), CudaError> {
        let set = Self::graph_fn(
            self.cuGraphExecKernelNodeSetParams,
            "cuGraphExecKernelNodeSetParams",
        )?;
        live(&e.life, what);
        assert!(
            Arc::ptr_eq(&node.graph, &e.graph),
            "{what}: the node is not a node of the graph this executable was instantiated from"
        );
        live(&node.graph, what);
        let sizes: Vec<usize> = params.iter().map(Vec::len).collect();
        f.check_params(&sizes, what);
        let mut p: Vec<*mut c_void> = params
            .iter_mut()
            .map(|b| b.as_mut_ptr().cast::<c_void>())
            .collect();
        let kp = KernelNodeParams {
            func: f.raw as *mut c_void,
            grid_x: grid,
            grid_y: 1,
            grid_z: 1,
            block_x: block,
            block_y: 1,
            block_z: 1,
            shmem,
            kernel_params: p.as_mut_ptr(),
            extra: core::ptr::null_mut(),
            kern: core::ptr::null_mut(),
            ctx: core::ptr::null_mut(),
        };
        // SAFETY: as `launch_args` — every element of `p` points into a live Vec of `params`,
        // one per declared parameter of `f` with exactly its size (`check_params`, above), and
        // the driver copies them during the call. `kp` is a live, fully-initialised
        // `CUDA_KERNEL_NODE_PARAMS_v2`; `e` is alive and `node` belongs to (and is kept alive by)
        // the graph `e` was instantiated from (both asserted above).
        self.check(what, unsafe {
            set(e.raw as *mut c_void, node.raw as *mut c_void, &raw const kp)
        })
    }

    /// `cuGraphUpload(e, s)` — move the instantiated graph's work descriptors to the device
    /// now, so the FIRST `cuGraphLaunch` does not pay for it on the submitting thread.
    ///
    /// # Errors
    /// [`CudaError`].
    ///
    /// # Panics
    /// When a handle's context was destroyed.
    pub fn graph_upload(&self, e: &GraphExecHandle, s: &StreamHandle) -> Result<(), CudaError> {
        let f = Self::graph_fn(self.cuGraphUpload, "cuGraphUpload")?;
        live(&e.life, "cuGraphUpload");
        live(&s.life, "cuGraphUpload");
        // SAFETY: both handles came from this binding and are alive.
        self.check("cuGraphUpload", unsafe {
            f(e.raw as *mut c_void, s.raw as *mut c_void)
        })
    }

    /// `cuEventRecordWithFlags(e, s, CU_EVENT_RECORD_EXTERNAL)` — under stream capture this
    /// becomes an EVENT-RECORD NODE of the graph (a plain `cuEventRecord` during capture only
    /// expresses a dependency and records nothing), so every replay records `e` on the GPU.
    ///
    /// # Errors
    /// [`CudaError`].
    ///
    /// # Panics
    /// When a handle's context was destroyed.
    pub fn event_record_external(
        &self,
        e: &EventHandle,
        s: &StreamHandle,
    ) -> Result<(), CudaError> {
        let f = Self::graph_fn(self.cuEventRecordWithFlags, "cuEventRecordWithFlags")?;
        live(&e.life, "cuEventRecordWithFlags");
        live(&s.life, "cuEventRecordWithFlags");
        // SAFETY: both handles came from this binding and are alive; `1` is
        // `CU_EVENT_RECORD_EXTERNAL`.
        self.check("cuEventRecordWithFlags", unsafe {
            f(e.raw as *mut c_void, s.raw as *mut c_void, 1)
        })
    }

    /// `cuGraphExecDestroy`. ⊘ Infallible: it runs in a `Drop`. ★ Consumes the handle.
    pub fn graph_exec_destroy(&self, e: GraphExecHandle) {
        if let (Some(f), true) = (self.cuGraphExecDestroy, e.raw != 0 && e.life.alive()) {
            // SAFETY: `e` came from `graph_instantiate` in a live context and is consumed here.
            unsafe { f(e.raw as *mut c_void) };
        }
    }

    /// `cuGraphDestroy`. ⊘ Infallible: it runs in a `Drop`. ★ Consumes the handle and ends the
    /// graph's token: its nodes refuse from now on.
    pub fn graph_destroy(&self, g: GraphHandle) {
        g.graph.end();
        if let (Some(f), true) = (self.cuGraphDestroy, g.raw != 0 && g.life.alive()) {
            // SAFETY: `g` came from `stream_end_capture` in a live context and is consumed here.
            unsafe { f(g.raw as *mut c_void) };
        }
    }

    /// As [`Cuda::launch_raw`], checked.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    ///
    /// # Panics
    /// As [`Cuda::launch_raw`].
    pub fn launch(
        &self,
        f: &Func,
        grid: u32,
        block: u32,
        param: &mut [u8],
        what: &'static str,
    ) -> Result<(), CudaError> {
        self.check(what, self.launch_raw(f, grid, block, param))
    }
}

/// ★ A context that is destroyed on drop unless [`CtxGuard::disarm`]ed
/// ([`Cuda::ctx_create_guarded`], D-15).
pub(crate) struct CtxGuard<'a> {
    cu: &'a Cuda,
    ctx: Option<CtxHandle>,
}

impl CtxGuard<'_> {
    /// Hand the context over: it is no longer destroyed when the guard goes.
    pub(crate) fn disarm(mut self) -> CtxHandle {
        // `ctx` is `Some` from construction until here, the only place that takes it
        self.ctx.take().unwrap_or_else(CtxHandle::null)
    }
}

impl Drop for CtxGuard<'_> {
    fn drop(&mut self) {
        if let Some(ctx) = self.ctx.take() {
            self.cu.ctx_destroy(ctx);
        }
    }
}

/// An opaque CUDA context handle. ⊘ A `usize` and not a pointer, so nothing outside this file
/// can dereference it and `WalkKernel` can stay `Send`. ★ Not `Copy`/`Clone` (D-04):
/// [`Cuda::ctx_destroy`] consumes it.
pub struct CtxHandle {
    raw: usize,
    life: Arc<Life>,
}

impl core::fmt::Debug for CtxHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CtxHandle")
            .field("alive", &self.life.alive())
            .finish()
    }
}

impl CtxHandle {
    /// Whether this handle names no live context (the null handle, or a destroyed one).
    #[must_use]
    pub fn is_null(&self) -> bool {
        !self.life.alive()
    }
    /// The null handle: names nothing, destroys nothing, and cannot be made current.
    #[must_use]
    pub fn null() -> Self {
        CtxHandle {
            raw: 0,
            life: Life::dead(),
        }
    }
}

/// An opaque CUDA stream handle. ★ Not `Copy`/`Clone`; refuses once its context is destroyed.
#[derive(Debug)]
pub struct StreamHandle {
    raw: usize,
    life: Arc<Life>,
}

/// An opaque CUDA event handle. ★ Not `Copy`/`Clone`; refuses once its context is destroyed.
#[derive(Debug)]
pub struct EventHandle {
    raw: usize,
    life: Arc<Life>,
}

impl StreamHandle {
    /// A handle naming no stream (left behind when an owner moves its real stream out to destroy
    /// it). Every use panics; destroying it does nothing.
    #[must_use]
    pub fn null() -> Self {
        StreamHandle {
            raw: 0,
            life: Life::dead(),
        }
    }
}

impl EventHandle {
    /// A handle naming no event (see [`StreamHandle::null`]).
    #[must_use]
    pub fn null() -> Self {
        EventHandle {
            raw: 0,
            life: Life::dead(),
        }
    }
}

/// ★★★ D-02 (2026-11-13, S1-04): what a device allocation is to safe code — its address AND its
/// length, minted only in this file. Not `Copy`/`Clone`: [`Cuda::mem_free`] consumes it. Safe code
/// reaches the bytes only through a [`DevSpan`], whose range is checked against this length.
///
/// ⊘ Not freed on drop (freeing needs the [`Cuda`] that allocated it): a buffer dropped without
/// [`Cuda::mem_free`] is reclaimed by `cuCtxDestroy`, like every allocation of its context.
pub struct DevBuf {
    ptr: CUdeviceptr,
    len: usize,
    kind: DevKind,
    /// The context it was allocated in.
    life: Arc<Life>,
    /// For a pinned view: the pinned buffer's own token (ended when that buffer is released).
    owner: Option<Arc<Life>>,
}

/// Where a [`DevBuf`] came from, and so whether [`Cuda::mem_free`] may hand it to `cuMemFree`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevKind {
    /// `cuMemAlloc`: freed by `cuMemFree`.
    Owned,
    /// An imported RM object, mapped (`import_and_map`): reclaimed with the context.
    Imported,
    /// A device view of page-locked host memory: the host buffer's, released with it.
    PinnedView,
}

impl core::fmt::Debug for DevBuf {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // ⊘ The address is not printed: it is not a value safe code needs to see.
        f.debug_struct("DevBuf")
            .field("len", &self.len)
            .field("kind", &self.kind)
            .finish()
    }
}

impl DevBuf {
    /// Its length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether it holds no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// What kind of allocation it is.
    #[must_use]
    pub fn kind(&self) -> DevKind {
        self.kind
    }

    /// `[off, off + len)` of this allocation, or `None` when that leaves it (checked arithmetic).
    #[must_use]
    pub fn try_span(&self, off: u64, len: usize) -> Option<DevSpan<'_>> {
        let end = off.checked_add(len as u64)?;
        if end > self.len as u64 {
            return None;
        }
        Some(DevSpan {
            ptr: self.ptr.checked_add(off)?,
            len,
            buf: self,
        })
    }

    /// `[off, off + len)` of this allocation.
    ///
    /// # Panics
    /// When the range leaves the allocation (checked arithmetic) — a bug in the caller, refused
    /// before any driver call can be made with it.
    #[must_use]
    #[track_caller]
    pub fn span(&self, off: u64, len: usize) -> DevSpan<'_> {
        self.try_span(off, len).unwrap_or_else(|| {
            panic!(
                "DevBuf::span: [{off:#x}, +{len:#x}) leaves the {:#x}-byte allocation",
                self.len
            )
        })
    }

    /// The whole allocation.
    #[must_use]
    pub fn whole(&self) -> DevSpan<'_> {
        DevSpan {
            ptr: self.ptr,
            len: self.len,
            buf: self,
        }
    }

    /// Whether this buffer (and, for a pinned view, its host buffer) is still alive.
    fn alive(&self) -> bool {
        self.life.alive() && self.owner.as_ref().is_none_or(|o| o.alive())
    }
}

/// ★ A checked range of a [`DevBuf`], borrowed from it (so the allocation cannot be freed while a
/// span exists). The only way safe code names device bytes to a copy or a set.
#[derive(Clone, Copy)]
pub struct DevSpan<'a> {
    ptr: CUdeviceptr,
    len: usize,
    buf: &'a DevBuf,
}

impl core::fmt::Debug for DevSpan<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DevSpan").field("len", &self.len).finish()
    }
}

impl<'a> DevSpan<'a> {
    /// Its length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether it holds no bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `[off, off + len)` of this span.
    ///
    /// # Panics
    /// When that leaves the span (checked arithmetic).
    #[must_use]
    #[track_caller]
    pub fn sub(&self, off: u64, len: usize) -> DevSpan<'a> {
        let ok = off
            .checked_add(len as u64)
            .is_some_and(|e| e <= self.len as u64);
        assert!(
            ok,
            "DevSpan::sub: [{off:#x}, +{len:#x}) leaves the {:#x}-byte span",
            self.len
        );
        DevSpan {
            ptr: self.ptr + off,
            len,
            buf: self.buf,
        }
    }

    /// ★ The span's first address as a KERNEL PARAMETER value (the 8 bytes a `T*` parameter or a
    /// `KfArgs` pointer field holds).
    ///
    /// CALLER-CONTRACT: the kernel this value is passed to bounds its own accesses by the length
    /// passed beside it (or a fixed size its source states). Not validated here: a kernel
    /// parameter is opaque bytes to the driver; typed per-kernel launch builders inside this file
    /// would be the full fix (the `v3-sec-rawaddr` design), and are not built yet. Otherwise: a
    /// kernel writes past the span.
    #[must_use]
    pub fn kernel_addr(&self) -> u64 {
        self.ptr
    }

    /// [`Self::kernel_addr`] as the 8 little-endian bytes of one kernel parameter.
    #[must_use]
    pub fn kernel_arg(&self) -> Vec<u8> {
        self.ptr.to_le_bytes().to_vec()
    }

    /// Panic unless `n <= len` and the allocation is alive. Every copy and set calls it.
    #[track_caller]
    fn check(&self, n: usize, what: &str) {
        assert!(
            n <= self.len,
            "{what}: {n:#x} bytes do not fit the {:#x}-byte device span",
            self.len
        );
        assert!(
            self.buf.alive(),
            "{what}: the device allocation was released (or its context destroyed); refused \
             before the driver saw it"
        );
    }
}

/// ★ Page-locked host memory the walk's async copies land in. `pub(crate)` on purpose: its
/// bytes are written by DMA **after** the call that queued the copy returns, so reading them
/// means something only once the walk's completion was observed — a state machine
/// `WalkKernel` owns and nothing outside this crate can be trusted to follow. Reclaimed by
/// `cuCtxDestroy`, like every other allocation of the walker's context.
#[derive(Debug)]
pub(crate) struct PinnedBuf {
    ptr: usize,
    len: usize,
    /// ★ 2026-11-13: the context it lives in. `cuCtxDestroy` reclaims the memory, so every
    /// access refuses (panics) once that token ended (D-05(a): a `display::Frame` read after its
    /// `DisplayGpu` dropped used to copy from reclaimed memory).
    ctx: Arc<Life>,
    /// Its own token, ended by `pinned_free` / `host_unregister` (device views check it).
    own: Arc<Life>,
    /// ★ 2026-11-14 (D-05(b)): what the GPU may still write here — the fence after the last queued
    /// copy, and whether a device view was ever handed out. Consulted before the pages go back.
    dma: PinnedDma,
}

/// ★★ D-05(b) (security review 2, 2026-11-14): the GPU writes still pending on a [`PinnedBuf`].
/// [`Cuda::memcpy_d2h_async`] records an event after every copy it queues into the buffer; giving
/// the buffer back ([`Cuda::pinned_free`], [`Cuda::host_unregister`]) first asks that event, and
/// REFUSES — keeping the pages until the context dies — while a copy may still land. A buffer that
/// lent a device view to a kernel ([`Cuda::pinned_device_view`]) is never given back alone: the
/// binding cannot see which queued launch names a view, so only `cuCtxDestroy` reclaims it.
#[derive(Debug, Default)]
struct PinnedDma {
    fence: std::sync::Mutex<Fence>,
    viewed: core::sync::atomic::AtomicBool,
}

/// The last copy queued into a pinned buffer.
#[derive(Debug, Default)]
enum Fence {
    /// No copy was ever queued, or the last one was observed complete.
    #[default]
    Clear,
    /// An event recorded on the copy's stream right after it.
    After(EventHandle),
    /// A copy was queued but its event could not be recorded: completion is unknowable.
    Unknown,
}

impl PinnedBuf {
    /// A placeholder naming nothing (`len` 0), left behind when a frame's real buffer is moved
    /// out to be released. Never handed to a CUDA call.
    pub(crate) fn empty() -> PinnedBuf {
        PinnedBuf {
            ptr: 0,
            len: 0,
            ctx: Life::dead(),
            own: Life::dead(),
            dma: PinnedDma::default(),
        }
    }

    /// The allocation's host address, as an integer — handed across an FFI (the display console
    /// reads a finished frame there); never dereferenced by safe code.
    pub(crate) fn addr(&self) -> usize {
        self.ptr
    }

    /// Its length.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Copy `n` bytes at `off` out. ⊘ Only after the copy that filled them completed.
    ///
    /// # Panics
    /// If the range leaves the buffer.
    pub(crate) fn read(&self, off: usize, n: usize) -> Vec<u8> {
        assert!(
            off.checked_add(n).is_some_and(|e| e <= self.len),
            "pinned read leaves the buffer"
        );
        live(&self.ctx, "PinnedBuf::read");
        live(&self.own, "PinnedBuf::read");
        let mut out = vec![0u8; n];
        // SAFETY: the range is inside the pinned allocation (asserted) and the allocation is
        // alive (its context and its own token, asserted); `out` is a fresh local of exactly `n`
        // bytes, so the regions cannot overlap. A raw-pointer copy: no reference is formed over
        // memory a DMA may write (a racing DMA tears the copy, it is not UB of a Rust reference).
        unsafe {
            core::ptr::copy_nonoverlapping((self.ptr + off) as *const u8, out.as_mut_ptr(), n)
        };
        out
    }

    /// Copy `bytes` in at `off`. ⊘ Only while no queued copy reads the range.
    ///
    /// # Panics
    /// If the range leaves the buffer.
    pub(crate) fn write(&mut self, off: usize, bytes: &[u8]) {
        assert!(
            off.checked_add(bytes.len()).is_some_and(|e| e <= self.len),
            "pinned write leaves the buffer"
        );
        live(&self.ctx, "PinnedBuf::write");
        live(&self.own, "PinnedBuf::write");
        // SAFETY: the range is inside the pinned allocation (asserted), which is alive
        // (asserted), and `bytes` is a distinct Rust slice; a raw-pointer copy, so a kernel
        // reading the range concurrently sees torn bytes, never Rust UB.
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                (self.ptr + off) as *mut u8,
                bytes.len(),
            );
        }
    }
}

/// ★★★ **THE WALK'S COMPLETION FD** — an `eventfd(EFD_NONBLOCK|EFD_CLOEXEC)` a CUDA host
/// function writes when a walk's last copy has landed. Put [`CompletionFd::raw`] in the
/// worker's `epoll` set; readiness is the wake and [`CompletionFd::drain`] consumes it.
#[derive(Debug)]
pub struct CompletionFd {
    /// ★ D-07: shared with every queued host signal (`Cuda::launch_host_signal`), so the fd is
    /// closed only once the last of them has run.
    fd: Arc<std::os::fd::OwnedFd>,
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
                what: "eventfd (the walk's completion fd)",
                code: fd,
                name: "eventfd(2) refused".to_string(),
            });
        }
        // SAFETY: `fd` was just returned by `eventfd`, is open, and is owned by nothing else.
        let fd = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
        Ok(CompletionFd { fd: Arc::new(fd) })
    }

    /// The fd number. Borrowed: this value owns it.
    #[must_use]
    pub fn raw(&self) -> i32 {
        std::os::fd::AsRawFd::as_raw_fd(&self.fd)
    }

    /// Signal it once, as the walk's host function does. For a caller that multiplexes other
    /// work onto the same kind of fd (a harness's request queue); a walk never needs it.
    pub fn signal(&self) {
        signal_fd(&self.fd);
    }

    /// Consume every pending signal; `0` when none was pending. **Never blocks.**
    #[must_use]
    pub fn drain(&self) -> u64 {
        let mut v: u64 = 0;
        // SAFETY: `v` is a live 8-byte buffer; the fd is non-blocking, so an empty counter
        // returns -1/EAGAIN at once rather than waiting.
        let n = unsafe { read(self.raw(), (&raw mut v).cast::<c_void>(), 8) };
        if n == 8 { v } else { 0 }
    }

    /// `poll(2)` for readability, up to `timeout_ms`; whether it became readable.
    /// ⚠ This **blocks the calling thread**. It exists for harnesses and the selftest — never
    /// for a worker, whose only wait is its `epoll` (§35).
    #[must_use]
    pub fn wait_readable(&self, timeout_ms: i32) -> bool {
        let mut p = PollFd {
            fd: self.raw(),
            events: POLLIN,
            revents: 0,
        };
        // SAFETY: one live `pollfd`, count 1.
        let r = unsafe { poll(&raw mut p, 1, timeout_ms) };
        r > 0 && (p.revents & POLLIN) != 0
    }
}

impl std::os::fd::AsFd for CompletionFd {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// `write(fd, 1)` on an eventfd this process owns.
fn signal_fd(fd: &std::os::fd::OwnedFd) {
    let one: u64 = 1;
    // SAFETY: `one` is a live 8-byte value and `fd` is an open descriptor (borrowed, so it
    // cannot close during the call); `write` touches nothing else.
    let _ = unsafe {
        write(
            std::os::fd::AsRawFd::as_raw_fd(fd),
            (&raw const one).cast::<c_void>(),
            8,
        )
    };
}

/// ★ The host function every walk's stream ends with: `write(fd, 1)`. ★ D-07: `user` is one strong
/// `Arc<OwnedFd>` reference made by [`Cuda::launch_host_signal`], taken back here, so the fd is
/// open while this runs whatever the `CompletionFd`'s owner did meanwhile. It runs on a CUDA driver
/// thread and makes no CUDA call (the driver forbids that).
///
/// ⊘ Private: only `launch_host_signal` may hand it to the driver, with the datum it requires.
extern "C" fn completion_hostfn(user: *mut c_void) {
    // SAFETY: the driver calls this exactly once per `cuLaunchHostFunc` that succeeded, with the
    // datum `launch_host_signal` passed: one `Arc::into_raw` reference, consumed here once.
    let fd = unsafe { Arc::from_raw(user.cast_const().cast::<std::os::fd::OwnedFd>()) };
    signal_fd(&fd);
}

#[cfg(test)]
mod completion_fd_tests {
    use super::*;

    /// ★ The completion path WITHOUT a GPU: the exact function the driver will call, called
    /// here, must make the fd readable and drain to exactly one signal.
    #[test]
    fn the_host_function_signals_the_fd_and_drain_consumes_it() {
        let fd = CompletionFd::new().expect("eventfd");
        assert_eq!(fd.drain(), 0, "a fresh fd has nothing pending");
        assert!(!fd.wait_readable(0), "and is not readable");
        completion_hostfn(Arc::into_raw(fd.fd.clone()).cast_mut().cast::<c_void>());
        assert!(
            fd.wait_readable(0),
            "the host function must make the fd readable"
        );
        assert_eq!(fd.drain(), 1, "exactly one signal per walk");
        assert_eq!(
            fd.drain(),
            0,
            "drain consumed it — a stale count would complete the NEXT walk early"
        );
    }
}

/// An opaque CUDA module handle, with the parameter lists of its PTX's entries (D-01).
/// ★ Not `Copy`/`Clone`; refuses once its context is destroyed.
#[derive(Debug)]
pub struct ModuleHandle {
    raw: usize,
    life: Arc<Life>,
    entries: Vec<PtxEntry>,
}

/// An opaque CUDA kernel-function handle, carrying its entry's name and the byte size of each of
/// its parameters as the module's PTX declares them (D-01). `Clone` (a function is never destroyed
/// on its own: it lives as long as its module, i.e. its context, whose token it carries).
#[derive(Debug, Clone)]
pub struct Func {
    raw: usize,
    life: Arc<Life>,
    name: &'static str,
    params: Arc<[usize]>,
}

impl PartialEq for Func {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw && self.name == other.name && Arc::ptr_eq(&self.life, &other.life)
    }
}
impl Eq for Func {}

impl Func {
    /// The entry's name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The byte size of each declared parameter, in order.
    #[must_use]
    pub fn param_sizes(&self) -> &[usize] {
        &self.params
    }

    /// ★★★ D-01: panic unless `sizes` is exactly this entry's parameter list (count, then every
    /// size) and the context is alive. The driver takes both from the kernel's signature, not
    /// from our array, so a short list or a short buffer is a read past host memory.
    #[track_caller]
    fn check_params(&self, sizes: &[usize], what: &str) {
        live(&self.life, what);
        assert_eq!(
            sizes.len(),
            self.params.len(),
            "{what}: `{}` declares {} parameters and {} were passed",
            self.name,
            self.params.len(),
            sizes.len()
        );
        for (i, (got, want)) in sizes.iter().zip(self.params.iter()).enumerate() {
            assert_eq!(
                got, want,
                "{what}: `{}` parameter {i} is {want} bytes and {got} were passed",
                self.name
            );
        }
    }
}

/// ★ A stream capture in progress ([`Cuda::stream_begin_capture`]): borrows the stream, and
/// names the graph it will become so its nodes ([`Cuda::capture_leaf`]) are tied to that graph.
#[derive(Debug)]
pub struct Capture<'s> {
    stream: &'s StreamHandle,
    graph: Arc<Life>,
}

/// An opaque CUDA graph (the captured, un-instantiated walk). ★ Not `Copy`/`Clone`;
/// [`Cuda::graph_destroy`] consumes it and ends the token its nodes check.
#[derive(Debug)]
pub struct GraphHandle {
    raw: usize,
    life: Arc<Life>,
    graph: Arc<Life>,
}

/// An opaque instantiated CUDA graph — what `cuGraphLaunch` submits. ★ Not `Copy`/`Clone`;
/// remembers which graph it was instantiated from, so a node of another graph is refused.
#[derive(Debug)]
pub struct GraphExecHandle {
    raw: usize,
    life: Arc<Life>,
    graph: Arc<Life>,
}

/// An opaque node of a [`GraphHandle`]; refuses once that graph is destroyed. Not `Copy`/`Clone`.
#[derive(Debug)]
pub struct GraphNode {
    raw: usize,
    graph: Arc<Life>,
}

/// ★★★ D-01: one `.entry` of a PTX image — its name and the byte size of each parameter, in
/// declaration order, as the PTX text states them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtxEntry {
    /// The entry's (possibly mangled) name.
    pub name: String,
    /// Byte size of each parameter.
    pub params: Vec<usize>,
}

/// ★★★ D-01: the `.entry` parameter lists of a PTX image (text, without the NUL).
///
/// Every parameter declaration must have the shape `.param [.align N] [.ptr] [state space]
/// <.type> name[N]?`; anything else refuses the WHOLE image, by name, rather than guess a size.
/// A guessed size would make the launch check itself the hole.
///
/// # Errors
/// Which entry and which declaration did not parse; a repeated entry name; non-UTF-8 text.
pub fn ptx_entries(ptx: &[u8]) -> Result<Vec<PtxEntry>, String> {
    let text = core::str::from_utf8(ptx).map_err(|e| format!("the PTX is not UTF-8: {e}"))?;
    // strip `//` comments (PTX has no string literals, so `//` always starts one) and `/* */`
    let mut clean = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let line = rest.find("//");
        let block = rest.find("/*");
        match (line, block) {
            (Some(l), b) if b.is_none_or(|b| l < b) => {
                clean.push_str(&rest[..l]);
                rest = rest[l..].find('\n').map_or("", |n| &rest[l + n..]);
            }
            (_, Some(b)) => {
                clean.push_str(&rest[..b]);
                let end = rest[b + 2..]
                    .find("*/")
                    .ok_or_else(|| "an unterminated /* comment in the PTX".to_string())?;
                clean.push(' ');
                rest = &rest[b + 2 + end + 2..];
            }
            _ => {
                clean.push_str(rest);
                rest = "";
            }
        }
    }
    let mut out: Vec<PtxEntry> = Vec::new();
    let mut at = 0;
    while let Some(i) = clean[at..].find(".entry") {
        let start = at + i;
        at = start + ".entry".len();
        let before_ok = start == 0 || clean[..start].ends_with(char::is_whitespace);
        if !before_ok || !clean[at..].starts_with(char::is_whitespace) {
            continue;
        }
        let open = clean[at..]
            .find('(')
            .ok_or_else(|| "an .entry with no parameter list".to_string())?;
        let name = clean[at..at + open].trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.'))
        {
            return Err(format!("an .entry name that does not parse: {name:?}"));
        }
        let body_start = at + open + 1;
        let close = clean[body_start..]
            .find(')')
            .ok_or_else(|| format!(".entry {name}: no closing parenthesis"))?;
        let body = &clean[body_start..body_start + close];
        at = body_start + close + 1;
        let mut params = Vec::new();
        if !body.trim().is_empty() {
            for decl in body.split(',') {
                params.push(
                    ptx_param_size(decl)
                        .map_err(|e| format!(".entry {name}: {e} in {:?}", decl.trim()))?,
                );
            }
        }
        if out.iter().any(|e| e.name == name) {
            return Err(format!(".entry {name} is declared twice"));
        }
        out.push(PtxEntry {
            name: name.to_string(),
            params,
        });
    }
    Ok(out)
}

/// The byte size of one `.param` declaration (see [`ptx_entries`]).
fn ptx_param_size(decl: &str) -> Result<usize, String> {
    let toks: Vec<&str> = decl.split_whitespace().collect();
    if toks.first() != Some(&".param") || toks.len() < 3 {
        return Err("not a `.param <type> <name>` declaration".to_string());
    }
    let mut elem: Option<usize> = None;
    let mut i = 1;
    while i + 1 < toks.len() {
        match toks[i] {
            ".align" => {
                toks[i + 1]
                    .parse::<u32>()
                    .map_err(|_| "an .align without a number".to_string())?;
                i += 2;
                continue;
            }
            ".ptr" | ".global" | ".const" | ".local" | ".shared" => {}
            t => {
                let size = match t {
                    ".b8" | ".u8" | ".s8" => 1,
                    ".b16" | ".u16" | ".s16" | ".f16" => 2,
                    ".b32" | ".u32" | ".s32" | ".f32" => 4,
                    ".b64" | ".u64" | ".s64" | ".f64" => 8,
                    _ => return Err(format!("an unknown token {t:?}")),
                };
                if elem.replace(size).is_some() {
                    return Err("two types".to_string());
                }
            }
        }
        i += 1;
    }
    let elem = elem.ok_or_else(|| "no type".to_string())?;
    if i != toks.len() - 1 {
        return Err("a declaration that does not end in its name".to_string());
    }
    let name = toks[i];
    let count = match name.find('[') {
        None => 1,
        Some(b) => {
            let n = name[b + 1..]
                .strip_suffix(']')
                .ok_or_else(|| "a malformed array suffix".to_string())?;
            n.parse::<usize>()
                .map_err(|_| "a non-numeric array length".to_string())?
        }
    };
    let base = name.split('[').next().unwrap_or("");
    if base.is_empty() || base.starts_with('.') || count == 0 {
        return Err("a missing or empty parameter".to_string());
    }
    elem.checked_mul(count)
        .ok_or_else(|| "an array size that overflows".to_string())
}

/// `CUDA_KERNEL_NODE_PARAMS_v2` (`cuda.h`, CUDA 12). ⊘ The v1 struct is its first seven fields;
/// `kern`/`ctx` are NULL here, which the driver reads as "use `func`" / "the current context".
#[repr(C)]
pub(crate) struct KernelNodeParams {
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

/// `CU_STREAM_CAPTURE_MODE_THREAD_LOCAL`: an unsafe API call on THIS thread during the capture
/// is an error (so a stray synchronous call cannot be silently left out of the graph), while
/// other threads of the isolate are not constrained.
const CU_STREAM_CAPTURE_MODE_THREAD_LOCAL: c_uint = 1;
/// `CU_STREAM_CAPTURE_STATUS_ACTIVE`.
const CU_STREAM_CAPTURE_STATUS_ACTIVE: c_uint = 1;

/// ★★★ D-08 (2026-11-13): the types whose all-zero value this crate may build ([`zeroed`]) and
/// whose bytes it may serialise ([`abi_bytes`]). SEALED: implemented only in this file — for the
/// primitive integers, arrays of such fields, and (by [`abi_struct!`]) the `#[repr(C)]` ABI
/// aggregates of `abi.rs`, whose every field is itself an `AbiField`. So all-zero is a valid value
/// of every implementor (no reference, no `NonZero`, no enum niche, no pointer Rust tracks).
///
/// Before this, `zeroed` and `view_bytes` were generic over any `T: Copy`: `view_bytes` read
/// padding (uninitialised bytes: `KfFormat` has two after `small_ps`, MEASURED by the tiling check
/// below) and `zeroed::<&T>()` relied on Rust's runtime abort. `view_bytes` is gone: bytes are
/// produced FIELD BY FIELD by [`abi_bytes`], padding written as zero, with no `unsafe` at all.
///
/// # Safety
/// Implemented only in this file, for types whose all-zero bit pattern is valid; sealed so no
/// other crate can implement it.
pub unsafe trait AbiField: Copy + sealed::Sealed {
    /// Write this value's bytes (native byte order, the layout the compiler gives it) into
    /// `out`, which is exactly `size_of::<Self>()` long; padding is left as it is (zero).
    fn encode_into(&self, out: &mut [u8]);
}

mod sealed {
    /// The seal of [`super::AbiField`].
    pub trait Sealed {}
}

macro_rules! abi_int {
    ($($t:ty),*) => {$(
        impl sealed::Sealed for $t {}
        // SAFETY: a primitive integer: every bit pattern, all-zero included, is valid.
        unsafe impl AbiField for $t {
            fn encode_into(&self, out: &mut [u8]) {
                out.copy_from_slice(&self.to_ne_bytes());
            }
        }
    )*};
}
abi_int!(u8, u16, u32, u64, i8, i16, i32, i64);
impl<T: AbiField, const N: usize> sealed::Sealed for [T; N] {}
// SAFETY: an array of all-zero-valid elements is all-zero valid.
unsafe impl<T: AbiField, const N: usize> AbiField for [T; N] {
    fn encode_into(&self, out: &mut [u8]) {
        // arrays never pad between elements: element i is at i * size_of::<T>()
        for (v, o) in self
            .iter()
            .zip(out.chunks_exact_mut(core::mem::size_of::<T>()))
        {
            v.encode_into(o);
        }
    }
}

/// Panic at compile time unless the `(offset, size)` pairs are in order, do not overlap, and end
/// inside `total` (gaps are padding, written as zero by `encode_into`).
const fn abi_in_order(total: usize, fields: &[(usize, usize)]) {
    let mut at = 0;
    let mut i = 0;
    while i < fields.len() {
        assert!(fields[i].0 >= at, "fields out of order, or overlapping");
        at = fields[i].0 + fields[i].1;
        i += 1;
    }
    assert!(at <= total, "a field past the end of the struct");
}

/// Implement [`AbiField`] for a `#[repr(C)]` aggregate of `abi.rs`, listing EVERY field in
/// declaration order with its type. A field type that is not an `AbiField` does not compile; an
/// out-of-order or overlapping list fails `abi_in_order`; a MISSING field is caught by the test
/// `abi_struct_lists_every_field` (it compares each list with the struct in `abi.rs`).
macro_rules! abi_struct {
    ($($t:ident { $($f:ident : $ft:ty),* $(,)? })*) => {$(
        impl sealed::Sealed for $t {}
        // SAFETY: every field is an `AbiField` (all-zero valid), so the aggregate is too; its
        // padding has no validity requirement.
        unsafe impl AbiField for $t {
            fn encode_into(&self, out: &mut [u8]) {
                $(
                    let at = core::mem::offset_of!($t, $f);
                    self.$f.encode_into(&mut out[at..at + core::mem::size_of::<$ft>()]);
                )*
            }
        }
        const _: () = abi_in_order(
            core::mem::size_of::<$t>(),
            &[$((core::mem::offset_of!($t, $f), core::mem::size_of::<$ft>())),*],
        );
    )*
        /// Every `abi_struct!` list, by struct name (the completeness test reads it).
        #[cfg(test)]
        pub(crate) const ABI_STRUCT_FIELDS: &[(&str, &[&str])] =
            &[$((stringify!($t), &[$(stringify!($f)),*])),*];
    };
}

use crate::abi::{
    KF_DIRS, KF_MAX_PDB, KF_MAX_RESET, KF_MAX_SLOTS, KF_REFUSAL_SAMPLES, KfAck, KfArgs, KfDev,
    KfDir, KfField, KfFormat, KfLayout, KfMapRun, KfPdbEntry, KfRefusalSample, KfReportHeader,
    KfScope, KfSlot, KfWin,
};

abi_struct! {
    KfField { lo: u8, bits: u8, shift: u8, pad: u8 }
    KfDir { active: u8, va_lo: u8, entry_bytes: u8, leaf_ps: u8, entries: u16, pad: u16 }
    KfFormat { abi_version: u32, table_version: u32, dir: [KfDir; KF_DIRS],
        big_va_lo: u8, small_va_lo: u8, big_entry_bytes: u8, small_entry_bytes: u8,
        big_entries: u16, small_entries: u16, big_ps: u8, small_ps: u8, root_align: u32,
        first_dir: u8, pad0: [u8; 3], valid_bit: u8, ap_lo: u8, ap_bits: u8, pde_ap_invalid: u8,
        pte_ap_map: [u8; 4], pde_ap_map: [u8; 4], addr_sel: [u8; 4], addr_local: KfField,
        addr_sys: KfField, big_addr_local: KfField, big_addr_sys: KfField, bit_volatile: u8,
        bit_privilege: u8, bit_read_only: u8, bit_atomic_disable: u8, pcf: KfField,
        pcf_sparse: u8, pad1: [u8; 3], kind: KfField, ps_log2: [u8; 4] }
    KfWin { base: u64, len: u64, span: u64 }
    KfRefusalSample { va: u64, raw: u64, gpga: u64, ps_bytes: u64, bit: u32, level: u16, entry: u16 }
    KfDev { generation: u64, committed: u64, runs_per_pdb: u32, entry_budget: u32,
        run_capacity: u32, pdb_capacity: u32, max_pdbs: u32, max_slots: u32,
        tbl_run_count: [u32; KF_MAX_PDB], diff_count: [u32; KF_MAX_PDB],
        diff_vflags: [u32; KF_MAX_PDB], entry_refuse: [u32; KF_MAX_PDB], entries_visited: u64,
        refusals: u32, refuse_mask: u32, hdr_flags: u32, walk_trunc: u32, walk_abort: u32,
        sparse_slots: u32, need: [u32; KF_MAX_PDB], nsample: u32, sample_pad: u32,
        sample: [KfRefusalSample; KF_REFUSAL_SAMPLES] }
    KfSlot { n: [u32; 4] }
    KfLayout { walk_off: [u32; KF_MAX_PDB], walk_cap: [u32; KF_MAX_PDB],
        prev_off: [u32; KF_MAX_PDB], prev_cap: [u32; KF_MAX_PDB],
        slot_off: [u32; KF_MAX_SLOTS], slot_cap: [u32; KF_MAX_SLOTS] }
    KfAck { generation: u64, nrun: u32, nreset: u32, reset: [u32; KF_MAX_RESET] }
    KfArgs { win: KfWin, fmt: KfFormat, dev: u64, walk: u64, com: u64, slot: u64, pdbs: u64,
        slots: u64, npdb: u32, key_perm: u32, ack: u64, ack_code: u64, scratch: u64,
        iscratch: u64, hdr: u64, rpdb: u64, rrun: u64, lay: u64 }
    KfReportHeader { magic: u32, version: u16, flags: u16, generation: u64,
        acked_generation: u64, pdb_count: u32, pdb_capacity: u32, run_count: u32,
        run_capacity: u32, entries_visited: u64, refusals: u32, refuse_mask: u32,
        sparse_slots: u32, ps_log2: [u8; 4], sample_count: u32, sample_total: u32,
        samples: [KfRefusalSample; KF_REFUSAL_SAMPLES] }
    KfPdbEntry { pdb: u64, first_run: u32, run_count: u32, vas_flags: u32, reserved: u32,
        reserved2: u64 }
    KfMapRun { va: u64, gpga: u64, len: u64, flags: u32, op: u16, pdb_index: u16 }
    KfScope { pdb: u64, va_base: u64, va_len: u64 }
}

/// ★ **The bytes of one ABI value** — what `cuLaunchKernel` and `cuMemcpyHtoD_v2` are handed for
/// a `#[repr(C)]` struct: every field at its compiler offset, native byte order, padding ZERO.
/// Safe code, no `unsafe`: it replaces `view_bytes`, which formed a `&[u8]` over the value and so
/// read its padding.
#[must_use]
pub fn abi_bytes<T: AbiField>(v: &T) -> Vec<u8> {
    let mut out = vec![0u8; core::mem::size_of::<T>()];
    v.encode_into(&mut out);
    out
}

// ⊘ `read_struct` (a raw `copy_nonoverlapping` of device bytes into any `Copy` type) was removed
// 2026-09-27: its only callers decoded the walk report, which now has ONE safe decoder per
// struct (`abi.rs`, `report_codec!`, by `offset_of!`). One fewer unsafe block in the audited file.
// ⊘ `view_bytes` was removed 2026-11-13 for the same reason, replaced by `abi_bytes`.

/// ★★★★★ **AN ALL-ZERO VALUE** of an [`AbiField`] type.
///
/// # ⊘⊘⊘ WHY THIS EXISTS, AND WHAT IT CAUGHT
///
/// `KfFormat` has interior padding (two bytes between `small_ps` and `root_align`). A Rust struct
/// literal leaves padding **undefined** — so the 216 bytes `view_bytes` handed `cuLaunchKernel`
/// contained whatever was on the stack, and the ABI differential's byte comparison failed at
/// **byte 58** against a `.cu` that begins its own builder with `memset(&F, 0, sizeof(F))`.
/// ⊘ 2026-11-13: the bytes are now produced by [`abi_bytes`], which writes padding as zero
/// whatever the value's padding holds; the zeroed base stays so every field starts at zero, as
/// the `.cu`'s does.
#[must_use]
pub fn zeroed<T: AbiField>() -> T {
    // SAFETY: `T: AbiField` (sealed, implemented only in this file): an integer, an array of such,
    // or an aggregate of such fields — all-zero is a valid value (no reference, no `NonZero`, no
    // niche, no pointer Rust tracks).
    unsafe { core::mem::zeroed() }
}

/// `CU_MEM_LOCATION_TYPE_DEVICE`.
const CU_MEM_LOCATION_TYPE_DEVICE: u32 = 0x1;
/// `CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR`.
const CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR: u32 = 0x1;

// ⊘ 2026-11-13: `export_device_allocation` / `ExportedAllocation` (w755i: a CUDA allocation
// exported to an fd) were removed. No caller in the tree; it returned a raw fd number nothing owned
// and never released the allocation handle — an unsound-shaped public API kept for an experiment
// whose answer (RM refuses to import CUDA's fd, w755v) is recorded. Four unsafe blocks fewer.

impl Cuda {
    /// ★★★★★ **w755w — IMPORT AN RM-EXPORTED fd AND MAP IT TO A DEVICE POINTER.**
    ///
    /// The direction `[measured w755v]` established: RM allocates and exports; CUDA imports.
    /// The reverse is refused by RM at `os.c:2377` (`nvfp->handles == NULL`), because RM
    /// registers `handles[0]` only in its own export path.
    ///
    /// ⊘ **This is the question the whole table-refresh design waits on.** The walk kernel
    /// dereferences `KfWin { base, len }` at **GPGA offsets** — so if the single store can be
    /// given a device pointer here, the kernel walks the guest's tables **in place** and the
    /// blind CPU walk (refused by CUT A) is deleted rather than worked around.
    ///
    /// ⚠ `osHandle` for `POSIX_FILE_DESCRIPTOR` is the **fd cast to a pointer**, not a
    /// pointer to the fd. The other reading yields `INVALID_VALUE`, which reads like a
    /// rejected handle rather than a mis-passed argument — the shape that cost w755v two
    /// wrong answers.
    ///
    /// ★ 2026-11-13: returns a [`DevBuf`] of exactly `bytes` (D-02); a negative fd and an empty
    /// object are refused here (a negative fd used to become fd 0, stdin: D-14).
    ///
    /// CALLER-CONTRACT: `fd` names an open RM export of at least `bytes` bytes. Not validated
    /// here beyond its sign: an fd number cannot be checked for openness without owning it, and
    /// the public callers (`WalkKernel::import_store`, `DisplayGpu::import_*`) receive a raw `i32`
    /// from kf-qemu. The driver refuses a non-RM fd and a map larger than the object (INFERRED),
    /// so a wrong fd is a refusal, not memory access.
    ///
    /// # Errors
    /// A string naming the call that refused **and its rc**, so a precondition failure can
    /// never be read as CUDA's verdict (w755v).
    pub fn import_and_map(&self, device: i32, fd: i32, bytes: usize) -> Result<DevBuf, String> {
        let (Some(import), Some(reserve), Some(map), Some(set_access)) = (
            self.cuMemImportFromShareableHandle,
            self.cuMemAddressReserve,
            self.cuMemMap,
            self.cuMemSetAccess,
        ) else {
            return Err(
                "NO-VMM-API: this libcuda has no Import/AddressReserve/Map/SetAccess".into(),
            );
        };
        let Ok(fd_bits) = usize::try_from(fd) else {
            return Err(format!("import_and_map: fd {fd} is negative"));
        };
        if bytes == 0 {
            return Err("import_and_map: an empty object".into());
        }
        let life = current_life("cuMemImportFromShareableHandle").map_err(|e| e.to_string())?;
        let mut handle: u64 = 0;
        // SAFETY: `handle` is a live local; the non-negative `fd` is cast to the pointer-sized
        // osHandle the POSIX_FILE_DESCRIPTOR type specifies (a number, never dereferenced by us).
        let rc = unsafe {
            import(
                &raw mut handle,
                fd_bits as *mut c_void,
                CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR,
            )
        };
        if rc != 0 {
            return Err(format!("cuMemImportFromShareableHandle rc={rc} fd={fd}"));
        }
        // ★ 2026-11-14 (D-15): from here on a failed step gives back what the earlier steps took
        // (`import_unwind`), instead of leaking the imported handle and the reservation into the
        // context for its life.
        let mut ptr: u64 = 0;
        // SAFETY: `ptr` is a live local. Alignment 0 lets CUDA choose.
        let rc = unsafe { reserve(&raw mut ptr, bytes, 0, 0, 0) };
        if rc != 0 {
            self.import_unwind(handle, None, false);
            return Err(format!("cuMemAddressReserve rc={rc} bytes={bytes}"));
        }
        // SAFETY: `ptr` is a reservation of `bytes` and `handle` is a live imported handle.
        let rc = unsafe { map(ptr, bytes, 0, handle, 0) };
        if rc != 0 {
            self.import_unwind(handle, Some((ptr, bytes)), false);
            return Err(format!("cuMemMap rc={rc} ptr={ptr:#x} bytes={bytes}"));
        }
        // `CUmemAccessDesc { CUmemLocation { type, id }, flags }` — 12 bytes, transcribed and
        // checked against `cuda.h`'s own layout rather than remembered (w755v).
        #[repr(C)]
        struct AccessDesc {
            location_type: c_uint,
            location_id: c_int,
            flags: c_uint,
        }
        let desc = AccessDesc {
            location_type: CU_MEM_LOCATION_TYPE_DEVICE,
            location_id: device,
            flags: 3, // CU_MEM_ACCESS_FLAGS_PROT_READWRITE
        };
        // SAFETY: one live `AccessDesc` for a mapped range of `bytes`.
        let rc = unsafe { set_access(ptr, bytes, (&raw const desc).cast::<c_void>(), 1) };
        if rc != 0 {
            self.import_unwind(handle, Some((ptr, bytes)), true);
            return Err(format!("cuMemSetAccess rc={rc} ptr={ptr:#x}"));
        }
        Ok(DevBuf {
            ptr,
            len: bytes,
            kind: DevKind::Imported,
            life,
            owner: None,
        })
    }

    /// ★ D-15: undo a half-done [`Self::import_and_map`], in reverse order — unmap (when
    /// `mapped`), free the reservation, release the imported handle. Best effort: a driver without
    /// an undo entry, or one that refuses it, leaves that piece to `cuCtxDestroy` (the old
    /// behaviour); nothing else names these values, so nothing can use them afterwards.
    fn import_unwind(&self, handle: u64, reserved: Option<(u64, usize)>, mapped: bool) {
        if let (true, Some((ptr, bytes)), Some(unmap)) = (mapped, reserved, self.cuMemUnmap) {
            // SAFETY: `[ptr, ptr + bytes)` is the range `cuMemMap` mapped in this failed import;
            // no `DevBuf` names it, so nothing can reach it after this.
            unsafe { unmap(ptr, bytes) };
        }
        if let (Some((ptr, bytes)), Some(free)) = (reserved, self.cuMemAddressFree) {
            // SAFETY: `ptr`/`bytes` are exactly the reservation `cuMemAddressReserve` returned in
            // this failed import (unmapped just above if it had been mapped).
            unsafe { free(ptr, bytes) };
        }
        if let Some(release) = self.cuMemRelease {
            // SAFETY: `handle` is the one `cuMemImportFromShareableHandle` returned in this failed
            // import, released once; a mapping of it was undone above.
            unsafe { release(handle) };
        }
    }
}

/// ★★★ THE FAKE DRIVER (tests only, 2026-11-13). A [`Cuda`] whose every entry point is a plain
/// safe `extern "C" fn` that records the call and returns `CUDA_SUCCESS`, so the perimeter's
/// checks (and the in-tree callers' use of them) run without a GPU. ⊘ It contains no `unsafe`: the
/// stubs never write an out-pointer (so every handle and device address it mints is 0 — the checks
/// are about lengths, offsets, counts and lifetimes, not about absolute addresses), and the two
/// places that need a real value are test hooks in the safe surface (`device_count`,
/// `pinned_alloc`). A call that reaches a stub is logged in [`fake::calls`].
#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    /// One driver call that reached the fake.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum Call {
        Launch {
            grid: u32,
            block: u32,
            shmem: u32,
        },
        HtoD {
            dst: u64,
            n: usize,
        },
        DtoH {
            src: u64,
            n: usize,
        },
        Memset {
            dst: u64,
            n: usize,
        },
        MemsetAsync {
            dst: u64,
            n: usize,
        },
        DtoD {
            dst: u64,
            src: u64,
            n: usize,
        },
        DtoHAsync {
            src: u64,
            n: usize,
        },
        Free(u64),
        CtxDestroy,
        /// `cuMemFreeHost` / `cuMemHostUnregister` (D-05(b)).
        FreeHost,
        HostUnregister,
        /// The three undo steps of a failed import (D-15).
        Unmap,
        AddressFree,
        Release,
        StreamDestroy,
        EventDestroy,
        HostFn,
        GraphSet,
    }

    thread_local! {
        /// Entry points that refuse (return 1) on this thread until [`succeed_all`] (D-15 tests).
        static FAILING: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
        static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
        /// Host functions queued by [`deferred_hostfn`], run by [`run_deferred`].
        static DEFERRED: RefCell<Vec<(extern "C" fn(*mut c_void), usize)>> =
            const { RefCell::new(Vec::new()) };
    }

    fn log(c: Call) {
        CALLS.with(|v| v.borrow_mut().push(c));
    }

    /// Make entry point `name` refuse on this thread (the stubs that consult it: event create,
    /// event record, address reserve, map, set access; `cuEventQuery` answers NOT READY).
    pub(crate) fn fail(name: &'static str) {
        FAILING.with(|f| f.borrow_mut().push(name));
    }

    /// Every entry point succeeds again.
    pub(crate) fn succeed_all() {
        FAILING.with(|f| f.borrow_mut().clear());
    }

    fn rc(name: &str) -> CUresult {
        CUresult::from(FAILING.with(|f| f.borrow().contains(&name)))
    }

    /// Every call this thread's fake saw since the last `calls()`, oldest first.
    pub(crate) fn calls() -> Vec<Call> {
        CALLS.with(|v| core::mem::take(&mut *v.borrow_mut()))
    }

    /// How many kernel launches reached the fake since the last `calls()`.
    pub(crate) fn launches() -> usize {
        CALLS.with(|v| {
            v.borrow()
                .iter()
                .filter(|c| matches!(c, Call::Launch { .. }))
                .count()
        })
    }

    extern "C" fn init(_: c_uint) -> CUresult {
        0
    }
    extern "C" fn dev_get(_: *mut c_int, _: c_int) -> CUresult {
        0
    }
    extern "C" fn dev_count(_: *mut c_int) -> CUresult {
        0
    }
    extern "C" fn dev_name(_: *mut c_char, _: c_int, _: c_int) -> CUresult {
        1 // "no name": the safe surface then never reads the buffer
    }
    extern "C" fn dev_by_pci(_: *mut c_int, _: *const c_char) -> CUresult {
        0
    }
    extern "C" fn ctx_create(_: *mut *mut c_void, _: c_uint, _: c_int) -> CUresult {
        0
    }
    extern "C" fn ctx_destroy(_: *mut c_void) -> CUresult {
        log(Call::CtxDestroy);
        0
    }
    extern "C" fn stream_destroy(_: *mut c_void) -> CUresult {
        log(Call::StreamDestroy);
        0
    }
    extern "C" fn event_destroy(_: *mut c_void) -> CUresult {
        log(Call::EventDestroy);
        0
    }
    extern "C" fn p1(_: *mut c_void) -> CUresult {
        0
    }
    extern "C" fn p0() -> CUresult {
        0
    }
    extern "C" fn module_load(_: *mut *mut c_void, _: *const c_void) -> CUresult {
        0
    }
    extern "C" fn module_fn(_: *mut *mut c_void, _: *mut c_void, _: *const c_char) -> CUresult {
        0
    }
    extern "C" fn mem_alloc(_: *mut CUdeviceptr, _: usize) -> CUresult {
        0
    }
    extern "C" fn mem_free(p: CUdeviceptr) -> CUresult {
        log(Call::Free(p));
        0
    }
    extern "C" fn memset(dst: CUdeviceptr, _: u8, n: usize) -> CUresult {
        log(Call::Memset { dst, n });
        0
    }
    extern "C" fn h2d(dst: CUdeviceptr, _: *const c_void, n: usize) -> CUresult {
        log(Call::HtoD { dst, n });
        0
    }
    extern "C" fn d2h(_: *mut c_void, src: CUdeviceptr, n: usize) -> CUresult {
        log(Call::DtoH { src, n });
        0
    }
    #[allow(clippy::too_many_arguments)]
    extern "C" fn launch(
        _: *mut c_void,
        grid: c_uint,
        _: c_uint,
        _: c_uint,
        block: c_uint,
        _: c_uint,
        _: c_uint,
        shmem: c_uint,
        _: *mut c_void,
        _: *mut *mut c_void,
        _: *mut *mut c_void,
    ) -> CUresult {
        log(Call::Launch { grid, block, shmem });
        0
    }
    extern "C" fn create2(_: *mut *mut c_void, _: c_uint) -> CUresult {
        0
    }
    extern "C" fn elapsed(_: *mut f32, _: *mut c_void, _: *mut c_void) -> CUresult {
        0
    }
    extern "C" fn hostfn(
        _: *mut c_void,
        f: extern "C" fn(*mut c_void),
        user: *mut c_void,
    ) -> CUresult {
        log(Call::HostFn);
        f(user);
        0
    }
    /// A `cuLaunchHostFunc` that QUEUES the function (as the driver does) until [`run_deferred`].
    pub(crate) extern "C" fn deferred_hostfn(
        _: *mut c_void,
        f: extern "C" fn(*mut c_void),
        user: *mut c_void,
    ) -> CUresult {
        log(Call::HostFn);
        DEFERRED.with(|d| d.borrow_mut().push((f, user as usize)));
        0
    }
    /// Run every queued host function, as the driver's callback thread would.
    pub(crate) fn run_deferred() {
        for (f, user) in DEFERRED.with(|d| core::mem::take(&mut *d.borrow_mut())) {
            f(user as *mut c_void);
        }
    }
    extern "C" fn alloc_host(_: *mut *mut c_void, _: usize) -> CUresult {
        0
    }
    extern "C" fn host_devptr(_: *mut CUdeviceptr, _: *mut c_void, _: c_uint) -> CUresult {
        0
    }
    extern "C" fn d2h_async(
        _: *mut c_void,
        src: CUdeviceptr,
        n: usize,
        _: *mut c_void,
    ) -> CUresult {
        log(Call::DtoHAsync { src, n });
        0
    }
    extern "C" fn d2d_async(
        dst: CUdeviceptr,
        src: CUdeviceptr,
        n: usize,
        _: *mut c_void,
    ) -> CUresult {
        log(Call::DtoD { dst, src, n });
        0
    }
    extern "C" fn memset_async(dst: CUdeviceptr, _: u8, n: usize, _: *mut c_void) -> CUresult {
        log(Call::MemsetAsync { dst, n });
        0
    }
    extern "C" fn import(_: *mut u64, _: *mut c_void, _: c_uint) -> CUresult {
        0
    }
    extern "C" fn reserve(_: *mut u64, _: usize, _: usize, _: u64, _: u64) -> CUresult {
        rc("cuMemAddressReserve")
    }
    extern "C" fn map(_: u64, _: usize, _: usize, _: u64, _: u64) -> CUresult {
        rc("cuMemMap")
    }
    extern "C" fn set_access(_: u64, _: usize, _: *const c_void, _: usize) -> CUresult {
        rc("cuMemSetAccess")
    }
    extern "C" fn unmap(_: u64, _: usize) -> CUresult {
        log(Call::Unmap);
        0
    }
    extern "C" fn address_free(_: u64, _: usize) -> CUresult {
        log(Call::AddressFree);
        0
    }
    extern "C" fn release(_: u64) -> CUresult {
        log(Call::Release);
        0
    }
    extern "C" fn event_create(_: *mut *mut c_void, _: c_uint) -> CUresult {
        rc("cuEventCreate")
    }
    extern "C" fn dev_attr(_: *mut c_int, _: c_int, _: c_int) -> CUresult {
        rc("cuDeviceGetAttribute")
    }
    extern "C" fn event_record(_: *mut c_void, _: *mut c_void) -> CUresult {
        rc("cuEventRecord")
    }
    /// `fail("cuEventQuery")` makes every event NOT READY (the GPU has not got there yet).
    extern "C" fn event_query(_: *mut c_void) -> CUresult {
        if rc("cuEventQuery") != 0 {
            CUDA_ERROR_NOT_READY
        } else {
            0
        }
    }
    extern "C" fn free_host(_: *mut c_void) -> CUresult {
        log(Call::FreeHost);
        0
    }
    extern "C" fn host_unregister(_: *mut c_void) -> CUresult {
        log(Call::HostUnregister);
        0
    }
    extern "C" fn host_register(_: *mut c_void, _: usize, _: c_uint) -> CUresult {
        0
    }
    extern "C" fn graph_set(
        _: *mut c_void,
        _: *mut c_void,
        _: *const KernelNodeParams,
    ) -> CUresult {
        log(Call::GraphSet);
        0
    }

    impl Cuda {
        /// The fake binding. No graph API (capture needs out-pointers the fake cannot write);
        /// the VMM import set and host registration are present.
        pub(crate) fn fake() -> Cuda {
            Cuda {
                _handle: core::ptr::null_mut(),
                cuInit: init,
                cuDeviceGet: dev_get,
                cuDeviceGetCount: dev_count,
                cuDeviceGetName: dev_name,
                cuDeviceGetByPCIBusId: dev_by_pci,
                cuDeviceGetAttribute: Some(dev_attr),
                cuCtxCreate: ctx_create,
                cuCtxDestroy: ctx_destroy,
                cuCtxSynchronize: p0,
                cuCtxSetCurrent: p1,
                cuModuleLoadData: module_load,
                cuModuleGetFunction: module_fn,
                cuMemAlloc: mem_alloc,
                cuMemFree: mem_free,
                cuMemsetD8: memset,
                cuMemcpyHtoD: h2d,
                cuMemcpyDtoH: d2h,
                cuLaunchKernel: launch,
                cuGetErrorName: None,
                cuStreamCreate: create2,
                cuStreamDestroy: stream_destroy,
                cuEventCreate: event_create,
                cuEventDestroy: event_destroy,
                cuEventRecord: event_record,
                cuEventQuery: event_query,
                cuEventElapsedTime: elapsed,
                cuLaunchHostFunc: hostfn,
                cuMemAllocHost: alloc_host,
                cuMemFreeHost: free_host,
                cuMemHostGetDevicePointer: host_devptr,
                cuMemcpyDtoHAsync: d2h_async,
                cuMemcpyDtoDAsync: d2d_async,
                cuMemsetD8Async: memset_async,
                cuStreamBeginCapture: None,
                cuStreamEndCapture: None,
                cuStreamGetCaptureInfo: None,
                cuGraphInstantiateWithFlags: None,
                cuGraphLaunch: None,
                cuGraphExecKernelNodeSetParams: Some(graph_set),
                cuGraphExecDestroy: None,
                cuGraphDestroy: None,
                cuGraphUpload: None,
                cuEventRecordWithFlags: None,
                ctx_sync_calls: core::sync::atomic::AtomicU64::new(0),
                cuMemImportFromShareableHandle: Some(import),
                cuMemAddressReserve: Some(reserve),
                cuMemMap: Some(map),
                cuMemSetAccess: Some(set_access),
                cuMemUnmap: Some(unmap),
                cuMemAddressFree: Some(address_free),
                cuMemRelease: Some(release),
                cuMemHostRegister: Some(host_register),
                cuMemHostUnregister: Some(host_unregister),
            }
        }
    }
}

#[cfg(test)]
impl DevBuf {
    /// A buffer of a live (test) context at a fixed fake address, for pure checks that never
    /// reach a driver.
    pub(crate) fn fake(len: usize) -> DevBuf {
        DevBuf {
            ptr: 0x1000_0000,
            len,
            kind: DevKind::Owned,
            life: Life::new(),
            owner: None,
        }
    }
}

/// ★★★ Security review 2 (2026-11-13): every check of the perimeter, reached with a hostile or
/// wrong argument. A refusal is a panic BEFORE the driver is called (the fake's log proves the
/// "before"), never a driver call with the bad value.
#[cfg(test)]
mod perimeter_tests {
    use super::fake::{Call, calls, launches};
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    /// Run `f`, require that it panicked, and return the driver calls made meanwhile.
    fn refused_before_the_driver(f: impl FnOnce()) -> Vec<Call> {
        let _ = calls();
        let r = catch_unwind(AssertUnwindSafe(f));
        assert!(r.is_err(), "the hostile call was not refused");
        calls()
    }

    /// A fake binding with a live context current on this thread.
    fn up() -> (Cuda, CtxHandle) {
        let cu = Cuda::fake();
        let ctx = cu.ctx_create(0).expect("fake context");
        let _ = calls();
        (cu, ctx)
    }

    fn module(cu: &Cuda, ptx: &[u8]) -> ModuleHandle {
        let mut v = ptx.to_vec();
        v.push(0);
        cu.module_load(&v).expect("the committed PTX parses")
    }

    // ═══ D-01: parameter lists ═══

    /// The committed PTX's entry lists, against the counts the review MEASURED (§0 of appendix D).
    #[test]
    fn d01_the_committed_ptx_parameter_lists_parse_as_measured() {
        let walk = ptx_entries(crate::walk::WALK_PTX).unwrap();
        assert_eq!(walk.len(), 14, "kf_walk.ptx has 14 entries");
        let n = |name: &str| {
            walk.iter()
                .find(|e| e.name.contains(name))
                .unwrap_or_else(|| panic!("{name}"))
                .params
                .clone()
        };
        for (k, count) in [
            ("kf_par_seed", 4),
            ("kf_par_expand", 9),
            ("kf_par_scan", 4),
            ("kf_par_compact", 8),
            ("kf_par_leaf", 7),
            ("kf_par_heads", 6),
            ("kf_par_bases", 2),
            ("kf_par_emit", 8),
            ("kf_par_join", 7),
        ] {
            assert_eq!(n(k).len(), count, "{k}");
        }
        let kfargs = core::mem::size_of::<crate::abi::KfArgs>();
        assert_eq!(kfargs, 264);
        assert_eq!(n("kf_commit_kernel"), vec![kfargs]);
        assert_eq!(n("kf_par_seed")[0], kfargs);
        assert!(n("kf_par_scan").iter().all(|&s| s == 8), "four pointers");
        let disp = |ptx: &[u8], name: &str| {
            ptx_entries(ptx)
                .unwrap()
                .into_iter()
                .find(|e| e.name == name)
                .unwrap_or_else(|| panic!("{name}"))
                .params
        };
        use crate::display::{COLOR_PTX, PACK_PTX, SCANOUT_PTX, SUM_PTX, YUV_PTX};
        assert_eq!(disp(SCANOUT_PTX, "kf_compose").len(), 18);
        assert_eq!(disp(PACK_PTX, "kf_bl_pack").len(), 12);
        assert_eq!(disp(SUM_PTX, "kf_sum").len(), 4);
        assert_eq!(disp(COLOR_PTX, "kf_color_validate").len(), 3);
        assert_eq!(disp(COLOR_PTX, "kf_tmo_validate").len(), 3);
        assert_eq!(disp(COLOR_PTX, "kf_color_compose").len(), 25);
        let out = disp(COLOR_PTX, "kf_color_output");
        assert_eq!(out.len(), 7);
        assert!(out.contains(&48), "the by-value 3x4 matrix");
        assert_eq!(disp(YUV_PTX, "kf_compose_yuv").len(), 34);
        assert_eq!(disp(YUV_PTX, "kf_color_yuv").len(), 34);
    }

    #[test]
    fn d01_a_declaration_that_does_not_parse_refuses_the_image() {
        let ok = ".visible .entry k(\n .param .u64 k_param_0,\n .param .align 8 .b8 k_param_1[24]\n)\n{ ret; }";
        assert_eq!(
            ptx_entries(ok.as_bytes()).unwrap(),
            vec![PtxEntry {
                name: "k".into(),
                params: vec![8, 24]
            }]
        );
        for bad in [
            ".entry k(\n .param .u128 a\n)",                     // unknown type
            ".entry k(\n .param a\n)",                           // no type
            ".entry k(\n .param .u32\n)",                        // no name
            ".entry k(\n .param .u32 .u64 a\n)",                 // two types
            ".entry k(\n .param .b8 a[x]\n)",                    // non-numeric array
            ".entry k(\n .param .b8 a[0]\n)",                    // empty array
            ".entry k(\n .param .b64 a[2305843009213693952]\n)", // overflows
            ".entry k(\n .reg .u32 a\n)",                        // not a .param
            ".entry k(\n .param .u32 a\n) .entry k(\n)",         // repeated entry
            ".entry k(\n .param .u32 a\n",                       // unterminated list
            "/* .entry k(",                                      // unterminated comment
            ".entry k@(\n)",                                     // bad name
        ] {
            assert!(
                ptx_entries(bad.as_bytes()).is_err(),
                "{bad:?} must not parse"
            );
        }
        assert!(ptx_entries(&[0xff, 0xfe]).is_err(), "not UTF-8");
        // a commented-out entry is not an entry
        let c = "// .entry ghost(\n.entry k()\n{}";
        assert_eq!(ptx_entries(c.as_bytes()).unwrap().len(), 1);
    }

    #[test]
    fn d01_an_entry_the_ptx_text_does_not_declare_is_refused_before_the_driver() {
        let (cu, _ctx) = up();
        let m = module(&cu, crate::display::SUM_PTX);
        assert!(cu.module_function(&m, "kf_not_there").is_err());
        assert_eq!(
            cu.module_function(&m, "kf_sum")
                .unwrap()
                .param_sizes()
                .len(),
            4
        );
    }

    #[test]
    fn d01_a_short_parameter_list_is_refused_before_the_launch() {
        let (cu, _ctx) = up();
        let s = cu.stream_create().unwrap();
        let m = module(&cu, crate::display::SCANOUT_PTX);
        let f = cu.module_function(&m, "kf_compose").unwrap();
        // the review's scenario: one 1-byte parameter for an 18-parameter kernel
        let seen = refused_before_the_driver(|| {
            let _ = cu.launch_args(&s, &f, 1, 1, 0, &mut [vec![0u8; 1]], "t");
        });
        assert!(!seen.iter().any(|c| matches!(c, Call::Launch { .. })));
        let mut params: Vec<Vec<u8>> = f.param_sizes().iter().map(|&n| vec![0u8; n]).collect();
        params.pop();
        let seen = refused_before_the_driver(|| {
            let _ = cu.launch_args(&s, &f, 1, 1, 0, &mut params, "t");
        });
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[test]
    fn d01_a_wrong_parameter_size_is_refused_before_the_launch() {
        let (cu, _ctx) = up();
        let s = cu.stream_create().unwrap();
        let m = module(&cu, crate::walk::WALK_PTX);
        let f = cu
            .module_function(&m, "_Z16kf_commit_kernel6KfArgs")
            .unwrap();
        // a 263-byte KfArgs: the review's one-byte host over-read
        let seen = refused_before_the_driver(|| {
            let _ = cu.launch_args(&s, &f, 1, 1, 0, &mut [vec![0u8; 263]], "t");
        });
        assert!(seen.is_empty());
        let seen = refused_before_the_driver(|| {
            let _ = cu.launch_raw(&f, 1, 1, &mut [0u8; 265]);
        });
        assert!(seen.is_empty());
        // and the exact list reaches the driver
        cu.launch_args(&s, &f, 1, 1, 0, &mut [vec![0u8; 264]], "t")
            .unwrap();
        assert_eq!(launches(), 1);
        assert_eq!(cu.launch_raw(&f, 1, 1, &mut [0u8; 264]), CUDA_SUCCESS);
    }

    #[test]
    fn d01_launch_raw_refuses_a_kernel_with_several_parameters() {
        let (cu, _ctx) = up();
        let m = module(&cu, crate::display::SUM_PTX);
        let f = cu.module_function(&m, "kf_sum").unwrap();
        let seen = refused_before_the_driver(|| {
            let _ = cu.launch_raw(&f, 1, 1, &mut [0u8; 8]);
        });
        assert!(seen.is_empty());
    }

    #[test]
    fn d01_graph_node_parameters_are_checked_and_the_node_must_be_the_graphs() {
        let (cu, ctx) = up();
        let m = module(&cu, crate::display::SUM_PTX);
        let f = cu.module_function(&m, "kf_sum").unwrap();
        let graph = Life::new();
        let exec = GraphExecHandle {
            raw: 1,
            life: ctx.life.clone(),
            graph: graph.clone(),
        };
        let node = GraphNode {
            raw: 2,
            graph: graph.clone(),
        };
        let mut ok: Vec<Vec<u8>> = vec![vec![0; 8], vec![0; 8], vec![0; 4], vec![0; 4]];
        let seen = refused_before_the_driver(|| {
            let _ = cu.graph_exec_kernel_set(&exec, &node, &f, 1, 1, 0, &mut ok[..3].to_vec(), "t");
        });
        assert!(seen.is_empty(), "short list");
        let foreign = GraphNode {
            raw: 3,
            graph: Life::new(),
        };
        let seen = refused_before_the_driver(|| {
            let _ = cu.graph_exec_kernel_set(&exec, &foreign, &f, 1, 1, 0, &mut ok.clone(), "t");
        });
        assert!(seen.is_empty(), "a node of another graph");
        cu.graph_exec_kernel_set(&exec, &node, &f, 1, 1, 0, &mut ok, "t")
            .unwrap();
        assert_eq!(calls(), vec![Call::GraphSet]);
        graph.end(); // the graph destroyed: its nodes refuse
        let seen = refused_before_the_driver(|| {
            let _ = cu.graph_exec_kernel_set(&exec, &node, &f, 1, 1, 0, &mut ok.clone(), "t");
        });
        assert!(seen.is_empty(), "a node of a destroyed graph");
    }

    #[test]
    fn d01_a_wrong_size_in_a_graph_node_update_is_refused() {
        let (cu, ctx) = up();
        let m = module(&cu, crate::display::SUM_PTX);
        let f = cu.module_function(&m, "kf_sum").unwrap();
        let graph = Life::new();
        let exec = GraphExecHandle {
            raw: 1,
            life: ctx.life.clone(),
            graph: graph.clone(),
        };
        let node = GraphNode { raw: 2, graph };
        let mut p: Vec<Vec<u8>> = vec![vec![0; 8], vec![0; 8], vec![0; 5], vec![0; 4]];
        let seen = refused_before_the_driver(|| {
            let _ = cu.graph_exec_kernel_set(&exec, &node, &f, 1, 1, 0, &mut p, "t");
        });
        assert!(seen.is_empty(), "a 5-byte u32");
        // a node recorded in ANOTHER capture is not this executable's
        let other = GraphNode {
            raw: 2,
            graph: Life::new(),
        };
        p[2] = vec![0; 4];
        let seen = refused_before_the_driver(|| {
            let _ = cu.graph_exec_kernel_set(&exec, &other, &f, 1, 1, 0, &mut p, "t");
        });
        assert!(seen.is_empty(), "a node of another capture");
    }

    #[test]
    fn d01_an_unparseable_image_is_refused_by_module_load() {
        let (cu, _ctx) = up();
        let bad = b".visible .entry k(\n .param .q64 a\n)\n{}\0";
        assert!(cu.module_load(bad).is_err(), "an unknown parameter type");
        let ok = b".visible .entry k(\n .param .u64 a\n)\n{}\0";
        let m = cu.module_load(ok).unwrap();
        assert_eq!(cu.module_function(&m, "k").unwrap().param_sizes(), &[8]);
    }

    #[test]
    fn d01_module_function_refuses_a_name_only_the_driver_knows() {
        let (cu, _ctx) = up();
        let m = module(&cu, crate::walk::WALK_PTX);
        // the fake driver would hand out a function for ANY name; the PTX text decides
        assert!(cu.module_function(&m, "kf_compose").is_err());
        assert!(cu.module_function(&m, "").is_err());
    }

    // ═══ D-02: device ranges ═══

    #[test]
    fn d02_a_span_outside_its_allocation_cannot_be_made() {
        let (cu, _ctx) = up();
        let b = cu.mem_alloc_zeroed(4096, "t").unwrap();
        assert!(b.try_span(0, 4096).is_some());
        assert!(b.try_span(4096, 0).is_some(), "an empty span at the end");
        assert!(b.try_span(1, 4096).is_none(), "one byte past");
        assert!(b.try_span(4097, 0).is_none());
        assert!(
            b.try_span(u64::MAX, 2).is_none(),
            "offset + length overflows"
        );
        assert!(catch_unwind(AssertUnwindSafe(|| b.span(4000, 97))).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| b.span(0, 64).sub(60, 5))).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| b.span(0, 64).sub(u64::MAX, 1))).is_err());
        assert_eq!(b.span(0, 64).sub(60, 4).len(), 4);
    }

    #[test]
    fn d02_copies_and_sets_are_bounded_by_the_span() {
        let (cu, _ctx) = up();
        let s = cu.stream_create().unwrap();
        let b = cu.mem_alloc_zeroed(64, "t").unwrap();
        let other = cu.mem_alloc_zeroed(16, "t").unwrap();
        let _ = calls();
        let seen = refused_before_the_driver(|| {
            let _ = cu.memcpy_h2d(b.span(0, 16), &[0u8; 17], "t");
        });
        assert!(seen.is_empty(), "host bytes longer than the device span");
        let seen = refused_before_the_driver(|| {
            let _ = cu.memcpy_d2h(&mut [0u8; 65], b.whole(), "t");
        });
        assert!(seen.is_empty(), "read past the allocation");
        let seen = refused_before_the_driver(|| {
            let _ = cu.memcpy_d2d_async(&s, other.whole(), b.whole(), "t");
        });
        assert!(seen.is_empty(), "64 bytes into 16");
        // in bounds: exactly the span's range reaches the driver
        cu.memset_d8(b.span(8, 16), 0, "t").unwrap();
        cu.memset_d8_async(&s, b.span(0, 4), 0, "t").unwrap();
        cu.memcpy_h2d(b.span(32, 32), &[1u8; 32], "t").unwrap();
        cu.memcpy_d2d_async(&s, b.span(0, 16), other.whole(), "t")
            .unwrap();
        assert_eq!(
            calls(),
            vec![
                Call::Memset { dst: 8, n: 16 },
                Call::MemsetAsync { dst: 0, n: 4 },
                Call::HtoD { dst: 32, n: 32 },
                Call::DtoD {
                    dst: 0,
                    src: 0,
                    n: 16
                },
            ]
        );
    }

    #[test]
    fn d02_a_pinned_view_is_bounded_and_dies_with_its_buffer() {
        let (cu, _ctx) = up();
        let s = cu.stream_create().unwrap();
        let pin = cu.pinned_alloc(256, "t").unwrap();
        assert!(catch_unwind(AssertUnwindSafe(|| cu.pinned_device_view(&pin, 200, 57))).is_err());
        assert!(
            catch_unwind(AssertUnwindSafe(|| cu.pinned_device_view(
                &pin,
                usize::MAX,
                2
            )))
            .is_err()
        );
        let v = cu.pinned_device_view(&pin, 128, 128).unwrap();
        assert_eq!(v.len(), 128);
        let dev = cu.mem_alloc_zeroed(512, "t").unwrap();
        let seen = refused_before_the_driver(|| {
            let _ = cu.memcpy_d2h_async(&s, &pin, 200, dev.span(0, 57), "t");
        });
        assert!(seen.is_empty(), "a D2H into pinned memory past its end");
        // ★ D-05(b): a buffer that lent a device view is kept until the context dies; its token
        // ends all the same, so the view refuses
        assert!(cu.pinned_free(pin, "t").is_err());
        let seen = refused_before_the_driver(|| {
            let _ = cu.memset_d8(v.whole(), 0, "t");
        });
        assert!(seen.is_empty(), "a view of a freed pinned buffer");
    }

    #[test]
    fn d02_import_refuses_a_negative_fd_and_an_empty_object() {
        let (cu, _ctx) = up();
        assert!(
            cu.import_and_map(0, -1, 4096).is_err(),
            "never fd 0 (stdin)"
        );
        assert!(cu.import_and_map(0, 3, 0).is_err());
        let b = cu.import_and_map(0, 3, 4096).unwrap();
        assert_eq!((b.len(), b.kind()), (4096, DevKind::Imported));
        let _ = calls();
        cu.mem_free(b);
        assert!(calls().is_empty(), "an import is never handed to cuMemFree");
    }

    #[test]
    fn d02_a_copy_longer_than_its_span_is_refused() {
        let (cu, _ctx) = up();
        let b = cu.mem_alloc_zeroed(4096, "t").unwrap();
        for (off, len, host) in [(0u64, 8usize, 9usize), (4088, 8, 16), (100, 0, 1)] {
            let span = b.span(off, len);
            let seen = refused_before_the_driver(|| {
                let _ = cu.memcpy_h2d(span, &vec![0u8; host], "t");
            });
            assert!(seen.is_empty(), "h2d {host} into {len}");
            let seen = refused_before_the_driver(|| {
                let _ = cu.memcpy_d2h(&mut vec![0u8; host], span, "t");
            });
            assert!(seen.is_empty(), "d2h {host} from {len}");
        }
    }

    #[test]
    fn d02_pinned_views_and_async_reads_are_exactly_their_region() {
        let (cu, _ctx) = up();
        let s = cu.stream_create().unwrap();
        let pin = cu.pinned_alloc(64, "t").unwrap();
        assert!(catch_unwind(AssertUnwindSafe(|| cu.pinned_device_view(&pin, 0, 65))).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| cu.pinned_device_view(&pin, 64, 1))).is_err());
        assert_eq!(cu.pinned_device_view(&pin, 64, 0).unwrap().len(), 0);
        let dev = cu.mem_alloc_zeroed(64, "t").unwrap();
        let seen = refused_before_the_driver(|| {
            let _ = cu.memcpy_d2h_async(&s, &pin, 1, dev.whole(), "t");
        });
        assert!(
            seen.is_empty(),
            "64 bytes at offset 1 of a 64-byte pinned buffer"
        );
        let seen = refused_before_the_driver(|| {
            let _ = cu.memcpy_d2h_async(&s, &pin, usize::MAX, dev.span(0, 1), "t");
        });
        assert!(seen.is_empty(), "offset + length overflows");
        cu.memcpy_d2h_async(&s, &pin, 0, dev.whole(), "t").unwrap();
    }

    #[test]
    fn d04_a_buffer_of_a_destroyed_context_is_never_freed_again() {
        let (cu, ctx) = up();
        let b = DevBuf {
            ptr: 0x8000,
            ..cu.mem_alloc_zeroed(8, "t").unwrap()
        };
        let pin = cu.pinned_alloc(8, "t").unwrap();
        cu.ctx_destroy(ctx);
        let _ = calls();
        cu.mem_free(b);
        cu.pinned_free(pin, "t").unwrap();
        assert!(calls().is_empty(), "cuCtxDestroy already reclaimed them");
    }

    // ═══ D-04: lifetimes ═══

    #[test]
    fn d04_every_child_handle_refuses_after_its_context_is_destroyed() {
        let (cu, ctx) = up();
        let s = cu.stream_create().unwrap();
        let e = cu.event_create().unwrap();
        let m = module(&cu, crate::display::SUM_PTX);
        let f = cu.module_function(&m, "kf_sum").unwrap();
        // a non-zero address (the fake's are 0, which `mem_free` skips anyway)
        let b = DevBuf {
            ptr: 0x4000,
            ..cu.mem_alloc_zeroed(64, "t").unwrap()
        };
        let pin = cu.pinned_alloc(64, "t").unwrap();
        let _ = calls();
        cu.ctx_destroy(ctx);
        assert_eq!(calls(), vec![Call::CtxDestroy]);
        let mut p: Vec<Vec<u8>> = vec![vec![0; 8], vec![0; 8], vec![0; 4], vec![0; 4]];
        for (what, seen) in [
            (
                "stream",
                refused_before_the_driver(|| {
                    let _ = cu.event_record(&e, &s);
                }),
            ),
            (
                "event",
                refused_before_the_driver(|| {
                    let _ = cu.event_query(&e);
                }),
            ),
            (
                "module",
                refused_before_the_driver(|| {
                    let _ = cu.module_function(&m, "kf_sum");
                }),
            ),
            (
                "func",
                refused_before_the_driver(|| {
                    let _ = cu.launch_args(&s, &f, 1, 1, 0, &mut p, "t");
                }),
            ),
            (
                "buffer",
                refused_before_the_driver(|| {
                    let _ = cu.memset_d8(b.whole(), 0, "t");
                }),
            ),
            (
                "pinned",
                refused_before_the_driver(|| {
                    let _ = pin.read(0, 8);
                }),
            ),
        ] {
            assert!(seen.is_empty(), "{what}: {seen:?}");
        }
        assert!(
            cu.stream_create().is_err(),
            "the destroyed context was popped: nothing current"
        );
        // the destroys of reclaimed handles do not reach the driver either
        cu.mem_free(b);
        cu.stream_destroy(s);
        cu.event_destroy(e);
        assert!(
            calls().is_empty(),
            "a handle of a destroyed context is never destroyed again"
        );
    }

    #[test]
    fn d04_destroys_consume_and_reach_the_driver_once() {
        let (cu, ctx) = up();
        let s = cu.stream_create().unwrap();
        // the fake's handles are 0 (no out-pointer write), which a destroy treats as "none"; give
        // these real-looking values so the destroy path itself is exercised
        let s = StreamHandle { raw: 7, ..s };
        let e = EventHandle {
            raw: 9,
            life: ctx.life.clone(),
        };
        let b = DevBuf {
            ptr: 0x4000,
            ..cu.mem_alloc_zeroed(8, "t").unwrap()
        };
        let _ = calls();
        cu.stream_destroy(s);
        cu.event_destroy(e);
        cu.mem_free(b);
        cu.ctx_destroy(ctx);
        cu.ctx_destroy(CtxHandle::null());
        assert_eq!(
            calls(),
            vec![
                Call::StreamDestroy,
                Call::EventDestroy,
                Call::Free(0x4000),
                Call::CtxDestroy
            ]
        );
    }

    #[test]
    fn d04_no_context_current_refuses_creation_and_a_dead_one_cannot_be_made_current() {
        let cu = Cuda::fake();
        assert!(
            cu.stream_create().is_err(),
            "no context current on this thread"
        );
        assert!(cu.mem_alloc_zeroed(8, "t").is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| cu.ctx_set_current(&CtxHandle::null()))).is_err());
        let a = cu.ctx_create(0).unwrap();
        let b = cu.ctx_create(0).unwrap();
        cu.ctx_destroy(b); // pops b: a is current again, as in the driver
        assert!(cu.stream_create().is_ok());
        cu.ctx_set_current(&a).unwrap();
        cu.ctx_destroy(a);
        assert!(cu.stream_create().is_err());
    }

    // ═══ D-07: the completion fd outlives every queued signal ═══

    #[test]
    fn d07_a_queued_signal_keeps_the_fd_open_after_its_owner_dropped_it() {
        // ⊘ 2026-11-14: deterministic. The first version looked the fd NUMBER up in /proc after
        // the close, and a parallel test's new eventfd could take the same number (flaky 3/30).
        // Now the eventfd OBJECT is observed: a `Weak` to the shared `OwnedFd` (alive exactly while
        // a reference exists) and a dup of the descriptor (same open file description), which must
        // receive the queued signal however the fd numbers are reused meanwhile.
        use std::io::Read;
        let (mut cu, _ctx) = up();
        cu.cuLaunchHostFunc = fake::deferred_hostfn;
        let s = cu.stream_create().unwrap();
        let fd = CompletionFd::new().unwrap();
        let alive = Arc::downgrade(&fd.fd);
        let mut witness = std::fs::File::from(fd.fd.try_clone().expect("dup the eventfd"));
        cu.launch_host_signal(&s, &fd).unwrap();
        drop(fd);
        assert!(
            alive.upgrade().is_some(),
            "the queued signal holds the fd open after its owner dropped it"
        );
        fake::run_deferred();
        assert!(
            alive.upgrade().is_none(),
            "the last reference (the callback's) closed it"
        );
        let mut v = [0u8; 8];
        witness
            .read_exact(&mut v)
            .expect("the signal reached THIS eventfd, not a reused fd number");
        assert_eq!(u64::from_ne_bytes(v), 1);
    }

    #[test]
    fn d07_the_signal_reaches_the_fd() {
        let (cu, _ctx) = up();
        let s = cu.stream_create().unwrap();
        let fd = CompletionFd::new().unwrap();
        cu.launch_host_signal(&s, &fd).unwrap();
        assert!(fd.wait_readable(0));
        assert_eq!(fd.drain(), 1);
    }

    // ═══ D-08: byte views and zero values ═══

    /// Every `abi_struct!` list names exactly the fields of its struct in `abi.rs`, in order: a
    /// field missing from a list would be serialised as zero.
    #[test]
    fn abi_struct_lists_every_field() {
        let src = include_str!("abi.rs");
        for (name, fields) in ABI_STRUCT_FIELDS {
            let at = src
                .find(&format!("pub struct {name} {{"))
                .unwrap_or_else(|| panic!("NON-VACUITY: {name} is not in abi.rs"));
            let body = &src[at..at + src[at..].find("\n}").unwrap()];
            let declared: Vec<&str> = body
                .lines()
                .filter_map(|l| l.trim().strip_prefix("pub "))
                .filter_map(|l| l.split(':').next())
                .filter(|f| !f.starts_with("struct"))
                .collect();
            assert_eq!(&declared[..], *fields, "{name}");
        }
        assert_eq!(
            ABI_STRUCT_FIELDS.len(),
            src.matches("#[repr(C)]\npub struct ").count(),
            "every repr(C) struct of abi.rs has a list"
        );
    }

    #[test]
    fn d08_abi_bytes_writes_padding_as_zero_and_fields_in_place() {
        let mut f: crate::abi::KfFormat = zeroed();
        f.small_ps = 0xAA;
        f.root_align = 0x0403_0201;
        let b = abi_bytes(&f);
        assert_eq!(b.len(), 120);
        let ps = core::mem::offset_of!(crate::abi::KfFormat, small_ps);
        let ra = core::mem::offset_of!(crate::abi::KfFormat, root_align);
        assert_eq!(ra - ps - 1, 2, "the two padding bytes the review found");
        assert_eq!(b[ps], 0xAA);
        assert_eq!(&b[ps + 1..ra], &[0, 0], "padding is zero");
        assert_eq!(&b[ra..ra + 4], &0x0403_0201u32.to_ne_bytes());
        let run = crate::abi::KfMapRun {
            va: 1,
            gpga: 2,
            len: 3,
            flags: 4,
            op: 5,
            pdb_index: 6,
        };
        let r = abi_bytes(&run);
        assert_eq!(&r[24..28], &4u32.to_ne_bytes());
        assert_eq!(&r[30..32], &6u16.to_ne_bytes());
    }

    /// ★★ D-05(b) (2026-11-14): a pinned buffer whose queued copy has not completed is NOT given
    /// back — `cuMemFreeHost` is never called — and the refusal says why; once the copy's event
    /// completed it is freed. A copy whose fence could not be recorded makes the buffer unfreeable.
    #[test]
    fn d05b_pinned_memory_is_never_freed_under_a_queued_copy() {
        use super::fake::{fail, succeed_all};
        let (cu, _ctx) = up();
        let s = cu.stream_create().unwrap();
        let dev = cu.mem_alloc_zeroed(4096, "t").unwrap();
        let freed = |c: &[Call]| c.iter().filter(|c| **c == Call::FreeHost).count();
        // the control: a buffer no copy ever targeted is freed
        let pin = cu.pinned_alloc(4096, "t").unwrap();
        let _ = calls();
        cu.pinned_free(pin, "t").unwrap();
        assert_eq!(freed(&calls()), 1);
        // a copy queued, the GPU not there yet: refused, kept
        let pin = cu.pinned_alloc(4096, "t").unwrap();
        cu.memcpy_d2h_async(&s, &pin, 0, dev.span(0, 4096), "t")
            .unwrap();
        fail("cuEventQuery");
        let _ = calls();
        let e = cu.pinned_free(pin, "t").unwrap_err();
        assert!(e.to_string().contains("not completed"), "{e}");
        assert_eq!(freed(&calls()), 0, "the pages stay until the context dies");
        succeed_all();
        // the copy completed: freed
        let pin = cu.pinned_alloc(4096, "t").unwrap();
        cu.memcpy_d2h_async(&s, &pin, 0, dev.span(0, 4096), "t")
            .unwrap();
        let _ = calls();
        cu.pinned_free(pin, "t").unwrap();
        assert_eq!(freed(&calls()), 1);
        // the fence could not be recorded: completion is unknowable, so never freed alone
        let pin = cu.pinned_alloc(4096, "t").unwrap();
        fail("cuEventRecord");
        assert!(
            cu.memcpy_d2h_async(&s, &pin, 0, dev.span(0, 4096), "t")
                .is_err()
        );
        succeed_all();
        let _ = calls();
        assert!(cu.pinned_free(pin, "t").is_err());
        assert_eq!(freed(&calls()), 0);
        // a buffer that lent a device view to a kernel is kept until the context dies
        let pin = cu.pinned_alloc(4096, "t").unwrap();
        let _view = cu.pinned_device_view(&pin, 0, 64).unwrap();
        let _ = calls();
        let e = cu.pinned_free(pin, "t").unwrap_err();
        assert!(e.to_string().contains("device view"), "{e}");
        assert_eq!(freed(&calls()), 0);
    }

    /// ★★ D-05(b): the same for a REGISTERED mapping — no `cuMemHostUnregister` while a queued
    /// copy into it has not completed; unregistered once it has.
    #[test]
    fn d05b_a_registration_is_never_undone_under_a_queued_copy() {
        use super::fake::{fail, succeed_all};
        let (cu, _ctx) = up();
        let s = cu.stream_create().unwrap();
        let dev = cu.mem_alloc_zeroed(4096, "t").unwrap();
        let page = kf_linux_raw::HostPageSize::query();
        let map = kf_linux_raw::MappedRegion::map(
            kf_linux_raw::Backing::PrivateAnonymous,
            page.bytes(),
            kf_linux_raw::HostProt::ReadWrite,
            kf_linux_raw::CachePolicy::WriteBack,
            page,
        )
        .unwrap();
        let undone = |c: &[Call]| c.iter().filter(|c| **c == Call::HostUnregister).count();
        let reg = cu.host_register(&map.host_span(), "t").unwrap();
        cu.memcpy_d2h_async(&s, &reg, 0, dev.span(0, 4096), "t")
            .unwrap();
        fail("cuEventQuery");
        let _ = calls();
        assert!(cu.host_unregister(reg, "t").is_err());
        assert_eq!(undone(&calls()), 0);
        succeed_all();
        let reg = cu.host_register(&map.host_span(), "t").unwrap();
        cu.memcpy_d2h_async(&s, &reg, 0, dev.span(0, 4096), "t")
            .unwrap();
        let _ = calls();
        cu.host_unregister(reg, "t").unwrap();
        assert_eq!(undone(&calls()), 1);
        // the first registration's pages must outlive it: keep the mapping (as display::Frame does)
        std::mem::forget(map);
    }

    /// ★ F-7 / S1-05 (2026-11-14): the pageable-memory question is asked, and a driver that has
    /// no entry or refuses it answers "unknown" (`None`), never "no".
    #[test]
    fn f7_pageable_memory_access_is_asked_and_unknown_is_not_no() {
        use super::fake::{fail, succeed_all};
        let cu = Cuda::fake();
        assert_eq!(
            cu.pageable_memory_access(0),
            Some(false),
            "the fake writes 0"
        );
        fail("cuDeviceGetAttribute");
        assert_eq!(cu.pageable_memory_access(0), None, "refused: unknown");
        succeed_all();
        let old = Cuda {
            cuDeviceGetAttribute: None,
            ..Cuda::fake()
        };
        assert_eq!(old.pageable_memory_access(0), None, "no entry: unknown");
    }

    /// ★★ D-15 (2026-11-14): an import that fails half way gives back what it took, in reverse
    /// order, and nothing else: reserve refused -> release; map refused -> free the reservation,
    /// release; set-access refused -> unmap, free, release. The control (no failure) undoes nothing.
    #[test]
    fn d15_a_half_done_import_is_unwound_in_reverse() {
        use super::fake::{fail, succeed_all};
        let (cu, _ctx) = up();
        let undo = |c: Vec<Call>| -> Vec<Call> {
            c.into_iter()
                .filter(|c| matches!(c, Call::Unmap | Call::AddressFree | Call::Release))
                .collect()
        };
        let b = cu.import_and_map(0, 3, 1 << 20).expect("control");
        assert_eq!(b.len(), 1 << 20);
        assert_eq!(undo(calls()), vec![], "a successful import undoes nothing");
        let cases: [(&str, Vec<Call>); 3] = [
            ("cuMemAddressReserve", vec![Call::Release]),
            ("cuMemMap", vec![Call::AddressFree, Call::Release]),
            (
                "cuMemSetAccess",
                vec![Call::Unmap, Call::AddressFree, Call::Release],
            ),
        ];
        for (step, want) in cases {
            succeed_all();
            fail(step);
            let e = cu.import_and_map(0, 3, 1 << 20).expect_err(step);
            assert!(e.contains(step), "{e}");
            assert_eq!(undo(calls()), want, "{step}");
        }
        succeed_all();
    }

    /// ★★ D-15: a context created behind the guard is destroyed when the guard drops, exactly
    /// once, and not when it was disarmed.
    #[test]
    fn d15_a_guarded_context_dies_with_its_guard_unless_disarmed() {
        let cu = Cuda::fake();
        let g = cu.ctx_create_guarded(0).unwrap();
        let _ = calls();
        drop(g);
        assert_eq!(calls(), vec![Call::CtxDestroy]);
        let ctx = cu.ctx_create_guarded(0).unwrap().disarm();
        assert_eq!(calls(), vec![], "disarmed: not destroyed");
        cu.ctx_destroy(ctx);
        assert_eq!(calls(), vec![Call::CtxDestroy]);
    }
}

/// ★★★ The perimeter's COMPILE-TIME rows (security review 2, D-02/D-04/D-08; doc tests). Each
/// `compile_fail` block is paired with a compiling twin that differs from it in one place, so it
/// fails for the reason it names and not for a typo. On `next` before 2026-11-13 every
/// `compile_fail` row below compiled (the handles were `Copy`, `zeroed`/`view_bytes` took any
/// `T: Copy`).
///
/// A stream handle is not `Copy` (D-04):
/// ```compile_fail
/// use kf_cuda::driver_unsafe::StreamHandle;
/// fn f(s: StreamHandle) -> (StreamHandle, StreamHandle) { (s, s) }
/// ```
/// ```
/// use kf_cuda::driver_unsafe::StreamHandle;
/// fn f(s: StreamHandle) -> (StreamHandle, StreamHandle) { (s, StreamHandle::null()) }
/// ```
/// Nor is an event or a context:
/// ```compile_fail
/// use kf_cuda::driver_unsafe::{CtxHandle, EventHandle};
/// fn f(e: EventHandle, c: CtxHandle) -> (EventHandle, EventHandle, CtxHandle) { (e, e, c) }
/// ```
/// ```
/// use kf_cuda::driver_unsafe::{CtxHandle, EventHandle};
/// fn f(e: EventHandle, c: CtxHandle) -> (EventHandle, EventHandle, CtxHandle) { (e, EventHandle::null(), c) }
/// ```
/// A device buffer cannot be freed twice (D-02/D-04):
/// ```compile_fail
/// use kf_cuda::driver_unsafe::{Cuda, DevBuf};
/// fn f(cu: &Cuda, b: DevBuf) { cu.mem_free(b); cu.mem_free(b); }
/// ```
/// ```
/// use kf_cuda::driver_unsafe::{Cuda, DevBuf};
/// fn f(cu: &Cuda, b: DevBuf) { cu.mem_free(b); }
/// ```
/// A span cannot outlive the buffer it was taken from (freed while a span is held):
/// ```compile_fail
/// use kf_cuda::driver_unsafe::{Cuda, DevBuf};
/// fn f(cu: &Cuda, b: DevBuf) -> usize { let s = b.whole(); cu.mem_free(b); s.len() }
/// ```
/// ```
/// use kf_cuda::driver_unsafe::{Cuda, DevBuf};
/// fn f(cu: &Cuda, b: DevBuf) -> usize { let s = b.whole(); let n = s.len(); cu.mem_free(b); n }
/// ```
/// No safe copy takes a raw device address (D-02, S1-04):
/// ```compile_fail
/// fn f(cu: &kf_cuda::Cuda) { let _ = cu.memcpy_h2d(0x1000_u64, &[0u8; 4], "x"); }
/// ```
/// ```
/// fn f(cu: &kf_cuda::Cuda, b: &kf_cuda::driver_unsafe::DevBuf) { let _ = cu.memcpy_h2d(b.span(0, 4), &[0u8; 4], "x"); }
/// ```
/// Graph handles and modules are not `Copy`:
/// ```compile_fail
/// use kf_cuda::driver_unsafe::{GraphExecHandle, GraphHandle, ModuleHandle};
/// fn f(g: GraphHandle, h: GraphHandle, e: GraphExecHandle, m: ModuleHandle) -> [GraphHandle; 2] { [g, g] }
/// ```
/// ```
/// use kf_cuda::driver_unsafe::{GraphExecHandle, GraphHandle, ModuleHandle};
/// fn f(g: GraphHandle, h: GraphHandle, e: GraphExecHandle, m: ModuleHandle) -> [GraphHandle; 2] { [g, h] }
/// ```
/// A walker image cannot be released twice (D-04, review 2's `kernel.release(img); kernel.release(img)`):
/// ```compile_fail
/// use kf_cuda::walk::{DeviceImage, WalkKernel};
/// fn f(k: &WalkKernel, i: DeviceImage) { k.release(i); k.release(i); }
/// ```
/// ```
/// use kf_cuda::walk::{DeviceImage, WalkKernel};
/// fn f(k: &WalkKernel, i: DeviceImage) { k.release(i); }
/// ```
/// `zeroed` builds only types for which all-zero is valid (D-08):
/// ```compile_fail
/// let _: &u64 = kf_cuda::driver_unsafe::zeroed();
/// ```
/// ```
/// let _: u64 = kf_cuda::driver_unsafe::zeroed();
/// ```
/// `abi_bytes` serialises only the sealed ABI types (a padded type of the caller's is refused):
/// ```compile_fail
/// #[derive(Clone, Copy)]
/// struct Padded(u8, u64);
/// let _ = kf_cuda::driver_unsafe::abi_bytes(&Padded(0, 0));
/// ```
/// ```
/// let _ = kf_cuda::driver_unsafe::abi_bytes(&kf_cuda::abi::KfWin::default());
/// ```
pub enum PerimeterCompileRows {}
