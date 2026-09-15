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
use std::ffi::CString;

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
}

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
        let opt_sym = |name: &str| -> *mut c_void {
            let n = CString::new(name).expect("a literal with no NUL");
            // SAFETY: as above.
            unsafe { dlsym(handle, n.as_ptr()) }
        };
        let err_name = opt_sym("cuGetErrorName");

        Ok(Cuda {
            _handle: handle,
            cuInit: sym!("cuInit"),
            cuDeviceGet: sym!("cuDeviceGet"),
            cuDeviceGetCount: sym!("cuDeviceGetCount"),
            cuDeviceGetName: sym!("cuDeviceGetName"),
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
        // SAFETY: `cuInit` takes an integer and returns a status; it has no pointer
        // arguments and no aliasing obligations at all.
        self.check("cuInit", unsafe { (self.cuInit)(0) })
    }

    /// `cuDeviceGetCount`.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    pub fn device_count(&self) -> Result<i32, CudaError> {
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
        self.check("cuDeviceGet", unsafe { (self.cuDeviceGet)(&raw mut d, ord) })?;
        Ok(d)
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

    /// `cuCtxCreate_v2`. The returned pointer is opaque and is only ever handed back.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    pub fn ctx_create(&self, dev: i32) -> Result<CtxHandle, CudaError> {
        let mut ctx: *mut c_void = core::ptr::null_mut();
        // SAFETY: one live out-pointer; the driver writes an opaque handle we never
        // dereference.
        self.check("cuCtxCreate_v2", unsafe {
            (self.cuCtxCreate)(&raw mut ctx, 0, dev)
        })?;
        Ok(CtxHandle(ctx as usize))
    }

    /// `cuCtxDestroy_v2`. ⊘ Infallible by design: it runs in a `Drop` and there is nobody
    /// left to tell.
    pub fn ctx_destroy(&self, ctx: CtxHandle) {
        // SAFETY: `ctx` is a handle this binding produced and is destroyed exactly once —
        // `WalkKernel::drop` nulls its copy before returning.
        unsafe { (self.cuCtxDestroy)(ctx.0 as *mut c_void) };
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
    pub fn ctx_set_current(&self, ctx: CtxHandle) -> Result<(), CudaError> {
        // SAFETY: `ctx` came from this library's own `cuCtxCreate_v2` and is only ever handed
        // back to it. `cuCtxSetCurrent` takes the handle and affects the calling thread only.
        self.check("cuCtxSetCurrent", unsafe {
            (self.cuCtxSetCurrent)(ctx.0 as *mut c_void)
        })
    }

    /// `cuCtxSynchronize`.
    ///
    /// # Errors
    /// [`CudaError::Refused`] — including a device-side fault raised by an earlier launch,
    /// which is where an illegal access surfaces.
    pub fn ctx_synchronize(&self) -> Result<(), CudaError> {
        // SAFETY: no arguments.
        self.check("cuCtxSynchronize", unsafe { (self.cuCtxSynchronize)() })
    }

    /// `cuModuleLoadData` over a NUL-terminated PTX image. **This is where the PTX JIT runs.**
    ///
    /// # Errors
    /// [`CudaError::Refused`] — a driver too old for the PTX ISA version refuses here, by
    /// name, which is the diagnosis a reader needs.
    pub fn module_load(&self, ptx_with_nul: &[u8]) -> Result<ModuleHandle, CudaError> {
        assert!(
            ptx_with_nul.last() == Some(&0),
            "cuModuleLoadData reads until a NUL and the image handed to it has none; \
             passing it would read past the end of our own buffer"
        );
        let mut m: *mut c_void = core::ptr::null_mut();
        // SAFETY: the image is a live slice for the whole call and is NUL-terminated (checked
        // immediately above), which is the driver's documented requirement; `m` is a live
        // out-pointer.
        self.check("cuModuleLoadData", unsafe {
            (self.cuModuleLoadData)(&raw mut m, ptx_with_nul.as_ptr().cast::<c_void>())
        })?;
        Ok(ModuleHandle(m as usize))
    }

    /// `cuModuleGetFunction`.
    ///
    /// # Errors
    /// [`CudaError::Refused`], renamed to say **which** entry point is absent.
    pub fn module_function(&self, m: ModuleHandle, sym: &'static str) -> Result<Func, CudaError> {
        let c = CString::new(sym).map_err(|_| CudaError::MissingSymbol(sym))?;
        let mut f: *mut c_void = core::ptr::null_mut();
        // SAFETY: `m` is a handle this binding produced, `c` is NUL-terminated and live for
        // the call, and `f` is a live out-pointer.
        let r = unsafe { (self.cuModuleGetFunction)(&raw mut f, m.0 as *mut c_void, c.as_ptr()) };
        self.check("cuModuleGetFunction", r).map_err(|e| match e {
            CudaError::Refused { code, .. } => CudaError::Refused {
                what: "cuModuleGetFunction",
                code,
                name: format!("the committed PTX has no entry `{sym}`"),
            },
            other => other,
        })?;
        Ok(Func(f as usize))
    }

    /// `cuMemAlloc_v2` followed by `cuMemsetD8_v2` to zero. ⊘ The zeroing is not hygiene: the
    /// kernel reads its own `KfDev` state, and uninitialised device memory would make the
    /// first refresh's behaviour a function of what the previous tenant left behind.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    pub fn mem_alloc_zeroed(&self, bytes: usize, what: &'static str) -> Result<CUdeviceptr, CudaError> {
        let mut p: CUdeviceptr = 0;
        // SAFETY: one live out-pointer; `bytes` is a length the driver owns entirely.
        self.check(what, unsafe { (self.cuMemAlloc)(&raw mut p, bytes) })?;
        // SAFETY: `p` is the allocation just returned and `bytes` is exactly its length.
        self.check(what, unsafe { (self.cuMemsetD8)(p, 0, bytes) })?;
        Ok(p)
    }

    /// `cuMemFree_v2`. ⊘ Infallible: it runs in a `Drop`.
    pub fn mem_free(&self, p: CUdeviceptr) {
        if p != 0 {
            // SAFETY: `p` came from `mem_alloc_zeroed` on a live context and is freed once —
            // `DevBuf` zeroes its copy on the only other path that reclaims it.
            unsafe { (self.cuMemFree)(p) };
        }
    }

    /// `cuMemcpyHtoD_v2` from a host slice.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    pub fn memcpy_h2d(&self, dst: CUdeviceptr, src: &[u8], what: &'static str) -> Result<(), CudaError> {
        // SAFETY: `src` is a live slice for the call and the byte count passed is its own
        // length, so the driver cannot read past it; `dst` is a live allocation of at least
        // that size, which every caller sizes from the same expression.
        self.check(what, unsafe {
            (self.cuMemcpyHtoD)(dst, src.as_ptr().cast::<c_void>(), src.len())
        })
    }

    /// `cuMemcpyDtoH_v2` into a host slice.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    pub fn memcpy_d2h(&self, dst: &mut [u8], src: CUdeviceptr, what: &'static str) -> Result<(), CudaError> {
        // SAFETY: `dst` is a live, exclusively-borrowed slice and the byte count passed is
        // its own length, so the driver cannot write past it.
        self.check(what, unsafe {
            (self.cuMemcpyDtoH)(dst.as_mut_ptr().cast::<c_void>(), src, dst.len())
        })
    }

    /// `cuLaunchKernel` with **one** by-value parameter, whose bytes are `param`.
    ///
    /// ⚠ The `CUresult` is returned rather than checked, because [`Cuda::launch_raw`]'s one
    /// deliberate caller is the failed-launch probe, for which a refusal is the **expected**
    /// outcome and an error return would discard it.
    pub fn launch_raw(&self, f: Func, grid: u32, block: u32, param: &mut [u8]) -> CUresult {
        let mut p: [*mut c_void; 1] = [param.as_mut_ptr().cast::<c_void>()];
        // SAFETY: `param` is a live, exclusively-borrowed buffer holding the kernel's
        // by-value argument, and `p` is a live array of one pointer into it. The driver reads
        // exactly the parameter size the module declares — which the ABI differential test
        // pins to `param.len()`, and which `launch` asserts below.
        unsafe {
            (self.cuLaunchKernel)(
                f.0 as *mut c_void,
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

    /// As [`Cuda::launch_raw`], checked.
    ///
    /// # Errors
    /// [`CudaError::Refused`].
    pub fn launch(
        &self,
        f: Func,
        grid: u32,
        block: u32,
        param: &mut [u8],
        what: &'static str,
    ) -> Result<(), CudaError> {
        self.check(what, self.launch_raw(f, grid, block, param))
    }
}

/// An opaque CUDA context handle. ⊘ A `usize` and not a pointer, so nothing outside this file
/// can dereference it and `WalkKernel` can stay `Send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtxHandle(usize);

impl CtxHandle {
    /// Whether this handle names nothing.
    #[must_use]
    pub fn is_null(self) -> bool {
        self.0 == 0
    }
    /// The null handle.
    #[must_use]
    pub fn null() -> Self {
        CtxHandle(0)
    }
}

/// An opaque CUDA module handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleHandle(usize);

/// An opaque CUDA kernel-function handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Func(usize);

/// ★ **A `&[u8]` over one `Copy`, `#[repr(C)]` value** — the only way the safe half of this
/// crate turns a struct into the bytes `cuLaunchKernel` and `cuMemcpyHtoD_v2` want.
///
/// ⊘ It lives here rather than beside its callers for the containment rule: this is the file
/// the audit reads, and a `from_raw_parts` anywhere else would put a second file in it.
#[must_use]
pub fn view_bytes<T: Copy>(v: &T) -> &[u8] {
    // SAFETY: `v` is a live, aligned `T` borrowed for the returned lifetime, and the length
    // is exactly `size_of::<T>()`. ⚠ The caller's obligation — discharged at every call site
    // in this crate by the `#[repr(C)]` integer-aggregate types in `abi.rs` — is that `T`
    // contains no padding holding uninitialised bytes and no pointer Rust tracks. The ABI
    // differential test pins the layouts these are used with.
    unsafe { core::slice::from_raw_parts((v as *const T).cast::<u8>(), core::mem::size_of::<T>()) }
}

/// ★ Read one `Copy`, `#[repr(C)]` value out of a byte buffer the device wrote.
///
/// # Panics
/// If `bytes` is shorter than `T`. ⊘ A panic and not a truncation: a short read would decode
/// whatever follows in the buffer — usually zeros — with no marker distinguishing it from a
/// real value, which is the `dlen=0` oracle failure one layer over.
#[must_use]
pub fn read_struct<T: Copy>(bytes: &[u8]) -> T {
    assert!(
        bytes.len() >= core::mem::size_of::<T>(),
        "a {}-byte buffer cannot hold a {}-byte {}",
        bytes.len(),
        core::mem::size_of::<T>(),
        core::any::type_name::<T>()
    );
    let mut out = core::mem::MaybeUninit::<T>::uninit();
    // SAFETY: `out` is a live, aligned, writable `T`-sized region; `bytes` is at least that
    // long (asserted immediately above) and the regions cannot overlap because `out` is a
    // fresh local. `T`'s validity for an arbitrary bit pattern is the caller's obligation and
    // is discharged by every call site using a `#[repr(C)]` aggregate of integers.
    unsafe {
        core::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            out.as_mut_ptr().cast::<u8>(),
            core::mem::size_of::<T>(),
        );
        out.assume_init()
    }
}

/// ★★★★★ **AN ALL-ZERO VALUE, INCLUDING ITS PADDING** — and the padding is the whole point.
///
/// # ⊘⊘⊘ WHY THIS EXISTS, AND WHAT IT CAUGHT
///
/// `KfFormat` has interior padding (two bytes between `small_ps` and `root_align`, three after
/// `pcf_sparse`). A Rust struct literal leaves padding **undefined** — so the 216 bytes
/// [`view_bytes`] hands `cuLaunchKernel` contained whatever was on the stack, and the ABI
/// differential's byte comparison failed at **byte 58** against a `.cu` that begins its own
/// builder with `memset(&F, 0, sizeof(F))`.
///
/// ⚠ Two separate problems, and the second is the serious one:
/// 1. the differential could not compare bytes it could not predict;
/// 2. reading uninitialised padding is **undefined behaviour**, and it was being read on every
///    launch. The kernel only touches named fields, so nothing misbehaved — which is exactly
///    why it survived: a defect whose consequence is invisible is one the tests must catch.
///
/// ⇒ Every descriptor this crate hands the GPU is built on a zeroed base, as the `.cu`'s is.
#[must_use]
pub fn zeroed<T: Copy>() -> T {
    // SAFETY: the caller's obligation is that the all-zero bit pattern is a valid value of
    // `T`. Every call site in this crate is a `#[repr(C)]` aggregate of integers and integer
    // arrays (`abi.rs`), for which it is; the types contain no reference, no `NonZero`, no
    // enum with a niche, and no pointer Rust tracks.
    unsafe { core::mem::zeroed() }
}
