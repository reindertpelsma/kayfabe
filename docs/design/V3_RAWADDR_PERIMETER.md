<!-- SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later -->
# Raw-address perimeter for `v3-sec-rawaddr`: no host or device address in safe code (S1-03, S1-04, with S1-02 and S1-05)

**STATUS: IMPLEMENTED ON BRANCH `v3-sec-rawaddr`, 2026-10-04 — NOT ON MASTER.**

⊘ **Updated later on 2026-10-04, after a second review (two reviewers, 14 findings; every one fixed
on the branch, §11.7).** Read this first; it corrects the paragraph below it:
- **H1 is no longer a merge blocker.** A store import's length is now the `kf_host::RmExport`
  token's, read from the session's own record of the reservation (§2.6 note), so no caller
  supplies it. H1 (does `cuMemMap` itself refuse an over-long map?) stays as a defence-in-depth
  reading.
- **H3 and H4's kf-cuda self-test now RUN** inside `kf-gate9`, which the bench's gate runner
  already runs (§8 note). The hardware rows have still not run: no box was used.
- **§9 is re-derived at v3-broker `82f98f42`.** The earlier "applies line for line" claim was
  wrong: three v3-broker commits after `f8d74ac3` touched the console seam.
- Fixes in commits `721bc69c`..`cd93271d` (§11.7). CI was green at each except `951a84a4`, whose
  one red row (a trybuild `.stderr` written from a direct `cargo check`) `1b4ffca6` corrected.

The design below
(revised 2026-10-04 after an adversarial review; it superseded a first draft of the same day) is
implemented in commits `985ccd5d`..`cb4627f6` plus the commit that adds this file. CI is green at
`cb4627f6` (run `37165301197`: build, tests, clippy, every gate including G1–G1d and both
ratchets). **Not done, and the merge bar:** the hardware rows H1–H4 (§8) have not run; H1 is a
merge blocker. Read in this order:
- §11 is the implementation record at the tip: where each validation site is, the test and the
  mutation run against it, the counts, and every deviation from the design with its reason.
- Where the implementation departs from the text of §0–§10, a dated **⊘ Implemented** note sits
  directly above or inside the passage it corrects. The rest of §0–§10 stands as written.
- Code citations in §0–§10 are to the revisions under *Sources read* (pre-implementation).
  §11 cites the tip.

- **Sources read.**
  - The branch `v3-sec-rawaddr` at `e4fb0190`, where it was created from `origin/master`.
  - `origin/master` at `789dee9f`. It differs from `e4fb0190` only in docs: OWNER_RULINGS §Q `:451` and §R `:493`. **Merge `origin/master` into the branch first**, so code comments can cite §R.
  - `origin/v3-broker`, re-read at **`28b5b6fe`**. It moved from `65394eac` during the review, adding four commits: the console cursor through QEMU's cursor API, the XOR compose blend, relay work, and a PTX-interpreter test of `kf_compose`. Every v3-broker citation below is at `28b5b6fe`.
- **What this design closes:**
  - S1-03 and S1-04;
  - S1-09 (`view_bytes`/`zeroed`);
  - S1-10 on the import path;
  - S1-16 for the kf-cuda handles;
  - S1-02 for the v3 crates, with both a lexical gate and a type-resolved gate;
  - check (b) of S1-05.
- **What it does not close:** listed in §10.

---

## 0. What the review changed

| # | Change | Why |
|---|---|---|
| D1 | Every address-carrying type, and every block that opens an address, moves into an inline `mod raw` in `driver_unsafe.rs`. Their fields are private to `raw`. | In Rust, child modules see the parent's private items and fields. In the draft, `walk_gpu_unsafe.rs` could have built a `DevRange { addr: X }`. |
| D2 | V5 ports the kernel's own `kf_format_check` (`kf_walk.cu:1444-1478`). | Rust checks only `abi_version` and `key_perm` (`walk.rs:726-749`). `KfFormat` has pub fields and comes from safe callers (kf-qemu `device.rs:337-345`). |
| D3 | The in-flight state moves into `PinnedStage` (V4). Only a `Drained` proof clears it, and any query error poisons the walker. | `try_collect` clears `inflight` on any query error (`walk.rs:1326-1336`). A non-sticky error would let a launch rewrite a `KfLayout` that a running walk reads in place. |
| D4 | Freeing memory or closing an fd needs a successful drain. Otherwise the resource leaks and is counted. | Freeing after a failed sync races queued work. A closed fd number can be reused, and the queued host function then writes into some other file. |
| D5 | Import teardown uses three RAII guards, each one driver call per block. The four §7 reductions become required. kf-cuda goes **75 → 66**. | §R: `unsafe {}` only where Rust does not compile without it. |
| D6 | A console frame is registered while it is still owned, and leaked only after registration succeeds. Frames are capped in count and size. | In the draft, a refused registration leaked up to 1 GiB, and the worker retries on every frame. |
| D7 | `static_span` accepts only `OwnedMunmap` + `PrivateAnonymous` mappings. The D2H destination is `&ConsoleFrame`, never an arbitrary span. | MMIO mappings, unsealed shared files and reservation placements could otherwise reach the console. |
| D8 | Spans and memory-owning handles implement no `Hash`, `Ord`, `PartialOrd` or `PartialEq`. `FrameView` loses `PartialEq`/`Eq`. | A derived `Hash` passes the address to a caller-written `Hasher`. |
| D9 | G1's patterns are narrowed. G1c (perimeter re-exports) and G1d (Clippy, type-resolved) are added. The child test file is named `*_unsafe.rs`. | The draft had false positives (kf-trap `pramin.rs:113`; v3-broker `compose_kernel.rs:184`) and bypasses (`Frame::addr`, `use core::{ptr as q}`, re-exports from the perimeter). |
| D10 | G1b becomes type-based. | The name rule flagged five guest-space ABI structs on master and missed v3-broker's `BrokerHooks`. |
| D11 | kf-util's per-acquisition key becomes O(1). | `claim_site` runs on every ranked lock acquisition, including inside MMIO traps. |
| D12 | Async device operations check the stream's context. Capacity scalars are derived from ranges. | Ctx identity was checked for launches only. `KF_MAX_FRONTIER` was passed as a constant. |
| D13 | §9 is re-derived at `28b5b6fe`, and the post-merge ratchets corrected (kf-linux-raw **94**, kf-qemu **59**). | The branch moved, and the draft's arithmetic was off. |

---

## 1. Principles applied (the five constraints, as rules for this code)

**R1. No address leaves the perimeter.**

- **The perimeter in this branch:**
  - `crates/kf-cuda/src/driver_unsafe.rs`, with its inline `mod raw`;
  - two child files: `src/driver_unsafe/walk_gpu_unsafe.rs` and `src/driver_unsafe/display_gpu_unsafe.rs`;
  - `src/driver_unsafe/tests_unsafe.rs` (tests only);
  - `crates/kf-qemu/src/ffi_unsafe.rs`;
  - `crates/kf-linux-raw/src/mapping_unsafe.rs`;
  - `qemu/hw/misc/kf3/kf3.{c,h}` (§R(d), `OWNER_RULINGS.md:535`).
- **Inside the perimeter there are two tiers.**
  - Only `raw` can construct or read an address.
  - The child files can call `raw`'s validating methods and nothing more. The compiler enforces this, not a convention.
- **Code outside the perimeter may not hold, build or pass any of these:**
  - **A CUDA device address.** This includes `cuMemHostGetDevicePointer` results, which under unified addressing are host addresses of QEMU (`driver_unsafe.rs:998-1013`).
  - **A host address**, as `usize`, `u64` or `AtomicUsize`.
  - **An address turned into a string**, whether through `{:p}`, a pointer-printing `Debug`, or error text such as `ptr={:#x}` (today at `driver_unsafe.rs:1719` and `:1737`).
  - **An address hashed or ordered**, through `Hash` or `Ord` on a handle.
- **What safe code may carry instead:**
  - the opaque handles `DeviceImage`, `ConsoleFrame`, `StaticSpan` and `HostSpan`;
  - offsets and lengths into objects it names by handle.
- **Kernel-launch argument assembly is perimeter code (§R(b), `OWNER_RULINGS.md:531`).** The bytes are encoded inside `raw` into an opaque `ArgBlock`.

**R2. Remove the raw API; every boundary validates its own inputs.**

- **Deleted, not made `unsafe fn`:** the raw-address methods of `Cuda`.
- **What every function reachable from safe code checks**, when it leads into a driver call or an FFI hand-off:
  - offset plus length, with `checked_add`;
  - that the range lies inside the real allocation;
  - context identity, of the ranges **and** of the stream;
  - alignment and format;
  - the descriptor (`KfFormat`);
  - lifetime, enforced by type;
  - GPU-flight state, which is a state the perimeter owns, never one a caller promises.
- **Where the validation lives:** in perimeter files.
  - `walk.rs` and `display.rs` keep the logic: capacity planning, retry and ack decisions, report decoding, and the console ring.
  - They may repeat a check to name a refusal, but no memory-safety argument depends on them.
- **Private helpers inside the perimeter** rely only on invariants of private types that validating constructors establish. No function has an unchecked precondition, so §R(a) (`:529`) requires no `unsafe fn`.
  - The one case that would have needed it, `PinnedStage::write` while a walk is in flight, now refuses by itself (V4).

**R2b. Failure policy.** "Cannot happen" must not quietly become "freed anyway".

- **F1.** A device allocation is freed, and a completion fd closed, only after its context has **drained successfully**. If the drain fails, the resource is leaked with `mem::forget` and a counter is incremented.
- **F2.** A completion query that fails is not completion. The walker becomes `Poisoned`: every later launch, pinned write and report read is refused by name. Recovery means a new `WalkKernel`.
- **F3.** The perimeter bounds what it leaks:
  - console frames: at most 16 per `DisplayGpu`, each at most 64 MiB;
  - other leaks are counted only after a failed drain.

**R3. The `unsafe` surface goes down.**

- kf-cuda goes from 75 to **66** (§7). kf-linux-raw stays at 75 and kf-qemu at 46.
  ⊘ **Implemented 2026-10-04: kf-cuda 75 → 64** (two more than planned removed; §7, §11.4).
- The child files and `tests_unsafe.rs` contain no `unsafe`.
- The perimeter's line count grows, because §R(b) moves launch assembly into it, so the line count is ratcheted (§R(c), G4).

**R4. Compile time first, then refusal at runtime.**

- **Enforced by the compiler:**
  - The `driver_unsafe` module and the `raw` fields are private, so safe files cannot name an address, and the child files cannot forge one.
  - Handles that own memory are not `Copy` and not `Clone`.
  - Spans and handles implement no `Hash`, `Ord` or `PartialEq`.
  - `DevRange<'a>` borrows its allocation.
  - `Composer<'a>` ties `(w,h)` to the staging buffer.
  - `ConsoleFrame` is the only D2H destination.
  - `StaticSpan` is minted only from a `&'static`, private-anonymous, owned mapping.
  - Descriptors cross as `BorrowedFd`.
- **At runtime:** refusal by name (`CudaError::Refused`, or `-2` to C).
- **Panics:** only on an internal inconsistency of the perimeter, never on caller input.

**R5. CI makes a regression loud (§6).**

- the lexical gate G1, with known-positive and look-alike fixtures;
- G1b, no pointer-printing `Debug` in the perimeter;
- G1c, no pointer re-exports from the perimeter;
- G1d, Clippy `disallowed-methods`/`disallowed-types`;
- trybuild rows checked against `REQUIRED_ROWS`;
- the census of exported perimeter functions;
- the unsafe ratchet and the perimeter-line ratchet, each with a written reason;
- unit tests, each paired with the mutation that turns it red.

**S1-05 working assumption.**

- Any device address in kayfabe's own CUDA contexts may be a host address of this process. For pinned or registered memory this is certain under unified addressing; under HMM or ATS it may be true of any VMM address.
- Device ranges therefore get the same discipline as host addresses.
- Guest values reach these contexts only as offsets and geometry (§Q, `OWNER_RULINGS.md:465-472`).

---

## 2. The trusted perimeter

### 2.1 Module tree and visibility

⊘ **Implemented 2026-10-04.** As drawn, with two differences: `lib.rs` also re-exports `CUresult`
and `driver_leaks()` (the F1 leak counter), and `raw::register_console_pages` is named
`raw::console_pages` and maps the pages itself (§5 note).

```
crates/kf-cuda/src/lib.rs        mod driver_unsafe;                       // PRIVATE (today `pub mod`, lib.rs:33)
                                 pub use driver_unsafe::{CudaError, CompletionFd};   // `Cuda` no longer exported (lib.rs:38)
crates/kf-cuda/src/driver_unsafe.rs
    mod raw { … }                // INLINE. Cuda binding, Ctx, DevMem, DevRange, PinnedStage, Flight/Drained,
                                 // ArgBlock, Kernel, Stream, Event, GraphExec, import guards, KfArgs/KfWin encoder,
                                 // register_console_pages, completion_hostfn. EVERY `unsafe` block of the crate.
                                 // Fields: private to `raw`. Methods: pub(in crate::driver_unsafe) (or pub for re-exported types).
                                 // #[cfg(test)] mod tests { … }  ← the only tests that open an address (T18)
    pub(crate) mod walk_gpu_unsafe;     // file driver_unsafe/walk_gpu_unsafe.rs: WalkGpu, DeviceImage, V5–V8 (0 unsafe)
    pub(crate) mod display_gpu_unsafe;  // file driver_unsafe/display_gpu_unsafe.rs: DisplayGpu, Composer, ConsoleFrame, V9–V12 (0 unsafe)
    #[cfg(test)] mod tests_unsafe;      // file driver_unsafe/tests_unsafe.rs: pure-validator tests (0 unsafe)
```

- **What each module can see.**
  - `crate::walk` and `crate::display` are siblings of `driver_unsafe`. They see only its `pub(crate)` items and the re-exported public types.
  - The child files see `raw`'s `pub(in crate::driver_unsafe)` methods. They cannot see `raw`'s fields, its struct literals, or the functions that open addresses.
- **Naming.** `WalkDevice` is already the public device selector (`walk.rs:667-674`). The walker's perimeter type is therefore **`WalkGpu`**.
- **No `#[path]`.** The ratchet's `find` is recursive (`ci.yml:1854-1879`), and gate B's `src/*` prune allows nested files (`ci.yml:1559-1563`).
- **Dependencies.** kf-cuda gains `kf-linux-raw.workspace = true`, as v3-broker's `Cargo.toml` does. This is allowed because kf-cuda is not PURE (`scripts/ci/dependencies.py:8-9`). kf-disp becomes a dev-dependency (PURE, so allowed).

### 2.2 Handle types

⊘ **Implemented 2026-10-04 — where the handles differ from the rows below** (each a minimal
deviation, reasons in §11.5):
- `Ctx::create` takes `Option<&str>` (a PCI bus id; `None` = ordinal 0), not `WalkDevice`;
  `walk.rs` maps one to the other. `Drained` carries the id of the context it proves drained.
- Device memory is reference-counted: `DevMem` holds an `Arc` of the allocation, and graphs and
  `ArgBlock`s hold clones, so memory a recorded launch names outlives every handle to it.
  `DeviceImage` therefore holds a `DevMem`, not an `Arc<DevMem>`.
- `PinnedStage`'s flight state lives in a shared, locked cell that `raw` itself sets to
  `InFlight` when a launch reading the stage is enqueued; there is no `begin_flight()` for a
  caller to forget. `end_flight(Drained)` and `poison` are as described.
- `CompletionFd` holds an `Arc<OwnedFd>`; a stream's host signal keeps a clone, so the fd cannot
  close under a queued host function.
- `ConsoleFrame` holds its pages and a private destination record (span plus context id) rather
  than the two fields listed.

All fields are private to the module that defines the type. "Opened" means the place where the raw value reaches a driver call or C.

**Cuda** (private)
- **Defined in:** `raw` (`driver_unsafe.rs:120`).
- **Fields:** function pointers, plus `cuMemHostRegister: Option<…>`, the same field v3-broker adds at `:259`/`:438`.
- **Minted by:** `Cuda::open`, called only by `Ctx::create`.
- **Lifetime:** the process.
- **Send/Sync:** the existing `unsafe impl` at `:259` and `:261`.
- **Copy/Clone/Debug:** none, none, none.

**`Ctx = Arc<CtxInner>`** (private)
- **Defined in:** `raw`.
- **Fields:** `cu`, `raw: usize`, `device: i32`, `id: u64` (a process counter, never the `CUcontext` value), `sync_calls: AtomicU64`.
- **Minted by:** `Ctx::create(WalkDevice)`: `cuInit`, then the device by PCI bus id (ordinal 0 only for harnesses), then `cuCtxCreate`.
- **Lifetime:** the last clone to drop calls `cuCtxDestroy` (the existing block, `:571`). Every handle holds a clone, which replaces the hand-ordered teardown at `walk.rs:2110-2153`.
  - `drain() -> Result<Drained, CudaError>` is make-current followed by `cuCtxSynchronize`.
  - `Drained` is a zero-sized proof that only `drain` and `Event::query()==Done` can mint.
- **Send/Sync:** automatic.
- **Copy/Clone/Debug:** cloned only internally; manual Debug (`id`).

**`DevMem`** (private)
- **Defined in:** `raw`.
- **Fields:** `ctx`, `addr: u64`, `len: u64`, and `kind: Alloc | Import { map: VaMapping, res: VaReservation }`.
- **Minted by:**
  - `alloc_zeroed(&Ctx, len)`: `len ≥ 1`, then `cuMemAlloc`, then `cuMemsetD8` over exactly `len`.
  - `import(&Ctx, BorrowedFd<'_>, len)` (V2), using the guards in the next row. No error text contains an address.
- **Lifetime (`Drop`):** `ctx.drain()`.
  - On `Ok`: `cuMemFree`, or the guards drop in order: unmap, then address free.
  - On `Err`: `forget`, and count it (F1).
- **Send/Sync:** automatic.
- **Copy/Clone/Debug:** no, no, manual (`len`, `kind`).

**Import guards** (private)
- **Defined in:** `raw`.
- **Types and their release:**
  - `ImportHandle(u64)`: `cuMemRelease`. It drops at the end of `import`, because the mapping keeps the memory alive.
  - `VaReservation { addr, len }`: `cuMemAddressFree`.
  - `VaMapping { addr, len }`: `cuMemUnmap`.
- **Minted by:** each step of `import`.
- **Lifetime:** each `Drop` is **one block, one driver call**. Any step that fails unwinds the guards already made.
- **Copy/Clone/Debug:** no, no, no.

**`DevRange<'a>`** (private)
- **Defined in:** `raw`.
- **Fields:** `addr`, `len`, `ctx_id`, `PhantomData<&'a DevMem>`.
- **Minted by:** only `DevMem::range` (V1) and `PinnedStage::device_range` (V4). The children cannot construct one (D1).
- **Lifetime:** it borrows its allocation. Async safety comes from `DevMem::drop` draining first.
- **Copy/Clone/Debug:** Copy (it owns nothing); never `pub`; no `Debug`.

**`ArgBlock`** (private)
- **Defined in:** `raw`.
- **Fields:** `bytes: [u8; 264]`, `ctx_id`.
- **Minted by:** only `raw::encode_kf_args(&KfArgsRanges<'_>, &KfFormat, npdb, key_perm)`. Every pointer field of the encoded struct is a `DevRange` or a `PinnedStage` region of the same `Ctx`.
- **Copy/Clone/Debug:** no, no, none.

**`PinnedStage`** (private)
- **Defined in:** `raw`. It replaces `PinnedBuf` (`:1344-1403`).
- **Fields:** `ctx`, `host: usize`, `total`, `regions: [(off, len); 8]`, `dev: [u64; 8]`, and `flight: Idle | InFlight | Poisoned(String)`.
- **Minted by:** `new(&Ctx, &RegionTable)`.
  - The offsets come from today's `PinLayout::for_cfg` (`walk.rs:483-509`), moved here.
  - They must be monotonic, 64-aligned, disjoint and within `total`.
  - Then `cuMemAllocHost`, a zero-fill, and one `cuMemHostGetDevicePointer`.
- **V4:** `write(&mut self, Region, off, bytes)` and `read(&self, Region, off, n)` refuse unless `Idle`. Host-writable regions are exactly `{Lay, Pdbs, Slots, Ack, AckCode}`; `{Hdr, Rpdb, Rrun}` are host-read-only. `begin_flight()` is called after an enqueue. `end_flight(Drained)` is the only way back to `Idle`. `poison(why)` is final.
- **Lifetime:** never freed individually. `cuCtxDestroy` reclaims it, and the stage holds a `Ctx` clone.
- **Send/Sync:** automatic. Writers need `&mut`.
- **Copy/Clone/Debug:** no, no, manual (`total`, `flight`).

**`Stream`, `Event`, `Module`, `Kernel`, `GraphExec`** (private)
- **Defined in:** `raw`.
- **Fields:** `ctx` and `raw`. `Kernel` also holds `name` and `sig: &'static [ArgKind]`, taken from `KERNEL_SIGS`, which is pinned to the PTX (T4).
- **Minted by:** `create`, `Module::load`, `Module::kernel(name)`, and graph capture.
- **Lifetime:** `Drop` destroys streams and events (`:872`, `:893`) and the exec, then the graph (`:1277`, `:1285`). `cuCtxDestroy` reclaims modules.
- **Copy/Clone/Debug:** no, no, **no `Debug`**, because `CUcontext` and `CUfunction` values are pointers into libcuda's heap.

**`CompletionFd`** (pub type)
- **Defined in:** `raw`, re-exported (`:1405`).
- **Fields:** `OwnedFd`.
- **Minted by:** `new()`.
- **Lifetime:** the owner's `Drop` drains before the fd field closes. If the drain fails, the fd is `forget`-ed (F1), because a queued host function would otherwise write into a reused fd number. `raw()` (`:1432`) and `launch_host_signal` (`:945`) become private.
- **Copy/Clone/Debug:** no, no, derived (fd only).

**`WalkGpu`** (`pub(crate)`)
- **Defined in:** `walk_gpu_unsafe.rs`.
- **Fields:**
  - ctx, kernels, stream, events, graph;
  - every `DevMem`, the `PinnedStage` and the done fd;
  - `store: Option<DevMem>`, `last_window: Option<WindowRef>` and the shape.
- **Minted by:** `bring_up(&WalkCfg, &KfFormat, WalkDevice)` (V5).
- **Lifetime (`Drop`):** `ctx.drain()`; on failure, `forget` the done fd (F1). Then the fields drop.
- **Send/Sync:** automatic (`WalkKernel` is `Send`).
- **Copy/Clone/Debug:** no, no, manual.

**`DeviceImage`** (pub, `kf_cuda::walk`)
- **Defined in:** `walk_gpu_unsafe.rs`.
- **Fields:** `Arc<DevMem>`.
- **Minted by:** `WalkKernel::upload` → `WalkGpu::upload` (non-empty).
- **Lifetime:** `Drop` releases one reference. `WalkGpu` keeps another while the image is the in-flight or the last window, which is what lets the capacity retry run (`walk.rs:1458`, `:1473`).
- **Send/Sync:** automatic.
- **Copy/Clone/Debug:** **no, no** (§R gate 5); manual Debug. **No `Hash`, `Ord` or `PartialEq`.**

**`DisplayGpu`** (pub)
- **Defined in:** `display_gpu_unsafe.rs`.
- **Fields:**
  - ctx, stream, done fd;
  - `compose: Result<Kernel, String>`;
  - `store: Option<DevMem>`, `staging: Option<DevMem>`;
  - `frames_minted: u32`, `frame_bytes_minted: u64`.
- **Minted by:** `bring_up_on(bdf)`.
- **Lifetime (`Drop`):** drain, applying F1 to the fd, then the fields drop.
- **Send/Sync:** automatic. It is moved into `WorkerInit` (kf-qemu `display.rs:772`, `:865-880`).
- **Copy/Clone/Debug:** no, no, manual.

**`Composer<'a>`** (pub)
- **Defined in:** `display_gpu_unsafe.rs`.
- **Fields:** `gpu: &'a DisplayGpu`, `w`, `h`.
- **Minted by:** `DisplayGpu::compose_begin(&mut self, w, h)` (V9).
- **Lifetime:** the exclusive borrow means no second `compose_begin` (which may grow the staging buffer) can run while it lives.
- **Copy/Clone/Debug:** no, no.

**`ConsoleFrame`** (pub)
- **Defined in:** `display_gpu_unsafe.rs`.
- **Fields:** `pages: &'static MappedRegion`, `span: StaticSpan`, `ctx_id`.
- **Minted by:** only `DisplayGpu::console_frame(len)` (V12). It is the **only** destination the GPU copies into host memory.
- **Lifetime:** **never unmapped.** It is registered while still owned, and leaked only once the registration succeeded (§5).
- **Send/Sync:** `!Send`, because `MappedRegion` is not `Sync` (`mapping_unsafe.rs:413-432`). Frames are made and kept on the display worker (kf-qemu `display.rs:1188-1255`).
- **Copy/Clone/Debug:** no, no, manual (`len`). No `Hash`, `Ord` or `PartialEq`.

**`StaticSpan`** (pub, kf-linux-raw)
- **Defined in:** `mapping_unsafe.rs`.
- **Fields:** `span: HostSpan`.
- **Minted by:** only `MappedRegion::static_span(&'static self) -> Option<StaticSpan>` (V15). It returns `None` unless the mapping is `Disposition::OwnedMunmap` with backing `PrivateAnonymous` (a new recorded `BackingKind` field). That excludes MMIO `DeviceFile` mappings, unsealed `SharedFile` mappings and `InsideReservation` placements.
- **Lifetime:** valid for the process.
- **Send/Sync:** automatic, through `HostSpan`'s impls (`:938`, `:940`).
- **Copy/Clone/Debug:** Copy; manual Debug (`len`). **No `Hash`, `Ord` or `PartialEq`.**

**`HostSpan`**
- **Defined in:** `mapping_unsafe.rs:930`.
- **Fields:** `base`, `len`.
- **Minted by:** the existing `within` (`:945-960`), `:1089`, `window_unsafe.rs:177`, and a new `MappedRegion::host_span(&self)` (through `within`, never with length 0).
- **Lifetime:** unchanged; the `as_ptr` contract is widened (§5).
- **Copy/Clone/Debug:** Copy. Debug becomes manual (`len` only). No `Hash`, `Ord` or `PartialEq`.

### 2.3 Every function safe code can call that leads into `unsafe`, and what it checks

Nothing is left to the caller in any row. What the code relies on, which is not a caller precondition, is listed in §2.6.

**`WalkGpu::bring_up(cfg, fmt, on)`: V5.**
- **The format.** A port of `kf_format_check` (`kf_walk.cu:1444-1478`):
  - `abi_version == KF_ABI_VERSION`; `table_version` is VER2 or VER3 (VER3 accepted, w826);
  - `first_dir < KF_DIRS`, and the deepest directory slot is active;
  - every slot below `first_dir` is active and every slot above it is inactive;
  - for active directories: `1 ≤ entries ≤ KF_MAX_ENT`, a power of two; `entry_bytes` ∈ {8, 16}; `va_lo < 64`; table bytes a power of two; `leaf_ps` is `KF_PS_NONE` or at most 3;
  - the deepest directory has 16-byte entries;
  - `small_entries` and `big_entries` are each in `1..=KF_MAX_ENT`;
  - the big/small stride is in `(0, 4]` and both tables cover the same VA range;
  - small and big table bytes, and `root_align`, are powers of two;
  - `1 ≤ ap_bits ≤ 2`, and `ps_log2` lies in `[12, 40]`.
- **The configuration:**
  - `key_perm ⊆ KFWR_RF_KEY_PERM_ALL`;
  - `1 ≤ walk_pool ≤ KF_MAX_SCRATCH/4`; `slot_pool ≥ 1`; `1 ≤ max_slots ≤ KF_MAX_SLOTS`;
  - `runs_per_pdb`, `run_capacity` and `pdb_capacity` are each at least 1;
  - every byte size goes through `checked_mul`.
- **The requirement table (§4.3).** It is written from the kernels' side and checked against the sizes actually allocated. `KfDev` is encoded from the same validated shape.

**`WalkGpu::import_store(BorrowedFd, bytes)`**
- A second import is refused; then V2.

**`WalkGpu::store_write(off, &[u8])` and `store_read(off, &mut [u8])`**
- Refused when no store is imported; then `DevMem::range(off, len)` (V1).

**`WalkGpu::upload(&[u8]) -> DeviceImage`**
- Non-empty input; an allocation of exactly that size; V1.

**`WalkGpu::image_read` and `image_write(&DeviceImage, off, buf)`**
- Refused when `img.ctx_id != self.ctx.id`; then V1.

**`WalkGpu::launch(Window, &WalkStage)`: V6.**
- **Flight state.** `stage.flight` must be `Idle`, otherwise the call is refused or reports `Poisoned`.
- **The window:**
  - `Store` requires an import.
  - `Image(&DeviceImage)` requires the same context and `len > 0`.
  - `Same` requires a retained window.
  - `win.len` and `win.span` are taken from the `DevMem`, never from a caller.
- **The stage:**
  - `npdb = pdbs.len() == slots.len() ≤ KF_MAX_PDB`;
  - every slot is below `max_slots`, with no duplicates.
- **Every layout row for `t < KF_MAX_PDB`, not only `t < npdb`,** in u64 arithmetic:
  - `walk_off + walk_cap ≤ walk_pool`;
  - `prev_off + prev_cap ≤ walk_pool`;
  - `walk_cap ≤ runs_per_pdb` and `prev_cap ≤ runs_per_pdb`.

  Because `4·walk_pool ≤ KF_MAX_SCRATCH` and `iscratch = 3·walk_pool` words, `kf_scr` and `kf_iscr` (`kf_walk.cu:931-938`) and their users (`:975-980`, `:1185`, `:1224-1232`) stay inside.
- **Every slot row for `s < KF_MAX_SLOTS`:** `slot_off + slot_cap ≤ slot_pool` and `slot_cap ≤ runs_per_pdb`.
- **The ack.** Write `min(codes.len(), run_capacity)` bytes into `AckCode`, with `nrun = codes.len()`; the kernel commits only when it equals `run_count`, `kf_walk.cu:1215`. Also `nreset ≤ KF_MAX_RESET`.
- **The writes go through V4.** `ArgBlock` is built in `raw` from owned ranges. Then a graph setter or an enqueue (V3), then `begin_flight()`.

**`WalkGpu::poll()`**
- An event query or a drained fd yields `Drained`, which leads to `end_flight`.
- Any query error leads to `poison()` (F2).

**`WalkGpu::read_report()`**
- V4 refuses unless `Idle`.
- The counts are clamped to `pdb_capacity` and `run_capacity` here, inside the perimeter. Today the clamp is in safe code (`walk.rs:1349-1353`).

**`WalkGpu::move_slot(from: Region, to: Region)`: V7.**
- Refused unless `Idle`.
- `off + cap ≤ slot_pool` for both regions, in u64.
- `from.cap ≤ to.cap`, and the regions are disjoint.
- Then `com.copy_async` (V1 and V1b).

**`WalkGpu::read_walk_region(off, cap)`: V8.**
- Refused unless `Idle`; `off + cap ≤ walk_pool`; then V1.

**`WalkGpu::probe_oversized_block()`**
- Refused unless `Idle`.
- Uses an `ArgBlock` with an empty window, and V3 with the block limit waived for this one call (block 2048, `walk.rs:2093-2108`).

**`WalkGpu` queries**
- `gpu_done_now`, `completion_fd`, `ctx_sync_calls`, `submits_as_graph`, `graph_param_updates`, `make_current`: no inputs.

**`DisplayGpu::import_store(BorrowedFd, bytes)`, `read_store`, `write_store`, `zero_store(off, len)`**
- A second import is refused; then V2; then V1, with a checked `u64 → usize` conversion.

**`DisplayGpu::console_frame(len) -> ConsoleFrame`: V12.**
⊘ *Implemented 2026-10-04:* the map, register and leak steps run inside `raw::console_pages`, so the
child file never holds an unregistered mapping; the order is the one below.
- `1 ≤ len ≤ 64 MiB`, rounded up to whole host pages.
- `frames_minted < 16`.
- Map `PrivateAnonymous`, `RW`, `WriteBack`, **owned**.
- `register_console_pages(&ctx, region.host_span())`. On failure the region drops and nothing was registered.
- On success: `Box::leak`, then `static_span()`, which must be `Some`; if it is not, that is an internal-inconsistency panic.

**`DisplayGpu::compose_begin(w, h) -> Composer`: V9.**
- `1 ≤ w, h ≤ 16384`, and `n = 4·w·h` computed in u64.
- The staging buffer grows when it is smaller; the old `DevMem` is dropped, which drains first.
- Then `fill_async(0, n)` (V1 and V1b).

**`Composer::layer(&ComposeLayer)`: V10.**
- `compose_layer_fits` (today's `display.rs:122-154`, moved), checked against the composer's own `(w,h)`. On v3-broker it also checks `flags & !COMPOSE_FLAGS == 0` (v3-broker `display.rs:188`).
- The source is `store.range(l.src, l.extent)` and the destination is `staging.range(0, 4wh)`.
- `dpitch = w·4` cannot overflow because `w ≤ 16384`; this also closes the `u(w*4)` wrap at `display.rs:330`.
- Rows ≤ 16384; block 256; V3.

**`Composer::finish(self, &ConsoleFrame)`: V11.**
- `frame.ctx_id == ctx.id`.
- `4wh ≤ frame.len()`; and `4wh ≤ staging.len` holds by construction.
- Then `staging.range(0, 4wh).copy_to_console(&stream, frame)` (V1b), then the host signal.

**`DisplayGpu::selftest_compose(surface, &l, w, h)`**
- `l.extent ≤ surface.len()`; then V9 and V10.
- The scratch buffer is a `DevMem`, so it is drained before it is freed on **every** path. Today an error path frees it without a sync (`display.rs:392`).
- The read-back is a synchronous `read` of the staging buffer; no console frame is used.

**`ConsoleFrame::read(off, n) -> Result<Vec<u8>, CudaError>`**
- `MappedRegion::read_into`, which is bounds-checked (`mapping_unsafe.rs:703-720`).

**`MappedRegion::static_span(&'static self)` and `MappedRegion::host_span(&self)`**
- V15: the `'static` borrow, plus the backing and disposition rules. `host_span` mints through `HostSpan::within`.

**kf-qemu `kf3_display_frame` → `check_frame(&FrameView)`: V13.**
- `span` is `None` → return `-1`, meaning no host frame (for example a frame published to VRAM only).
- The format is 1, 2, 4 or 5 (4 bytes per pixel) or 3 (2 bytes per pixel).
- `w, h ≥ 1`; `w`, `h` and `stride` are each at most `i32::MAX`.
- `stride % 4 == 0` and `u64(w)·bpp ≤ stride`.
- `u64(stride)·u64(h) ≤ span.len()`. The whole `stride·h` is required because QEMU's D-Bus listener sends exactly `stride × height` bytes (QEMU `ui/dbus-listener.c:720-722`).

**C `kf3_frame_ok(f, bpp)` in `kf3.h`: V14.**
- The same predicate, computed in `uint64_t`.

### 2.4 Where each raw address is opened

All openers are inside `raw` in `driver_unsafe.rs`, except the last two groups.

- **Device addresses.** `DevRange.addr` is read only inside these blocks:
  - `DevMem::write` (`cuMemcpyHtoD`) and `read` (`cuMemcpyDtoH`);
  - `fill` (`cuMemsetD8`) and `fill_async` (`cuMemsetD8Async`);
  - `copy_async` (`cuMemcpyDtoDAsync`);
  - `copy_to_console` (`cuMemcpyDtoHAsync`, which opens the frame's span in the same block);
  - `encode_kf_args` and `Arg::Ptr` packing, used by `Kernel::launch` and `GraphExec::set`;
  - `DevMem::drop` and the three import guards.
- **The pinned stage.** `PinnedStage::{read, write}` (the existing blocks, `:1375` and `:1392`) and the one `cuMemHostGetDevicePointer` call in `PinnedStage::new`.
- **Host spans.** `HostSpan::as_ptr()` is called in exactly three blocks:
  - `register_console_pages` (`cuMemHostRegister_v2`);
  - `copy_to_console`;
  - kf-qemu `kf3_display_frame`, inside the existing `*out =` block.
- **Unchanged openers:** `kf3_bar_ram` (`ffi_unsafe.rs:365-370`) and `kf3_usermode_view` (`:405-416`).
- **Driver object handles** (`CUcontext`, `CUstream`, …) are stored as `usize` in `raw` and opened only in their owners' methods.

### 2.5 The audit surface: validation sites

| Site | Function | Where | At the tip (`cb4627f6`; kf-cuda paths under `src/`) |
|---|---|---|---|
| V1 | `DevMem::range` | `raw` | `driver_unsafe.rs:614` `sub_range`; `:1003` `DevMem::range` |
| V1b | async ops: `stream.ctx_id == range.ctx_id` | `raw` | `driver_unsafe.rs:628` `ctx_matches` |
| V2 | `DevMem::import` | `raw` | `driver_unsafe.rs:620` `import_len`; `:908` `DevMem::import` |
| V3 | `check_args` | `raw` | `driver_unsafe.rs:1762` `check_args`; `:1540` `KERNEL_SIGS` |
| V4 | `PinnedStage` region table, writability and flight state | `raw` | `driver_unsafe.rs:2178` `stage_layout`; `:2222` `Flight::step`; `:2267` `PinnedStage` |
| V5 | `WalkGpu::bring_up`: format check, shape, requirement table | `walk_gpu_unsafe.rs` | `walk_gpu_unsafe.rs:80` `kf_format_check`; `:184` `validate_cfg`; `:292` `check_requirements` |
| V6 | `validate_stage` and the window | `walk_gpu_unsafe.rs` | `walk_gpu_unsafe.rs:329` `validate_stage` |
| V7 | `move_slot` | `walk_gpu_unsafe.rs` | `walk_gpu_unsafe.rs:389` `validate_move` |
| V8 | `read_walk_region` and the `read_report` clamps | `walk_gpu_unsafe.rs` | `walk_gpu_unsafe.rs:412` `walk_region_fits`; `:423` `report_counts` |
| V9 | `compose_begin` | `display_gpu_unsafe.rs` | `display_gpu_unsafe.rs:178` `composition_bytes` |
| V10 | `compose_layer_fits` | `display_gpu_unsafe.rs` | `display_gpu_unsafe.rs:143` `compose_layer_fits` |
| V11 | `Composer::finish` | `display_gpu_unsafe.rs` | `display_gpu_unsafe.rs:118` `finish_fits` |
| V12 | `console_frame` | `display_gpu_unsafe.rs` | `display_gpu_unsafe.rs:190` `console_frame_bytes`; `driver_unsafe.rs:2653` `console_pages` |
| V13 | `check_frame` | kf-qemu `ffi_unsafe.rs` | `ffi_unsafe.rs:532` `check_frame_geometry`; `:565` `check_frame` |
| V14 | `kf3_frame_ok` | `kf3.h` | `kf3.h:58` `kf3_frame_ok` |
| V15 | `static_span` | `mapping_unsafe.rs`; compile-time plus backing rules | `mapping_unsafe.rs:894` `static_span_allowed`; `:885` `static_span` |

- That is 15 runtime sites and 1 compile-time site.
- Every runtime site is a **pure function over numbers or states**: `sub_range(len, off, n)`, `kf_format_check(&KfFormat)`, `validate_stage(&Shape, &WalkStage)`, `compose_layer_fits(&ComposeLayer, w, h)`, `check_args(sig, &[ArgKind])`, `Flight::step(state, event)` and `check_frame(..)`. The handle method calls it first.
- So every site is unit-tested without a GPU (§8).

### 2.6 What the code relies on: not caller preconditions, each with a test row

- **Res-1: the import length.**
  - ⊘ **Implemented 2026-10-04 (second review, finding 2): the "fallback" below is now the design,
    and H1 is not a merge blocker.** `kf_host::RmExport { fd: OwnedFd, bytes: u64 }`
    (`crates/kf-host/src/lib.rs:398`), private fields, is minted only by
    `HostRm::export_store(&Reservation)` (`:1987`). `reserve_gpga` records the length it asked RM
    for in the session's object record (`Objects::stores`, `:352`, dropped with the subtree on
    free, `:369`), and `export_store` reads it from there: a handle this session did not reserve
    as a store is refused, and no caller can pair a handle with a length. `DevMem::import`
    (`crates/kf-cuda/src/driver_unsafe.rs:932`), both `import_store`s and
    `WalkKernel::import_store` take only `&RmExport`. **It lives in kf-host, not kf-linux-raw** as
    written below: only kf-host knows the length, and a kf-linux-raw type would need a public
    constructor any crate could call with any length. kf-cuda therefore depends on kf-host (it is
    not PURE). Tests: kf-host `an_export_length_comes_only_from_the_sessions_record`; trybuild
    `import_takes_an_rm_export` and `rm_export_is_minted_only_by_kf_host`. The import handle is
    now held until after the unmap (finding 14; `AllocKind::Import`, `driver_unsafe.rs:817`), as
    the pre-perimeter code held it for the process.
  - CUDA cannot report the size of an imported RM object. That `len` is no larger than the object is enforced by `cuMemMap`, which refuses an `offset + size` past the allocation. This is UNVERIFIED on the target drivers; hardware row **H1 is a merge blocker**.
  - The caller's `len` is RM's own `fb_length` (kf-qemu `device.rs:346-358`).
  - **Fallback if H1 fails:** `import` takes a typed `RmExport { fd: OwnedFd, bytes: u64 }` token. kf-host's export path mints it, carrying the size RM allocated, and it is defined in kf-linux-raw next to the export type (`chardev_unsafe.rs:525-540`). A caller-supplied `u64` is then no longer accepted.
- **Res-2: bounds inside the kernels.**
  - The walker relies on I2, the window (`kf_walk.cu:409-447`, with the `KF_BREAK_BOUNDS` known-positive), and on I3, the report clamps (`kf_walk.cu:1147-1175`, `:1213-1228`).
  - The compose kernel has no store bound of its own. It relies on V10, on T12, and on v3-broker's PTX interpreter, once it merges.
  - All of these are host-memory obligations (§Q). This branch does not change the `.cu` or the PTX.
- **Res-3: content races.** Frame pixels, and pinned bytes written by DMA, can race. The result is wrong content, never a memory error. The pages are leaked and V4 gates reads.
- **Res-4: the current-context discipline.**
  - Entry points call `make_current`. `DevMem::drop` makes its own context current on the dropping thread and does not restore the previous one; every entry point re-makes its own.
  - A wrong current context gives a driver refusal. Under F2, a refusal during polling **poisons** the walker; it never clears the flight.

---

## 3. Per site: before → after

Disposition codes:
- **R** = removed.
- **H** = converted to a handle.
- **P** = moved into the perimeter.
- **O** = out of scope (§10).

| Site (`e4fb0190`) | Before | After | Disposition |
|---|---|---|---|
| `driver_unsafe.rs:41` | `pub type CUdeviceptr = u64` in a `pub mod` | `type DevAddr = u64` private to `raw`; `mod driver_unsafe` private | P |
| `:658-680` `mem_alloc_zeroed` / `mem_free` | raw `u64` out; free of any `u64` | `DevMem::alloc_zeroed` / `Drop` (F1) | H |
| `:684-716` `memcpy_h2d` / `memcpy_d2h` | device side unchecked ("every caller sizes…", `:694-696`) | `DevMem::write` / `read` through V1 | H |
| `:792-842` `memcpy_d2d_async`, `memset_d8`, `memset_d8_async` | "callers size them" | `DevMem::copy_async` / `fill` / `fill_async` through V1 and V1b | H |
| `:615-640` `module_load`, `module_function` | public; take any PTX | private `Module::load` / `kernel(name)`, committed PTX only | P |
| `:720-747` `launch_raw`; `:1293-1302` `launch` | arbitrary parameter bytes | **removed**; folded into `Kernel::launch` (V3) | R |
| `:749-784` `launch_args`; `:1204-1244` `graph_exec_kernel_set` | parameter count and sizes unchecked | `Kernel::launch(&Stream, grid, block, shm, &[Arg])` / `GraphExec::set`, through `check_args` (V3). `Arg` is `Block(&ArgBlock)`, `Ptr(DevRange)`, `CapOf(DevRange, elem_bytes)`, `U32` or `I32`. | H |
| `:998-1014` `pinned_device_ptr` | `off < len` only; address returned to safe code | `PinnedStage::device_range(Region)` (V4) | H |
| `:963-990` `pinned_alloc` / `pinned_free`; `:1344-1403` `PinnedBuf{ptr: usize}`, `addr()` | address handed to safe files; read/write bounded by the whole buffer; "only between walks" left to callers | `PinnedStage` with per-region bounds, writability and flight state (V4); `addr()` and `pinned_free` **removed** | H / R |
| `:1026-1045` `memcpy_h2d_async` | dead code | **removed** | R |
| `:1056-1074` `memcpy_d2h_async` | device side unchecked; used by compose | `DevRange::copy_to_console(&Stream, &ConsoleFrame)`: `n ≤ range.len`, `n ≤ frame.len()`, ctx checks | H |
| `:1686-1740` `import_and_map` | `i32` fd (a negative fd becomes 0, `:1703`); raw `u64` out; leaks on failure; `ptr=` in error text | `DevMem::import(BorrowedFd, len)` with RAII guards; no address in errors | H |
| `:1618-1627`, `:1742-1811` `ExportedAllocation` / `export_device_allocation` | dead public surface | **removed** | R |
| `:1560-1597` `view_bytes` / `zeroed`; `walk.rs:680-696` `param_bytes<T: Copy>` / `bytes_of<T: Copy>` | safe generics that put the obligation on callers (S1-09) | **removed**. `offset_of!` encoders (`encode() -> [u8; N]`, padding zero by construction) for `KfFormat`, `KfDir`, `KfField`, `KfDev`, `KfLayout`, `KfAck` in safe `abi.rs`, beside `report_codec!` (`:721-765`); `KfArgs`/`KfWin` encoded in `raw` | R |
| `:1307-1337`, `:1510-1527` `Copy` handles with public destroy functions and derived `Debug` | double destroy and stale use (S1-16); `{:?}` prints libcuda pointers | owning, non-`Copy`, private handles with no `Debug` | H |
| `:945-957`, `:1432` `launch_host_signal`, `CompletionFd::raw` | public; can write to a recycled fd | private; the owner drains before close, and leaks the fd if it cannot (F1) | P |
| `:1477-1482` `completion_hostfn` | `pub(crate)` | private to `raw` | P |
| `:343-362`, `:434`, `:547` (`sym!`'s own `dlsym`, the `opt_sym` closure, the `cuGetErrorName` transmute, the `device_name` `CStr::from_ptr`) | four avoidable `unsafe` blocks | `sym!` resolves through `sym_or_null`; the closure folds into it; `cuGetErrorName` uses `opt!`; `device_name` uses `CStr::from_bytes_until_nul` over the `[u8; 128]`, which is also bounded if the driver omits the NUL. **Required**, not optional (§R). | R |
| `walk.rs:524-527` `DevBuf{ptr}`; `:827-920` allocations | raw device addresses in a safe file | `DevMem` / `PinnedStage` fields of `WalkGpu` | P |
| `walk.rs:726-749` bring-up check | `abi_version` and `key_perm` only | V5: the `kf_format_check` port plus the shape | P (check) |
| `walk.rs:1042-1088` `args_for`; `abi.rs:246-257`, `:426-464` `KfWin`/`KfArgs` with `pub u64` fields | safe code assembles 15 device pointers | `raw::encode_kf_args → ArgBlock`. `KF_ARGS_LAYOUT` and `KF_WIN_LAYOUT` (size plus `(field, offset)`) are defined in `raw` and re-exported through `abi` for the ABI differential (`tests/walk_abi_matches_the_cu.rs:170-187`, `:775-800`) | P |
| `walk.rs:1718-1906` `run_parallel`, `:1603-1716` `enqueue_walk`/`kl`/`record`, `:1006-1040` `build_graph`, `:1564-1598` `update_graph`, `:533-546` `GraphKernel.params`, `:985-1003` `warm_up` | pointer arithmetic and parameter bytes in safe code; capacity scalars passed as constants (`u(KF_MAX_FRONTIER as u32)`) | moved into `WalkGpu`. `p.nfr.ptr + src*4` becomes `p.nfr.range(src*4, 4)?`; capacity scalars become `Arg::CapOf(range, elem)`, derived from the buffer | P |
| `walk.rs:1320-1340` `try_collect` | any query error clears `inflight` | `WalkGpu::poll`; `Drained` returns to `Idle`, an error poisons (F2) | P |
| `walk.rs:1440-1456` slot growth | `com.ptr + to.off*32 …` | `WalkGpu::move_slot(from, to)` (V7) | P |
| `walk.rs:1948-2014` `import_store`/`store_at`/store fields | bound in a safe file; `i32` fd | `WalkGpu` store `DevMem`; `WalkKernel::import_store(BorrowedFd<'_>, u64)` | P / H |
| `walk.rs:2023-2041` `debug_walk_runs` | `walk.ptr + reg.off*32` | `WalkGpu::read_walk_region` (V8) | P |
| `walk.rs:2158-2176` `DeviceImage{ptr,len}` (`Copy`); `:2080` `release` | use after `release`, double free, use with another context | `DeviceImage(Arc<DevMem>)`, not `Clone`; `release` **removed** (callers `drop`) | H / R |
| `walk.rs:511-517` `InFlight.gpga` (derived `Debug`) | raw address kept for the retry | `Window::Same` (`WalkGpu` keeps the `Arc`) | R |
| `walk.rs:84-104` buffer constants | hand-mirrored, unpinned | pinned against `kf_walk.cu` `#define`s and `sizeof(KfEnt)`/`sizeof(KfSum)` by an ABI test (T6), and checked at bring-up by the requirement table | P |
| `walk.rs:1162-1194` + `capacity.rs:225-283` `KfLayout` | only the safe planner keeps regions inside the pools | the planner is unchanged (logic); V6 re-validates every row | P (check) |
| `display.rs:27-40`, `:435-480` store/staging tuples, `at()`, store read/write/zero | bound in a safe file | `DevMem` fields of `DisplayGpu`; V1 | P |
| `display.rs:239-341` `compose_begin`/`compose_layer`/`compose_finish`/`launch_compose`; `:122-154` `ComposeLayer::check` | `(w,h)` not tied to the staging buffer; the check in a safe file | `Composer<'a>` (V9–V11); the check moves. `ComposeLayer` stays as plain data in `display.rs` | P / H |
| `display.rs:44-76`, `:208-232` `Frame`, `addr()`, `frame()`, `release_frame()` | the console address as a public `usize`; freed by `cuCtxDestroy` | `ConsoleFrame` (register-then-leak, `StaticSpan`); `addr`, `frame` and `release_frame` **removed** | H / R |
| `display.rs:349-394` `selftest_compose` | frees without a sync on error paths; uses a pinned frame | perimeter; scratch `DevMem` (drained drop); reads the staging back | P |
| `display.rs:408-432` `import_store(fd: i32, ..)` | raw fd | `BorrowedFd<'_>` | H |
| kf-qemu `display.rs:417-424` `FrameSlot.addr: AtomicUsize`; `:426-442` `FrameView.addr: usize`, `derive(PartialEq, Eq)` | host address as an integer | `FrameSlot.frame: AtomicU32` (a span id, `NONE = u32::MAX`); `FrameView { span: Option<StaticSpan>, id: u32, width, height, stride, format, serial }`, deriving only `Clone, Copy` with a manual `Debug`; the test at `:2511` uses `is_none()` | H |
| kf-qemu `display.rs:2003-2030` `completed()` | `addr: f.addr(), stride: w*4` | publishes `(slot, span id, geometry)`; stride stays `w*4` (logic; V13 is the guard) | H |
| kf-qemu `display.rs:1849-1864`, `:2140-2180` `ScanState.frames`/`retired`, compose chain | `Frame`; `(w,h)` passed three times | `[Option<(ConsoleFrame, u32)>; SLOTS]`; `retired` deleted; `let c = gpu.compose_begin(w,h)?; c.layer(..)?; c.finish(frame)` | H |
| kf-qemu `display.rs:1742-1745` `forget(scan)`, `forget(gpu)` | lifetime held only by convention | `forget(scan)` deleted. `forget(gpu)` may stay; its comment no longer claims memory safety | R / unchanged |
| kf-qemu `display.rs:2267-2275` test helper `frame(addr: usize, ..)` | fake addresses `0x1000…` | spans from a leaked one-page `PrivateAnonymous` region | H |
| kf-qemu `ffi_unsafe.rs:484-527` `Kf3Frame`/`kf3_display_frame` | `f.addr as *mut u8`; no length; `derive(Debug)` | `len: u64` added; `check_frame` (V13); span opened in the existing `*out =` block; `Debug` dropped; `-1` for a missing span, `-2` for a refusal | P |
| `kf3.h:30-36`, `kf3.c:697-717` | `stride < width` compares bytes with pixels; `(int)` casts; no length | `kf3_frame_ok` (V14) plus a placeholder on `-2` (§5); `KF3_ABI 14` | P |
| kf-linux-raw `mapping_unsafe.rs:930` `HostSpan` `derive(Debug)`; `MappedRegion` | `{:?}` prints the address; no span from an owned region | manual Debug; new `StaticSpan`, `static_span` (V15), `host_span(&self)` through `within`, a recorded `BackingKind`; `as_ptr` contract widened (§5) | H |
| Debug on pointer holders: `Mapping` (`mapping_unsafe.rs:229`), `GuestWindow` (`window_unsafe.rs:79`), `RunMapping` (`vcpu_unsafe.rs:265`), `RawRegion` (`raw_unsafe.rs:5`), `OverlayHook` (`:139`), `IoeventfdHook` (`:191`) | `{:?}` prints the address | manual Debug impls (size or length only); gate G1b | H |
| kf-util `lock.rs:880`, `:923`, `trapwitness.rs:398` (`&'static str` keys, violation paths) | `as_ptr() as usize` identity keys | `fnv1a64(what.as_bytes()) \| 1`; equal texts share a slot, which is correct for a census | R |
| kf-util `lock.rs:1348` `claim_site` (called on **every** ranked acquisition: `:241`, `:267`, `:388`, `:1332-1335`) | `ptr::from_ref(site) as usize` | O(1) key: `(line << 40) ^ (col << 24) ^ (file.len() << 8) ^ fnv8(last ≤ 16 bytes of file)`, then `\| 1`. Probe mixing unchanged; a collision merges two sites' census rows, which is documented | R |
| kf-qemu `device.rs:357-359`, `:611-616` | `export.fd_number()` | `export.as_fd()` (`chardev_unsafe.rs:535`); `DisplayPlane::build(store_fd: BorrowedFd<'_>)` (`display.rs:825`) | H |
| kf-harness gates 2–8 (`kf-gate2.rs:61`, `kf-gate3.rs:235`, `kf-gate4.rs:333`, `kf-gate5.rs:93`, `kf-gate6.rs:102`, `kf-gate7.rs:70`, `kf-gate8.rs:364`); `kf-gate9.rs:478`, `:558`; `selftest.rs:195`, `:227` | `fd.fd_number()`; `k.release(img)` | `fd.as_fd()`; `drop(img)` | H |
| `tests/walk_abi_matches_the_cu.rs:740`, `:751` | `view_bytes(rust)` | `rust.encode()`, which also tests the encoder the launches use | R |
| `abi.rs:1156` (report-codec layout test) | `view_bytes(&v)` checks the compiler's layout | per-field `offset_of!`/`size_of` asserted against the documented offsets. `encode()` here would only repeat `:1153` | R |
| kf-qemu `ffi_unsafe.rs:57-73` `dev()`/`write_err` + 11 safe `extern "C" fn` | S1-08 | unchanged | O |
| kf-qemu `raw_unsafe.rs:4-28` `RawRegion` (`Copy`), `:97-121` `BackendFd`, `:138-174` `OverlayHook::submit` + `kf3.c:430` | S1-07; C-side unchecked add | unchanged (only their Debug) | O |
| kf-qemu `device.rs:1382-1385` `irq_fd -> i32` | fd | unchanged | O |
| kf-host `lib.rs:66-72` `CpuViewRelease.p_linear_address` | RM cookie (S1-15) | unchanged | O |
| kf-host `lib.rs:860-1076`, `:1855`; `chardev_unsafe.rs:565-776` | RM companion sizes; forged `NvP64` (S1-40/41) | unchanged | O (§10) |
| kf-linux-raw `mapping_unsafe.rs:790` `MappedRegion::addr_at`, `window_unsafe.rs:504` `GuestWindow::userspace_addr_at` | `pub(crate)` functions returning `u64` addresses, reachable from kf-linux-raw's safe files (used only by `chardev_unsafe.rs` today) | unchanged; recorded as OPEN in G3 | O (§10) |
| `ioctltrace.rs:101` | sees minted addresses before the scrub | unchanged | O (§10) |

---

## 4. kf-cuda API after the change

### 4.1 Public surface (safe; no address appears in any signature)

- **`kf_cuda::{CudaError, CompletionFd}`.** `CompletionFd` is used through `AsFd` only (kf-qemu `device.rs:1511`, `display.rs:1229`, kf-gate8 `:424`).
- **`kf_cuda::abi`:**
  - every pointer-free struct and constant, each with `encode()`;
  - `KfFormat: Default`, and `kf_format_ver2/ver3()` built from it with field assignment;
  - `kf_format_check(&KfFormat) -> Result<(), &'static str>`, re-exported from `walk_gpu_unsafe`;
  - `KF_ARGS_LAYOUT` and `KF_WIN_LAYOUT`.
  - `KfArgs` and `KfWin` are no longer public.
- **`kf_cuda::walk`:**
  - **Types:** `WalkKernel`, `WalkCfg`, `WalkEntry`, `WalkDevice`, `Report`, `ReportError`, `Collected`, `DeviceImage`, `WALK_PTX`.
  - **Unchanged `WalkKernel` methods:** `bring_up`, `bring_up_on`, `store_len`, `write_store`, `read_store`, `read_image`, `write_image`, `submit`, `submit_image`, `refresh`, `refresh_image`, `wait`, `try_collect`, `ack`, `reset_slot`, `max_slots`, `gpu_done_now`, `in_flight`, `completion_fd`, `ctx_sync_calls`, `submits_as_graph`, `graph_param_updates`, `take_capacity_events`, `capacity_stats`, `capacity_census`, `pool_bytes`, `debug_walk_runs`, `probe_failed_launch`, `make_current`.
  - **Changed:**
    - `import_store(BorrowedFd<'_>, u64)`;
    - `upload(&[u8]) -> DeviceImage`, where the image is no longer `Copy`;
    - `try_collect` reports `Poisoned` by name after a failed query (F2).
  - **Removed:** `release`.
- **`kf_cuda::display`:**
  - **Types:** `DisplayGpu`, `Composer`, `ConsoleFrame`, `ComposeLayer` (data), `SCANOUT_PTX`, `COMPOSE_ENTRY`.
  - **`DisplayGpu`:** `bring_up_on`, `make_current`, `import_store(BorrowedFd<'_>, u64)`, `read_store`, `write_store`, `zero_store`, `console_frame(usize)`, `completion_fd`, `compose_begin(&mut self, w, h) -> Result<Composer<'_>, CudaError>`, `selftest_compose`.
  - **`Composer`:** `layer(&ComposeLayer)` and `finish(self, &ConsoleFrame)`.
  - **`ConsoleFrame`:** `len`, `span() -> StaticSpan`, `read(off, n)`.
- **Removed from the public surface:**
  - `driver_unsafe`, `Cuda`, `CUdeviceptr`, every `*Handle`/`Func`/`GraphNode`, `view_bytes`, `zeroed`, `ExportedAllocation`, and every raw method;
  - `Frame`, `Frame::addr`, `DisplayGpu::frame`, `release_frame`, `WalkKernel::release`, `KfArgs`, `KfWin`.
- **Who outside kf-cuda is affected.**
  - kf-qemu uses `WalkKernel`, `DisplayGpu` and `ComposeLayer`. kf-mem uses `WalkKernel` and `abi` constants. kf-harness uses `WalkKernel` and `DeviceImage`.
  - No crate outside kf-cuda calls `Cuda`.
  - So the cost outside kf-cuda is the call-site edits in §3.

### 4.2 The `pub(crate)` edge: `walk.rs` → `WalkGpu`, the only path from a safe file to a driver

```rust
pub(crate) enum Window<'a> { Store, Image(&'a DeviceImage), Same }
pub(crate) struct WalkStage<'a> { pdbs: &'a [u64], slots: &'a [u32], layout: &'a KfLayout,
                                  ack: Option<(u64, &'a [u8])>, resets: &'a [u32] }
pub(crate) enum Polled { Pending, Done(GpuTimes), Poisoned(String) }
impl WalkGpu {
  pub(crate) fn bring_up(cfg: &WalkCfg, fmt: &KfFormat, on: WalkDevice<'_>) -> Result<(WalkGpu, Ident), CudaError>; // V5
  pub(crate) fn import_store(&mut self, fd: BorrowedFd<'_>, bytes: u64) -> Result<(), CudaError>;            // V2
  pub(crate) fn store_write(&self, off: u64, b: &[u8]) / store_read(&self, off: u64, b: &mut [u8]);         // V1
  pub(crate) fn upload(&self, b: &[u8]) -> Result<DeviceImage, CudaError>;                                    // V1
  pub(crate) fn image_read / image_write(&self, img: &DeviceImage, off: u64, b);                             // ctx + V1
  pub(crate) fn launch(&mut self, w: Window<'_>, s: &WalkStage<'_>) -> Result<(), CudaError>;                 // V6, V4, V3
  pub(crate) fn poll(&mut self) -> Polled;                                                                    // V4 flight
  pub(crate) fn read_report(&self) -> Result<(KfReportHeader, Vec<KfPdbEntry>, Vec<KfMapRun>), CudaError>; // V4, V8
  pub(crate) fn move_slot(&mut self, from: Region, to: Region) -> Result<(), CudaError>;                     // V7
  pub(crate) fn read_walk_region(&self, off: u32, cap: u32) -> Result<Vec<KfMapRun>, CudaError>;              // V8
  pub(crate) fn probe_oversized_block(&mut self) -> Option<String>;                                           // V3 (probe)
  /* queries without inputs */
}
```

- **What `walk.rs` keeps:**
  - `WalkCfg` and its logic checks;
  - `Capacity` planning and `capacity_retry`'s decisions (`move_slot`, then `launch(Window::Same, ..)`);
  - ack and reset staging;
  - `Report` decoding and `validate`;
  - its own bookkeeping of what is in flight.
- **It holds no address.** A planner bug becomes a named refusal at V6. A missed completion becomes a refusal at V4. Neither becomes a device write.

### 4.3 Each kernel launch: its wrapper and the checks on it

- **Every launch passes V3 at the point it is issued:**
  - argument count and kinds match `KERNEL_SIGS`;
  - `ArgBlock` is 264 bytes (`kf_walk.ptx:59`);
  - every `Ptr`/`CapOf` range, the `ArgBlock` and the stream belong to the kernel's context;
  - `grid ≥ 1` and `1 ≤ block ≤ 1024`; only the probe waives the block limit.
- **Graph launches.** They are checked when recorded. Each later `GraphExec::set` replaces only argument 0 and `kf_par_seed`'s grid, and passes V3 again.
- **Capacities.** Any kernel argument that is a capacity of a buffer is `Arg::CapOf(range, elem_bytes)`, which passes `range.len / elem_bytes`. A wrapper cannot pass a capacity larger than its buffer.

| Kernel (`kf_walk.ptx` / `kf_scanout.ptx`) | Wrapper (perimeter) | Arguments (bytes) | Extent rule (V5 at bring-up; `CapOf` at launch), and what the kernel relies on |
|---|---|---|---|
| `kf_commit_kernel`, `kf_begin_kernel`, `kf_diff_slots`, `kf_diff_emit` | `WalkGpu::enqueue` | `[264]` | `KfLayout` rows (V6); `slot ≥ max_slots·sizeof(KfSlot)`; `com ≥ slot_pool·32`; `scratch ≥ 4·walk_pool·32`; `iscratch ≥ 3·walk_pool·4`; pinned rpdb/rrun/ack_code sized from the same shape as `KfDev`. Relies on I3 (Res-2) |
| `kf_par_seed` | `par_seed` | `[264,8,8,8]` | `fr0 ≥ F·40`; `nfr ≥ 16`; `used ≥ (KF_DIRS+1)·4`; grid `ceil(npdb/128)` with `npdb ≤ 64` |
| `kf_par_expand` | `par_expand(k, src)` | `[264,4,8,8,8,4,8,8,8]` | the stage capacity is `CapOf(stage, 40)` (today the constant `KF_MAX_FRONTIER`, `walk.rs:1773`); `start/cnt ≥ F·4`; `used.range(k·4,4)` and `nfr.range(src·4,4)` via V1; `k < KF_DIRS` because V5 bounds `first_dir` |
| `kf_par_scan` (×2) | `par_scan` | `[8,8,8,8]` | `cnt/off ≥ F·4`; `total = nfr.range(4·(src^1) or 12, 4)` |
| `kf_par_compact` | `par_compact` | `[264,8,8,8,8,8,8,4]` | `cap = CapOf(dst, 40)` |
| `kf_par_leaf` | `par_leaf` | `[264,8,8,8,4,8,8]` | `cap = CapOf(runstage, 32)`; `sum ≥ F·64`; `used.range(5·4,4)` |
| `kf_par_heads` | `par_heads` | `[264,8,8,8,8,8]` | `head ≥ F` |
| `kf_par_bases` | `par_bases` | `[264,8]` | `pdbbase ≥ 64·4` |
| `kf_par_emit`, `kf_par_join` | `par_emit`, `par_join` | `[264,8×7]`, `[264,8×6]` | `walk ≥ walk_pool·32`, and V6 `walk_off+walk_cap ≤ walk_pool` (`kf_walk.cu:2184-2241`) |
| `kf_walk_kernel` | `probe_oversized_block` | `[264]` | an `ArgBlock` with an empty window; block 2048, refused by the driver |
| `kf_compose` | `Composer::layer` | `[8,8,4×12,4×4]` (`display.rs:542-561`) | V10: `need ≤ extent`, `[src, src+extent)` inside the store (V1); the destination is `staging.range(0,4wh)`; `fw=w`, `fh=h`, `dpitch=4w` come from the composer |

Here `F = KF_MAX_FRONTIER` and `S = KF_MAX_SCRATCH`. Both are pinned to `kf_walk.cu` by T6.

---

## 5. The console frame

**The carrier, end to end.**

1. **`DisplayGpu::console_frame(len)` (V12).**
   ⊘ *Implemented 2026-10-04:* the three middle lines run as one call, `raw::console_pages(&ctx, bytes)`
   (`driver_unsafe.rs:2653`): it maps the private-anonymous region, registers it in the one block, and
   leaks it only after the registration succeeded. Same order, same single block, and the child file
   never holds the mapping.
   ```rust
   // display_gpu_unsafe.rs (no `unsafe` here)
   let bytes = round_up(len, page);                                       // 1 ≤ len ≤ 64 MiB; frames_minted < 16
   let region = MappedRegion::map(Backing::PrivateAnonymous, bytes, HostProt::ReadWrite, CachePolicy::WriteBack, page)?;
   raw::register_console_pages(&self.ctx, region.host_span())?;           // on Err: `region` drops → munmap; nothing registered
   let pages: &'static MappedRegion = Box::leak(Box::new(region));        // from here: never unmapped
   let span = pages.static_span().expect("PrivateAnonymous + OwnedMunmap by construction");
   Ok(ConsoleFrame { pages, span, ctx_id: self.ctx.id })
   ```
   - `register_console_pages` is **one** block: `f(span.as_ptr().cast(), span.len(), 0)` on `cuMemHostRegister_v2`.
   - Its SAFETY argument: the span belongs to a live mapping this function owns; on success the caller leaks that mapping before returning, and on failure nothing was pinned.
   - This is the same backing pattern as v3-broker's `frame_over`, which uses a memfd `SharedFile` and runs on hardware there. `PrivateAnonymous` is UNVERIFIED (row H2). If the driver refuses it, use the sealed-memfd constructor from §9; the code is otherwise identical.
2. **Copy into the frame.** `Composer::finish(&ConsoleFrame)` (V11) calls `staging.range(0,4wh)?.copy_to_console(&stream, frame)`. That one block opens both the span and the range, after `n ≤ frame.len()` and the context checks.
3. **Publish.**
   - kf-qemu `ConsoleShare` gains `spans: [OnceLock<StaticSpan>; SPAN_TABLE]` with `SPAN_TABLE = 2·SLOTS`. The table is append-only and lock-free; only the worker writes it, and each slot grows at most once (`FRAME_SMALL → FRAME_MAX`, `display.rs:1841-1844`).
   - The worker calls `register(span) -> Option<u32>`, then `publish(slot, id, geometry)`. The slot stores `frame: AtomicU32` beside the geometry.
   - A full table means the frame is refused and counted, not published.
4. **Take.** `take()` runs on the main thread, after the existing compare-and-swap (`display.rs:483-501`). It resolves the id to `Option<StaticSpan>` and returns `FrameView { span, id, width, height, stride, format, serial }`.
5. **Hand-off to C.** `kf3_display_frame` calls `check_frame` (V13).
   - **Span missing:** return `-1` (no host frame).
   - **Refused:** count it, log once, and return `-2` without touching `*out`.
   - **Otherwise:**
     ```rust
     // SAFETY: `out` is writable (caller contract). The span is a StaticSpan: a &'static, owned, private-anonymous
     // mapping that is never unmapped; check_frame proved stride*height <= len. C reads only [data, data+len).
     unsafe { *out = Kf3Frame { data: c.span.host_span().as_ptr(), len: c.span.len() as u64,
                                width, height, stride, format, serial } };
     ```
     This is the existing block, so kf-qemu's count is unchanged.
- **Torn reads are harmless.** The id and the geometry are separate atomics. They can come from two different publishes, or, on v3-broker, an id can meet ring geometry for a larger mode. Validating against **that id's** span length after the compare-and-swap turns either case into a refusal or stale pixels, never an out-of-bounds read.

**The C side.** It now works in bytes, not pixels, and receives `-2` and a length.

```c
typedef struct Kf3Frame { uint8_t *data; uint64_t len; uint32_t width, height, stride, format; uint64_t serial; } Kf3Frame;
static inline uint32_t kf3_format_bpp(uint32_t f) { return (f==1||f==2||f==4||f==5) ? 4u : (f==3 ? 2u : 0u); }
static inline int kf3_frame_ok(const Kf3Frame *f, uint32_t bpp) {
    return f->data && bpp && f->width && f->height &&
           f->width <= INT32_MAX && f->height <= INT32_MAX && f->stride <= INT32_MAX &&
           (f->stride & 3u) == 0 && (uint64_t)f->width * bpp <= f->stride &&
           (uint64_t)f->stride * f->height <= f->len;
}
```

- **`kf3_gfx_update`** (`kf3.c:697-717`):
  - On `rc == -2`, or on `rc == 0` with `kf3_frame_ok` failing: call `dpy_gfx_replace_surface(s->con, NULL)` **once**, tracked by `s->placeholder`. QEMU 9.2 makes a placeholder for `NULL` (`ui/console.c:822-832`). Then clear `s->shown`.
  - On `rc == -1`, or an unchanged serial: return, keeping the current surface. Its pages are leaked, so this risks stale pixels at worst.
  - Otherwise, create the surface. The `(int)` casts are bounded by the `INT32_MAX` checks.
- **ABI.**
  - `KF3_ABI 11 → 14` in both `kf3.h:16` and `ffi_unsafe.rs:17`. 12 is v3-broker's and 13 is v3-dispsw-exp's.
  - The `wire_mirror.rs` row (`:113`) gains `len`.
- **New `kf3.h` contract:**
  - `[data, data+len)` is valid for the life of the process;
  - its contents are stable until the next call;
  - `-1` means no new frame; `-2` means a frame was refused.

**Lifetime.**

- Nobody frees a published console frame. `cuCtxDestroy`, whether reached by a panic unwinding or at process exit, only unpins the pages.
- This is required, not just cautious, because QEMU keeps reading after a surface is replaced:
  - screendump holds an image reference across a release of the global lock (`ui/ui-qmp-cmds.c:360-390`);
  - the D-Bus listener sends `stride × height` bytes, zero-copy (`ui/dbus-listener.c:716-731`).
- Reuse of a frame's contents is governed by the ring: a slot is written only when it is neither front nor ready.
- The worst-case leak is capped by F3: 16 frames of at most 64 MiB. Expected use is 2 × SLOTS frames of 8.3 MB or 33.2 MB.

**`HostSpan::as_ptr` contract** (`mapping_unsafe.rs:975-978`), widened to name three consumers:

- a hypervisor memory-region backing;
- a GPU driver's page-lock registration, and async copies into a registered span;
- a VMM console surface read.

Each consumer may use only `[ptr, ptr+len)`, must never form a Rust reference into it, and must not outlive the minting mapping. A `StaticSpan` satisfies the last condition by construction; `register_console_pages` satisfies it by leaking before it returns.

---

## 6. The CI gates

### G1: disguised-address gate (lexical)

The script is `scripts/ci/address_gate.py`. It runs as a new step after the host-pointer gate (`ci.yml:752-783`) in the `stable` job.

**Files scanned.**
- Included: `crates/kf-*/**/*.rs` and `firmware/**/*.rs`.
- Excluded: `*_unsafe.rs`, `*/tests/ui/*` (trybuild fixtures), and `target/`.
- The gate fails if it scanned fewer than 18 kf-* crates, the floor that `dependencies.py:42` uses (20 exist today).

**Lexing.** A real lexer removes:
- `//` comments and nested `/* */` comments;
- char literals;
- string literals, including raw strings (P7 runs on their contents only);
- `PhantomData<…>`, as the host-pointer gate does, since it cannot carry an address.

**Rejected patterns.** These were run against `e4fb0190` and v3-broker `28b5b6fe` with a prototype.
⊘ *Implemented 2026-10-04:* one P2 pattern was added, a bare `from_ref(`/`from_mut(` call not
qualified by `slice::`, `array::` or `Cell::` (`address_gate.py:145`), so `use core::ptr::from_ref`
followed by an unqualified call is caught too.
```
P0  \*\s*(const|mut)\b                                   # closes `*const()` / `*mut[u8]`, which the 4-token gate misses
P1  &raw\s+(const|mut)\b
P2  \bptr::(from_ref|from_mut|null|null_mut|dangling|dangling_mut|without_provenance(_mut)?|with_exposed_provenance(_mut)?|addr_of(_mut)?|slice_from_raw_parts(_mut)?|hash)\b
    \bptr\s+as\s+\w+                                     # any rename of the module, incl. `use core::{ptr as q}`, and `ptr as usize`
    use\s+(core|std)::ptr::\*
P3  \baddr_of(_mut)?!
P4  \b(as_ptr|as_mut_ptr|as_ptr_range|as_mut_ptr_range|into_raw|into_raw_parts|expose_provenance|with_addr|map_addr)\b
P5  \.addr\s*\(\s*\)  |  ::addr\b(?!\s*(::|\{))          # method and path forms (`map_or(0, Frame::addr)`); not the module `crate::addr::`
P6  \b(AtomicPtr|UnsafeCell|SyncUnsafeCell|CUdeviceptr|DevAddr)\b | \bfmt::Pointer\b | \bPointer::fmt\b | use[^;]*\bPointer\b
P7  (in string contents) \{[^{}]*:[^{}]*p\}             # {:p} {x:p} {:#p} {:>16p}
P9  \b(addr|addrs|ptr|hva|dptr|devptr|dev_ptr|host_addr|host_ptr|dev_addr|base_addr)\s*:\s*\[?\s*(usize|AtomicUsize|NonZeroUsize)\b
    \b(ptr|dptr|devptr|dev_ptr|dev_addr)\s*:\s*\[?\s*(u64|AtomicU64|NonZeroU64)\b
    \bfn\s+\w*(addr|ptr)\s*(<[^>]*>)?\s*\([^)]*\)\s*->\s*(Option<\s*)?usize\b     # usize only: guest/GPU `-> u64` getters are legitimate
```

**Hits today**, from a prototype of this lexer run on 2026-10-04:
- `e4fb0190`: only sites this branch removes. That is kf-util ×4; kf-qemu `display.rs:418`, `:430`, `:2023`, `:2267`; and kf-cuda `walk.rs`/`display.rs` (`CUdeviceptr`, `Frame::addr`).
- v3-broker `28b5b6fe`: only the sites §9 converts. That is kf-qemu `display.rs:430`, `:457`, `:2563`, `:3112`, plus the kf-cuda `display.rs` uses.
- The draft's `u64` function form would also have flagged kf-trap `pramin.rs:113 slot_addr` and v3-broker kf-disp `tests/compose_kernel.rs:184 addr`. The `usize`-only form does not.
- **The gate must report 0 at the branch tip.**

**Fixtures.**
⊘ *Implemented 2026-10-04:* the fixture files are named `*.rs.txt` (`positive.rs.txt`, `negative.rs.txt`,
`perimeter_debug.rs.txt`, `perimeter_debug_negative.rs.txt`, `perimeter_reexport.rs.txt`), so that
rustfmt, Cargo and the workspace's own unsafe and host-pointer gates never read them as sources. The
script also has a Python unit test, `scripts/ci/test_address_gate.py`.
- **Known positives:** `scripts/ci/fixtures/address_gate/positive.rs`, where every non-blank line is one violation. It includes:
  - master's S1-03 shapes: `pub fn addr(&self) -> usize {`, `addr: AtomicUsize,`, `pub addr: usize,`;
  - v3-broker's shapes: `addrs: [AtomicUsize; SLOTS],` and `host.map_or(0, Frame::addr)`;
  - the four kf-util lines, `CUdeviceptr`, `&raw const x`, `format!("{:p}", r)`, `Box::into_raw(b)`, `x as *const() as usize`, `use core::{ptr as q};` and `q::from_ref(&x)`.
- **Look-alikes:** `negative.rs`, which must produce zero hits. It holds:
  - `std::slice::from_ref`;
  - `"as_ptr"` inside a string, and a comment;
  - `fn cmd_write_ptr_off(&self) -> u64`, `addr: u64`, `fn wpr2_reg(addr: u64)`, `pub fn slot_addr(self, i: usize) -> Option<u64>` and `fn addr(&self, o: &Op) -> u64`;
  - `use crate::addr::{HostToken};` and `PhantomData<*mut ()>`.
- **Self-test** (`--self-test`, run before the scan): the hit count must equal the non-blank lines of `positive.rs`, and `negative.rs` must give zero.

### G1b: no pointer-printing `Debug` inside the perimeter

- The same script scans `*_unsafe.rs` in kf-linux-raw, kf-cuda and kf-qemu.
- **What it refuses:** a `#[derive(..Debug..)]` directly on a struct (named or tuple) or an enum when any field or variant payload:
  - has type `NonNull<…>`, `*mut …` or `*const …`; or
  - is typed `usize` and named `raw`, `ptr`, `host` or `addr`.
- **Deliberately not flagged:** `u64` fields named `addr`/`base`. They are guest-space ABI values: `kvm_unsafe.rs:76` `CoalescedZone`, `:85` `IoEventFd`, `vcpu_unsafe.rs:156` `KvmSegment`, `:175` `KvmDtable`, `ffi_unsafe.rs:43` `Kf3Region`.
- **Fixtures:**
  - known positive: `fixtures/address_gate/perimeter_debug.rs`, including a tuple struct and an enum with a pointer payload;
  - look-alike: `perimeter_debug_negative.rs`, a `CoalescedZone`-shaped struct.
- **Effect:** this forces the manual Debug impls in §3. On v3-broker it also forces v3-broker's `BrokerHooks` (`raw_unsafe.rs:257`), listed in §9.

### G1c: no pointer re-exports from the perimeter

- In `*_unsafe.rs`, refuse `pub(\([^)]*\))?\s+use\s+[^;]*\b(ptr|NonNull|addr_of|AtomicPtr|UnsafeCell)\b`.
- G1 skips perimeter files, so without this a perimeter could re-export a pointer function under a harmless name.
- Hits today: zero, on both trees.
- Known positive: `fixtures/address_gate/perimeter_reexport.rs`.

### G1d: type-resolved check (Clippy), S1-02's own recommendation

⊘ **Implemented 2026-10-04, as a separate CI step with its own configuration** (§11.5): the list
lives in `scripts/ci/address-clippy/clippy.toml` (23 methods, 3 types: the list below plus
`str::as_ptr`, `slice::as_ptr` and `slice::as_mut_ptr`, which Clippy resolves), selected with
`CLIPPY_CONF_DIR` by `scripts/ci/address_clippy.sh` over the kf-* crates only. The script's
self-test compiles `scripts/ci/fixtures/address_clippy/positive.rs.txt` and requires exactly one
`disallowed_*` diagnostic per marked line (27), and `perimeter.rs.txt` shows the opt-out silences
them. The workspace `clippy.toml` is unchanged.

- **`clippy.toml` additions:**
  - `disallowed-methods`: `core::ptr::{from_ref, from_mut, null, null_mut, dangling, dangling_mut, without_provenance, with_exposed_provenance, hash, slice_from_raw_parts}`, `alloc::boxed::Box::into_raw`, `alloc::sync::Arc::{into_raw, as_ptr}`, `alloc::rc::Rc::{into_raw, as_ptr}`, `alloc::ffi::CString::into_raw`, `core::ffi::CStr::as_ptr`, `alloc::vec::Vec::{as_ptr, as_mut_ptr}`, `core::cell::Cell::as_ptr`;
  - `disallowed-types`: `core::ptr::NonNull`, `core::sync::atomic::AtomicPtr`, `core::cell::UnsafeCell`.
- **Opt-out:** each `*_unsafe.rs` starts with `#![allow(clippy::disallowed_methods, clippy::disallowed_types)]`. G1 also refuses `(allow|expect)\(clippy::disallowed_` outside `*_unsafe.rs`.
- **Why it is needed:** Clippy resolves paths, so renames, re-exports and inference cannot hide a call.
- **Fixture:** `crates/kf-cuda/tests/ui/clippy_positive.rs` is a known positive that must produce exactly N `disallowed_*` diagnostics. UNVERIFIED: which slice and `str` inherent paths Clippy 1.99 resolves. The fixture decides; unresolved ones stay with G1's P4.

### G2: trybuild rows

kf-cuda gets `crates/kf-cuda/tests/compile_fail.rs`. It copies the `REQUIRED_ROWS` pattern of `kf-linux-raw/tests/compile_fail.rs:44-87` and adds `trybuild = "1.0"` as a dev-dependency.

| Row | Expected |
|---|---|
| `raw_binding_unreachable.rs` (`kf_cuda::driver_unsafe::Cuda::open()`) | E0603 |
| `no_device_address_type.rs` (`kf_cuda::CUdeviceptr`) | unresolved |
| `console_frame_has_no_addr.rs` (`frame.addr()`) | E0599 |
| `image_is_not_copy.rs` (`drop(img); k.read_image(&img, ..)`) | E0382 |
| `image_is_not_clone.rs` | E0599 |
| `image_is_not_hash.rs` (`fn h<T: Hash>(){}; h::<DeviceImage>()`), and the same for `ConsoleFrame` | E0277 |
| `composer_holds_the_staging.rs` (a second `compose_begin` while a `Composer` lives) | E0499 |
| `finish_needs_a_console_frame.rs` (`c.finish(&frame.span())`) | E0308 |
| `no_launch_args_type.rs` (`use kf_cuda::abi::KfArgs`) | E0432 |
| `import_takes_a_borrowed_fd.rs` (`k.import_store(3, 4096)`) | E0308 |

Rows added to kf-linux-raw's matrix:

| Row | Expected |
|---|---|
| `static_span_needs_static.rs` | E0597 |
| `host_span_open_needs_unsafe.rs` — ⊘ *implemented as `span_address_needs_an_unsafe_block.rs`* (2026-10-04: a fixture whose name ends in `_unsafe.rs` outside an audited `src/` fails gate B) | E0133 |
| `host_span_not_forgeable.rs` (`HostSpan { base, len }`) | E0451 |
| `span_is_not_hash.rs` / `span_is_not_ord.rs` (`StaticSpan`, `HostSpan`) | E0277 |

New for kf-qemu (crate-type includes `rlib`, `Cargo.toml:9`): `frame_view_has_no_address.rs` (`FrameView { addr: 0, .. }`), expected E0560.

The child modules' inability to forge a `DevRange` is crate-internal privacy, which trybuild cannot express. It is enforced by `raw`'s field privacy and checked by G3's census of `raw`'s exports.

### G3: perimeter-exports census (§R gate 3)

The test is `crates/kf-cuda/tests/perimeter_exports.rs`.

- **Full census, in both directions,** for kf-cuda's `src/**/*_unsafe.rs`:
  - every `pub`, `pub(crate)` and `pub(in crate::driver_unsafe)` fn, keyed `Type::fn` by tracking the enclosing `impl` and module;
  - compared against a committed `EXPORTS: &[Row { item, checks: "V1,V6", test: "fn name" }]`;
  - every named test must exist.
- **Presence-only rows outside kf-cuda** (not a census):
  - kf-linux-raw: `MappedRegion::static_span`, `MappedRegion::host_span`, `StaticSpan::*`;
  - kf-qemu: `kf3_display_frame` and `check_frame`;
  - **OPEN rows:** `CharDevice::ioctl` (S1-40), `MappedRegion::addr_at` (`mapping_unsafe.rs:790`) and `GuestWindow::userspace_addr_at` (`window_unsafe.rs:504`). These are listed so they are not read as covered.
- **Known positive:** the extractor runs on a fixture string holding an unlisted `pub fn`, and the comparison must report it.

### G4: ratchet constants

- **`AUDITED` (`ci.yml:1522`):** `kf-cuda:75 → 66`, with the per-item text from §7. The others stay.
  ⊘ *Implemented 2026-10-04:* `kf-cuda:75 → 64`, itemised at `ci.yml:1542-1568`.
- **A second number per audited crate, `PERIMETER` (§R(c)):**
  - It counts non-blank, non-`//` lines in `*_unsafe.rs`, compared exactly.
  - Baselines at `e4fb0190`, counted with the CI expression: kf-linux-raw 4596, kf-cuda 1084, kf-qemu 571.
  - kf-cuda's value is set to its count at the branch tip, about 2,100.
  - ⊘ *Implemented 2026-10-04:* `PERIMETER="kf-linux-raw:4810 kf-cuda:4493 kf-qemu:658"` (`ci.yml:1959`),
    itemised above it. kf-cuda is about twice the estimate because the GPU-free tests of every
    validation site live in the perimeter files beside the code they test (`tests_unsafe.rs` and the
    children's test modules), and a test module counts as perimeter lines.
  - Written reason: §R(b) moved launch-argument assembly and its validators out of `walk.rs` (1504 code lines) and `display.rs` (447).

---

## 7. `unsafe` count before and after (the CI regex, `ci.yml:1872-1877`)

⊘ **Implemented 2026-10-04: kf-cuda 75 − 15 + 4 = 64**, not 66; kf-linux-raw 75 and kf-qemu 46 as
planned. The two extra removals: `sym!` now goes through `opt!` plus a named refusal, which drops its
`transmute` block as well as its `dlsym` block; and `alloc_zeroed` zero-fills through the bounded
`DevMem::fill` instead of its own `cuMemsetD8` block. The `console_pages` block and the three import
guards' `Drop` blocks are the +4. Itemised at `ci.yml:1542-1568`.

| Crate | Before (counted at `e4fb0190`) | After | Items |
|---|---|---|---|
| kf-linux-raw | 75 | **75** | `StaticSpan`, `static_span`, `host_span`, the recorded `BackingKind` and the manual Debug impls are all safe in-module code |
| kf-cuda | 75 | **66** | see below |
| kf-qemu | 46 (ffi 28, raw 18) | **46** | the span is opened inside the existing `*out =` block; `check_frame` is plain Rust |

**kf-cuda: 75 − 9 − 4 + 1 + 3 = 66.**

- **Removed (−9):**
  - `launch_raw`'s block (`:726`), folded into the single `cuLaunchKernel` block;
  - `pinned_free` (`:985`);
  - dead `memcpy_h2d_async` (`:1042`);
  - `view_bytes` (`:1566`) and `zeroed` (`:1596`);
  - dead `export_device_allocation` (`:1766`, `:1782`, `:1789`, `:1800`).
- **Removed (−4), required by §R** (`unsafe {}` only where Rust needs it):
  - `sym!`'s own `dlsym` block (`:351`), now resolved through `sym_or_null`;
  - the `opt_sym` closure's block (`:362`), folded into `sym_or_null`;
  - `cuGetErrorName`'s separate transmute (`:434`), now through `opt!`;
  - `device_name`'s `CStr::from_ptr` (`:547`), now `CStr::from_bytes_until_nul` over a `[u8; 128]`.
- **Added (+1):** `register_console_pages`, one call of `cuMemHostRegister_v2` that opens a `HostSpan`.
- **Added (+3):** the import guards' `Drop` impls, one driver call each: `cuMemRelease`, `cuMemAddressFree`, `cuMemUnmap`.
- **Everything else** moves unchanged into its handle method. `copy_to_console` reuses `:1071` and opens the span inside it.
- **No `unsafe fn` is added.** `walk_gpu_unsafe.rs`, `display_gpu_unsafe.rs` and `tests_unsafe.rs` count 0.
- **Not counted:** the new `Option<unsafe extern "C" fn(..)>` field (`ci.yml:1474`).

---

## 8. Tests that can fail

All tests are GPU-free unless marked H.

⊘ **Implemented 2026-10-04:** every T row has a test; §11.3 names it and the mutation run against
it. One row is implemented differently: **T18** is a source scan of the perimeter files
(`no_handle_derives_a_pointer_printing_debug`, `tests_unsafe.rs`) for a derived `Debug` on any
type that holds an address, plus kf-linux-raw's `no_debug_output_carries_the_address`, because
most kf-cuda handles cannot be built without a GPU.

⊘ **Updated 2026-10-04 (second review; §11.7 has each test and mutation):**
- **New GPU-free rows:** T11 gains one refusal row per V10 arm and the overflowing layer; T12
  gains a 100 000-layer tail at the top of every type, read through a model of the PTX's own
  wrapping arithmetic; **T21b** a proof of an earlier flight is refused; **T25** a stale graph
  launch re-sets only the nodes that change; **T26** a console-frame read is bounded before it
  allocates; kf-host's store record; kf-linux-raw's memslot `Debug`; kf-harness's
  `merge_bar_rows` (kf-gate9 runs H3 and H4's self-test).
- **H1** is defence in depth now, not a blocker (§2.6 note).
- **H3 runs in `kf-gate9`** (`dropped_image`, `crates/kf-harness/src/bin/kf-gate9.rs:581`): the
  image is dropped right after `submit_image`, the report must be the tables' one run, the next
  walk over a fresh image must be right, and the dropped image must be freed by exactly ONE
  context drain across that next submit (taken before the walk is queued, finding 11).
- **H4's kf-cuda self-test runs in `kf-gate9`** (`selftest`, `:653`): `bring_up_and_prove` and
  `probe_after_sandbox`, one check per outcome. H4 must also read: (a) a store `write_store` /
  `read_store`, a walk and a display `read_store` with the import handle now held until the unmap
  (finding 14); (b) gate 8's submit and collect lines with the per-collect `cuEventQuery` that
  mints the flight's proof, and with a stale walk re-setting only the block-reading and regridded
  nodes (finding 12); (c) gate 8's `trigger_trap` ceiling with the O(1) census keys (finding 13);
  (d) gate 8's `ctx_sync_calls` falsifier: a walk over the store frees nothing, so it must stay
  unchanged.

| ID | Check | Violating input that must be refused | Mutation that turns it red |
|---|---|---|---|
| T1 | V1 `sub_range` | `off = u64::MAX, n = 2` with `len = 4096`; `off + n == len + 1`. An exact fit (`== len`) must be **accepted** | `checked_add` → `wrapping_add`; `>` → `>=`; delete the check |
| T1b | V1b async context | a range of ctx A with a stream of ctx B | drop the stream context compare |
| T2 | V2 import | `len = 0`; a negative fd cannot be expressed (G2) | delete the `len ≥ 1` arm |
| T3 | V3 `check_args` | 17 of 18 compose arguments; a `u32` where a pointer is expected; a pointer or `ArgBlock` of another context; `block = 2048` outside the probe | delete the kind loop; compare `len()` with `<=` |
| T4 | `KERNEL_SIGS` == PTX | parse every `.entry` in `kf_walk.ptx` and `kf_scanout.ptx` (extends `display.rs:500-546`, `walk_abi_matches_the_cu.rs:776-800`) | edit one entry in the table |
| T5 | V5 shape and requirement table | `4·walk_pool > KF_MAX_SCRATCH`; `max_slots = KF_MAX_SLOTS+1`; a requirement row 1 byte over its allocation | drop the `stage` row; loosen `≤` |
| T6 | constants pinned to the `.cu` | `KF_MAX_FRONTIER`, `KF_MAX_SCRATCH`, `KF_DIRS`, `KF_USED_SLOTS`, `KF_MAX_ENT`, `KF_ENT_BYTES = sizeof(KfEnt)`, `KF_SUM_BYTES = sizeof(KfSum)` against `kf_walk.cu:96`, `:1546-1586` | edit any Rust constant |
| T7 | V6 `validate_stage` | `walk_off[63]+walk_cap[63] = walk_pool+1` **with `npdb = 1`**; `slot_off[127]+slot_cap[127] = slot_pool+1`; `walk_cap > runs_per_pdb`; `npdb = 65`; `slot ≥ max_slots`; a duplicate slot; `nreset = 65`; `Window::Same` with none retained; an image of another context | loop only over `t < npdb` or `s < max_slots`; drop the context compare |
| T8 | V7 `move_slot` | `to.off+to.cap = slot_pool+1`; overlapping regions; `from.cap > to.cap`; while `InFlight` | delete the disjointness check |
| T9 | V8 | `read_walk_region` past `walk_pool`; `read_report` while `InFlight`; a header claiming `run_count > run_capacity` comes back clamped | remove the clamp |
| T10 | V9 | `compose_begin(16385, 1)`, `(0, 1)` | `> 16384` → `> 16385` |
| T11 | V10 `compose_layer_fits` | today's `display.rs:563-593` grid, moved; a 1921×1080 layer in a 1920×1080 composer; `extent` past the store's end | drop the `ox+width ≤ fw` arm |
| T12 | S1-05(b) compose address model | a seeded xorshift sweep of 200k V10-accepted layers plus boundary layers. The CPU model (pitch `r·pitch + 4x`; block-linear `kf_disp::scanout::bl_offset`, `scanout.rs:168`) stays `< src + extent`, and writes stay `< 4·fw·fh`. **At the v3-broker merge**, the same sweep drives v3-broker's PTX interpreter (`kf-disp/tests/compose_kernel.rs`), which already asserts every load and store is in range | `need` one byte short: the sweep must find an out-of-bounds read |
| T13 | V11 | finish into a frame of `4wh − 1` bytes; a frame of another context | `>` → `>=` |
| T14 | V13 `check_frame` | xrgb8888 with `stride == width` (**the `kf3.c:706` bug**); `stride·h == len` accepted and `len + 1` refused; `stride = 0x8000_0000`; `w = 2^30` at bpp 4; formats 0 and 6; `w = 0`; `h = 0`; `stride % 4 != 0`; span `None` → `-1`. A mutant row asserts the old predicate accepts what the new one refuses | the old predicate; delete the `len` arm |
| T15 | V14 = V13 | `wire_mirror.rs` (already builds C, `:133-146`) compiles `kf3_frame_ok` with `-Wall -Wextra -Werror` and runs T14's grid; any disagreement fails | change one arm on either side |
| T16 | `ConsoleShare` | over a one-page leaked span: a fitting geometry → `0` and `out.len == 4096`; 64×64 at stride 256 → `-2` with `out` untouched (**fails on today's code**); torn id/geometry from two publishes → refused | publish the geometry without the id |
| T17 | encoders | `KfFormat/KfDev/KfLayout/KfAck::encode()` and `raw::encode_kf_args` equal the `.cu` bytes (replacing `view_bytes` at `walk_abi_matches_the_cu.rs:740-751`); padding bytes are zero. The `abi.rs:1156` row becomes per-field `offset_of!`/`size_of` | skip one field |
| T18 | Debug | `format!("{:?}", ..)` of every span and handle in §2.2 contains no hex of the opened address. Inline in `raw` (`#[cfg(test)]`), the only place the address may be opened | re-derive `Debug` |
| T19 | gates | `address_gate.py --self-test` (G1, G1b, G1c fixtures); G1d fixture count; G2 `REQUIRED_ROWS`; G3 fixture | remove a pattern, a row or a census row |
| T20 | V5 format check | the `kf_format_check` grid: `first_dir = 5`; an active slot above `first_dir`; `entries = 1024` and `entries = 3`; `entry_bytes = 12`; `va_lo = 64`; `big_va_lo − small_va_lo = 5`; mismatched big/small coverage; `ap_bits = 3`; `ps_log2 = 41`. VER2 and VER3 descriptors (`kf_format_ver2/3`) are **accepted** | delete any arm |
| T21 | V4 flight state (pure `Flight::step`) | `write`/`read`/`launch` while `InFlight`; a query error → `Poisoned`, after which every operation is refused; only `Drained` returns to `Idle` | clear on error (today's `walk.rs:1333`) |
| T22 | F1 disposition (pure `drop_disposition(drain)`) | a failed drain → `Leak`, never `Free`; the completion fd is forgotten | free on `Err` |
| T23 | F3 / V12 bounds (pure) | the 17th console frame; `len = 64 MiB + 1`; byte totals | drop the count cap |
| T24 | kf-util keys | two call sites on different lines or files get different slots; one site always gets the same slot; the key computation reads at most 16 bytes of the file name | key on `line` only |
| H1 | Res-1 (**merge blocker**) | importing with `len = real + 2 MiB` is refused by `cuMemMap`. If it is not, switch to the `RmExport` token (§2.6) before merging | — |
| H2 | V12 on hardware | `cuMemHostRegister` of `PrivateAnonymous` succeeds (otherwise use the sealed memfd, §9); the console shows frames; screendump works; a forced registration refusal leaves no new mapping in `/proc/self/maps` | — |
| H3 | `DeviceImage` lifetime | kf-gate9 drops the image right after `submit_image`, before collecting; the report is still correct | — |
| H4 | regression | gates 2–9; the kf-cuda self-test (`probe_failed_launch` still refused); display and gop box rows; gate 8's `ctx_sync_calls` falsifier unchanged (no `DevMem` is dropped during a walk) | — |

---

## 9. What v3-broker must convert when the two meet (re-derived at `82f98f42`; whichever merges second converts)

⊘ **Re-derived 2026-10-04 at v3-broker `82f98f42` (second review, findings 7 and 9).** The note
that stood here said the list "still applies line for line" at `f8d74ac3`. It did not hold: three
later v3-broker commits (`2b7751bc`, `17239ad8`, `82f98f42`) changed the console seam. Every
v3-broker citation in this section is now at `82f98f42`, and the rows directly below are new. Its
counts are unchanged: kf-cuda 78, kf-linux-raw 94, kf-qemu 59, `KF3_ABI` 12 (CI regex over its
`*_unsafe.rs`, recounted). **The post-merge kf-cuda ratchet is 65** (this branch's 64 +
`host_unregister`), not 67; kf-qemu 59 and kf-linux-raw 94 stand. One name differs on this branch:
`register_console_pages` is `raw::console_pages`, which maps the private-anonymous pages itself (§5
note); generalising it to `Pages::{Static, Owned}` for the broker frames is part of the conversion.

**New since `f8d74ac3`, each a conversion row:**
- **The frame's cursor bit.** `FrameView` gained `cursor: bool` (kf-qemu `display.rs:444`) and still
  derives `PartialEq` over `addr` (`:429-432`). This branch's `Published` (the worker's publish
  record, `display.rs` on this branch) carries `cursor` too; `FrameView` carries it beside `span`
  and `id`, still with no `PartialEq`.
- **`ConsoleShare.cursors: FrameCursors` and `shown_frame()`.** `take` (`:552`) calls
  `self.cursors.took(slot)` (`:556`) before the C side has checked the frame, and `shown_frame`'s
  doc (`:527-530`) says *"the frame it would refuse — a bad format or geometry — the worker never
  makes"*. With this branch's checks that is false: V13/V14 can refuse a frame, and `kf3.c` then
  shows the placeholder. Convert: `kf3_display_frame` runs `check_frame` first and calls
  `cursors.took(slot)` only for a frame it hands the console; a `-1` keeps what was shown and a
  `-2` records `ShownFrame::Nothing` (the placeholder carries no cursor); correct the doc.
  `FrameCursors`/`ShownFrame` (kf-broker `console.rs:67`, `:83`) key on the ring SLOT, never on an
  address — keep it so (G1 would flag an address-keyed table).
- **`kf3_gfx_update` is now a wrapper** (`kf3.c:795`): `kf3_console_frame` (`:773`, its
  `kf3_display_frame` call at `:778`) and then `kf3_console_cursor` (`:806`), the cursor AFTER the
  frame. The merge target for `kf3_frame_ok`, `len`, the `-1`/`-2` split and the placeholder is
  `kf3_console_frame`, not `kf3_gfx_update`. Keep the cursor-after-frame order.
- **`kf3_display_frame` refuses a misaligned `out`** (`ffi_unsafe.rs:603`, `out.is_null() ||
  !out.is_aligned()`). Keep it: this branch's version checks only null (`ffi_unsafe.rs:583`). The
  same refusal is on `kf3_display_cursor` (`:637`) and `kf3_display_cursor_pixels` (`:702`).
- **`kf3_display_cursor_done`** (`ffi_unsafe.rs:674`; `kf3.h`): an applied-parts mask, no address.
  It gets a census presence row like the other cursor FFI.
- **`SLOTS` is `kf_broker::slots::MAX_SLOTS`** (`display.rs:426`; `= 5`, kf-broker `slots.rs:141`):
  the span table is `2 × MAX_SLOTS` = 10 entries, inside F3's cap of 16 frames.
- **Display slots export like the store.** v3-broker's kf-host already records each display slot's
  length (`Objects::display_slots`, kf-host `lib.rs:383`, written by `alloc_display_slot` `:1937`)
  and checks it in `export_display_slot` (`:1988`), which returns a `CharDevice`. Convert:
  `export_display_slot` returns this branch's `RmExport` (the fd and the recorded length; NVKMS's
  import borrows `as_fd()`), one record serving stores and slots, and
  `DisplayGpu::import_slot(&RmExport)` keeps its 2 MiB / 64 MiB check inside the perimeter. This
  replaces the `import_slot(BorrowedFd<'_>, bytes)` row below.
- **kf-disp `plan_block_linear`** (`scanout.rs:288`) still forms the extent with unchecked
  products on v3-broker; take this branch's checked form (finding 1, `scanout.rs:300` here). V10
  (finding 1) is the bound that matters either way.

**kf-cuda `driver_unsafe.rs`:**
- `host_register` (`:1006-1031`) returns `PinnedBuf { ptr: p as usize }`. Replace it with this branch's `register_console_pages`, generalised to a private input `Pages::{Static(StaticSpan), Owned(&MappedRegion)}`. It stays **one** block, so v3-broker's two blocks fold into it.
- `host_unregister` (`:1033-1044`) is reachable only from `BrokerFrame::release`.
- Delete `PinnedBuf::empty()` (`:1409`).
- The `cuMemHostRegister` field (`:259`, `:438`) is identical to this branch's.

**kf-cuda `display.rs`:**
- **Split `Frame{buf, map}` (`:134-177`) into two types:**
  - `BrokerFrame`: owned and releasable. It holds the `MappedRegion` and a `Registration`. `release(self, &DisplayGpu)` drains, unregisters, then unmaps; if the drain or the unregister fails, it forgets the map (F1). Its `Drop` forgets the map, as `:169-177` does today.
  - `ConsoleFrame`: produced only by `BrokerFrame::publish(self)`, which leaks the map with `Box::leak` and then calls `static_span()`.
- **Frame constructors:**
  - `frame_over(map)` (`:343-361`) becomes `broker_frame_over(&SharedRam)`. The perimeter maps the **sealed** memfd itself, checking that it was created sealed with `SHRINK|GROW|SEAL` (broker.rs `:7-9`). Today kf-qemu `broker.rs:141-170` builds an arbitrary `MappedRegion` and passes it in. After the change, the GPU writes only pages the perimeter mapped, and no peer can SIGBUS the console.
  - `static_span` gains a sealed-memfd case for these frames only.
  - `frame` (`:326`) becomes `console_frame`. `release_frame` (`:373-394`) becomes `BrokerFrame::release`. Delete `Frame::addr` (`:142`).
- **Slots:**
  - `slots: Vec<(CUdeviceptr,u64)>` (`:124`) becomes `Vec<DevMem>`.
  - `import_slot(fd: i32, bytes)` (`:533-550`) becomes `import_slot(BorrowedFd<'_>, bytes)`, with its 2 MiB / 64 MiB check inside the perimeter.
  - `zero_slot` (`:557-566`) becomes `DevMem::fill`. `slot_bytes` (`:569`) becomes `DevMem::len`.
- **Compose:**
  - `compose_to_slot` and `launch_pack` (`:484-531`) become `Composer::to_slot(SlotId, &BlPack)`. The pack source is bounded by the composition's `4wh`, not by the staging capacity as today (`:487-490`).
  - `BlPack::check` (`:69-103`) moves into the perimeter as V10b; `BlPack` stays as data.
  - `compose_to_host` (`:458-480`) becomes `Composer::to_host(&ConsoleFrame | &BrokerFrame)`. `compose_signal` (`:449`) becomes `Composer::signal(self)`. `compose_finish` (`:439`) becomes `Composer::finish`.
  - **V10 keeps v3-broker's `flags & !COMPOSE_FLAGS` refusal** (`:188`, `:240-242`).
  - `selftest_bl_pack` (`:580-612`) moves into the perimeter.
  - `kf_bl_pack` (`[8,8,4,4,4,4,8,4,4,4,4,4]`) is added to `KERNEL_SIGS`; its gob count becomes `CapOf`-style where it bounds a write.

**kf-cuda tests.** T12's sweep drives v3-broker's PTX interpreter (`kf-disp/tests/compose_kernel.rs`) once both are present. Its `fn addr(&self, o: &Op) -> u64` (`:184`) passes G1, which flags only `usize` returns.

**kf-qemu `display.rs`:**
- `FrameView.addr` (`:430-432`) becomes `span: Option<StaticSpan>, id`. Drop `PartialEq`/`Eq`.
- `ConsoleShare.addrs: [AtomicUsize; SLOTS]` (`:461`) becomes `frames: [AtomicU32; SLOTS]` plus `spans: [OnceLock<StaticSpan>; 2*SLOTS]`, i.e. 10 entries with the broker on, below F3's 16.
- `publish` (`:576-600`) stores the span id when `host` is set and `NONE` otherwise. A VRAM-only publish therefore yields `-1` in C (keep the surface), not `-2`.
- `take` (`:552-565`) resolves the id.
- `completed` (`:2581`, `addr: host.map_or(0, Frame::addr)`) becomes `span: host.map(ConsoleFrame::span)`.
- The test helper `frame(addr: usize, ..)` (`:3134`) gets leaked one-page spans.
- With this change, every console read is bounded by the length of the span its own id names, so the inventory's PLAUSIBLE out-of-bounds read becomes a refusal **by structure**, whatever was published before.

**kf-qemu `broker.rs`:**
- `BrokerSeat::frame` (`:156-205`) makes `BrokerFrame`s through `broker_frame_over(&ram)`, and publishes one into a `ConsoleFrame` only after `ring.install` succeeds (`:189`).
- The collision and refusal paths (`:196`, `:199`) stay on `BrokerFrame::release`.

**kf-qemu `raw_unsafe.rs`.** `BrokerHooks` (`:257`, `derive(Debug)` over `opaque: *mut c_void` and two function pointers) gets a **manual Debug** with no fields. Otherwise G1b fails.

**kf-qemu `ffi_unsafe.rs`:**
- In `kf3_display_frame` (`:601-620`) and `Kf3Frame` (`:510`), apply this branch's `check_frame`, `len`, and the `-1`/`-2` split.
- The new cursor FFI (`Kf3Cursor` `:551`, `kf3_display_cursor` `:634`, `kf3_display_cursor_pixels` `:697`, `kf3_display_cursor_done` `:674`) carries no Rust-side address.
  - `kf3_display_cursor_pixels` takes a C pointer and word count, checked for null, alignment and `≤ 256×256`.
  - `kf3.c:749` must pass exactly `width × height` for a buffer from `cursor_alloc(width, height)`, with both clamped to `KF3_CURSOR_MAX_DIM` first. That is a C-perimeter obligation and a G3 presence row.

**`kf3.c` `kf3_console_frame` (`:773`; `kf3_gfx_update` `:795` is now its wrapper).** **Merge**, do not replace: keep v3-broker's cursor calls and their order (`kf3_console_cursor` after the frame, `:806`), and add `kf3_frame_ok`, `len`, the `-1`/`-2` handling and the placeholder inside `kf3_console_frame`.

**kf-linux-raw `MappedRegion::host_span` (`mapping_unsafe.rs:637`):**
- Keep it; this branch adds the same function.
- Mint through `HostSpan::within`, not with `len: usize::try_from(..).unwrap_or(0)` (`:640`), which bypasses `within`'s length-0 refusal.
- `ConsoleShare` accepts only a `StaticSpan`, so a broker frame cannot reach the console without `publish`.

**fd class (S1-08/S1-10).** Not required by this design:
- `kf_broker::Host::watch(fd: i32)`;
- `kf3_broker_frame_fd` (`ffi_unsafe.rs:825`), `kf3_broker_stop` (`:874`), `kf3_display_ui_info` (`:888`);
- `gpucopy.rs:77` `export_fd() -> i32` is replaced by the slot's `RmExport` (row above), whose `as_fd()` NVKMS borrows.

**`KF3_ABI`.** 12 is v3-broker's (still 12 at `82f98f42`, `ffi_unsafe.rs:31`, `kf3.h:23`), 13 is v3-dispsw-exp's, and 14 is this branch's. The second merger takes the maximum plus one **at merge time** and records every claim in both files.

**Ratchet after both merge.** Counted at `28b5b6fe` and again at `82f98f42`: kf-cuda 78, kf-linux-raw 94, kf-qemu 59. Re-count at the merge commit.
- **kf-cuda 67** = 66 + 1 (`host_unregister`). ⊘ *2026-10-04: 65 = 64 + 1.* v3-broker's `host_register` blocks fold into `register_console_pages`.
- **kf-qemu 59** = 46 + 13.
- **kf-linux-raw 94** = 75 + 19.

**Merge order.** Merge this branch **first**, unless v3-broker has already met its master bar. The reasons:
1. v3-broker carries the stale-address read hazard that §5 closes.
2. v3-broker's conversion list is mechanical, and its hardware rows must re-run at the merge commit anyway (§R testing rule, `OWNER_RULINGS.md:543-546`).
3. This branch's kf-cuda restructuring is the larger rebase target.

If v3-broker merges first, this branch converts the list above before its own merge bar, and does not merge until V13 covers the 5-slot ring.

---

## 10. Out of scope, explicitly

- **S1-01** (gate A's line-grep forbid inheritance; gate C's `*` comment filter): untouched. G1 uses its own lexer and does not depend on gate C.
- **S1-05 policy:**
  - No query or refusal of `CU_DEVICE_ATTRIBUTE_PAGEABLE_MEMORY_ACCESS`; owner §Q says HMM is not refused.
  - No in-kernel clamp in `kf_compose`, and no PTX or `.cu` edits, since those need GPU tests.
  - It **does** assume the worst case (every device address may be a QEMU address), with protection host-side (V5–V11), by T12, and by the PTX interpreter at merge.
- **S1-40 / S1-41, the RM funnel.**
  - Companion size fields and forged `NvP64` fields are a safe-caller contract at `CharDevice::ioctl` (`chardev_unsafe.rs:565-776`).
  - The fix is a per-request pointer/size table at the funnel (NV_ESC 0x2A, 0x2B, 0x27; offsets in `census.rs:88`), shared with S1-41's allowlist. It belongs to that branch.
  - G3 lists it as **OPEN**.
- **kf-linux-raw `pub(crate)` address getters**, `MappedRegion::addr_at` (`:790`) and `GuestWindow::userspace_addr_at` (`window_unsafe.rs:504`): reachable by kf-linux-raw's safe files and used only by `chardev_unsafe.rs`. They are OPEN rows in G3 and are reviewed with S1-40.
- **§R(b) beyond kernel launch arguments:** the GPU page-table-entry producers (kf-mem `apply`, kf-host twins) and the RM ioctl struct builders.
- **S1-07** (`RawRegion` `Copy`), **S1-08** (11 safe `extern "C" fn`), **S1-10** beyond the import path (`irq_fd`, `BackendFd`).
- **S1-13** (full `kf3.c` CI compile; only `kf3_frame_ok` compiles here), **S1-15**.
- **`ioctltrace::record` before the scrub** (`chardev_unsafe.rs:765`).
- **v2 `kayfabe-cuda`** (`driver_unsafe.rs:518-532`), which is not in the kf3 graph.
- **Code addresses:** function-pointer-to-integer casts and `Backtrace` `Debug` are not lexically detectable. They give code addresses, not data addresses, and are a residual.
- **UNVERIFIED and not relied on:**
  - whether `cuMemFree` or `cuMemFreeHost` wait for queued work (F1 drains explicitly);
  - whether `cuCtxDestroy` drains host functions (owners drain first and forget the fd otherwise).
- **UNVERIFIED and relied on:** `cuMemMap` refusing an over-long mapping (Res-1, H1, which blocks merging, with a specified fallback).

---

### Appendix: implementation order (one branch) and the merge bar

0. Merge `origin/master` (`789dee9f`).
1. **kf-linux-raw:**
   - `StaticSpan`, `MappedRegion::static_span` (with the backing rules) and `MappedRegion::host_span` (through `within`), plus a recorded `BackingKind`;
   - manual Debug for `HostSpan`, `Mapping`, `GuestWindow` and `RunMapping`;
   - the widened `as_ptr` contract;
   - trybuild rows, including the `!Hash`/`!Ord` rows.
2. **kf-cuda `abi.rs`:** the encoders and `KfFormat: Default`. Remove `view_bytes`, `zeroed`, `param_bytes` and `bytes_of`. Switch tests to `encode()`, and the `abi.rs:1156` row to `offset_of!` (T17).
3. **kf-cuda `driver_unsafe.rs`:**
   - the inline `mod raw`: handles, `ArgBlock`, import guards, `PinnedStage` with `Flight`, `KERNEL_SIGS`;
   - the four required reductions and deletion of dead code;
   - a private module (T1–T4, T18, T21, T22).
4. **`walk_gpu_unsafe.rs`:** moved code plus V5 (format check) through V8, `CapOf`, `poll` with poisoning; rewire `walk.rs`; `DeviceImage` (T5–T9, T20).
5. **`display_gpu_unsafe.rs`:** `DisplayGpu`, `Composer`, `ConsoleFrame` (register, then leak, with caps); reduce `display.rs` to data types and re-exports (T10–T13, T23).
6. **kf-qemu:** `ConsoleShare`, `FrameView` (no `PartialEq`), `ScanState`, the compose chain, `check_frame`, `Kf3Frame.len`, `kf3.h`/`kf3.c`, `KF3_ABI 14`, `wire_mirror`, the `as_fd` call sites, Debug impls (T14–T16).
7. **kf-util** re-keying (T24); then the kf-harness and kf-mem call sites.
8. **CI:**
   - `address_gate.py` with G1, G1b and G1c and their fixtures;
   - the `clippy.toml` G1d entries, with the perimeter `#![allow]`s and their fixture;
   - trybuild for kf-cuda and kf-qemu;
   - the census test;
   - the `AUDITED` value (66) and `PERIMETER` values, each with written reasons.
9. **Local cargo** only through the shared lock and a private target directory, deleted afterwards; CI is the build of record (`gh run list --branch v3-sec-rawaddr`).

**Before master** (§R testing rule): CI green, plus H1 (blocker), H2, H3 and H4 on a box with a real NVIDIA GPU, at the exact commit.

---

## 11. Implementation record (2026-10-04, branch `v3-sec-rawaddr`)

Everything in this section is at `cb4627f6` unless it names another commit. kf-cuda paths are under
`crates/kf-cuda/src/`. "Red" means the named test failed with the mutation applied and passed
without it; each mutation was applied by hand, run with `cargo test` on 2026-10-04, and reverted.

### 11.1 What landed, by commit

| Commit | What |
|---|---|
| `985ccd5d` | kf-linux-raw: `StaticSpan`, `MappedRegion::static_span` (V15) and `host_span` (through `HostSpan::within`), the recorded `BackingKind`, hand-written `Debug` for `HostSpan`, `StaticSpan`, `Mapping`, `GuestWindow`, `RunMapping`; the widened `as_ptr` contract; trybuild rows. kf-qemu: hand-written `Debug` for `RawRegion`, `OverlayHook`, `IoeventfdHook`. kf-util: census keys from text (`textkey.rs`), not addresses (T24). |
| `27e14d2c`, `d169503b` | kf-linux-raw trybuild rows: the forgery row spelled without a pointer type name; the `*_unsafe.rs`-suffixed fixture renamed (§6 G2 note). |
| `071afc8e` | kf-cuda: the private `driver_unsafe` module with the inline `raw` tier (every `unsafe` block), the children `walk_gpu_unsafe.rs`, `display_gpu_unsafe.rs`, `tests_unsafe.rs` (0 `unsafe`); `walk.rs`/`display.rs` reduced to logic and data; `abi.rs` encoders (`launch_codec!`); callers in kf-harness gates 2–9 and kf-qemu (`as_fd()`, `drop(img)`); `AUDITED` kf-cuda 75 → 64. |
| `d79d7b28` | The console frame: kf-qemu `ConsoleShare` span table and span ids, `FrameView { span, id, .. }`, `check_frame` (V13), `Kf3Frame.len`; `kf3.h` `kf3_frame_ok` (V14) and `kf3.c` `kf3_gfx_update` (`-1` keeps the surface, `-2` or a refused frame shows the placeholder once); `KF3_ABI 14`; `wire_mirror.rs` T14/T15. |
| `b2021f43` | CI: G1/G1b/G1c (`scripts/ci/address_gate.py`), G1d (`scripts/ci/address_clippy.sh`), kf-cuda and kf-qemu trybuild matrices, the export census (`tests/perimeter_exports.rs`), the `PERIMETER` ratchet, the perimeter opt-out attributes. |
| `d7dea977` | The census's presence row for `kf3_display_frame` names the item without the qualifier keyword (the unsafe-surface gate reads string literals). |
| `cb4627f6` | V2, V8 and V11 become pure functions with GPU-free tests T2, T9, T13. |

### 11.2 The perimeter at the tip

- **kf-cuda.** `lib.rs:36` declares `mod driver_unsafe` private. What leaves it is re-exported by
  name: `CUresult`, `CompletionFd`, `CudaError`, `driver_leaks` (`lib.rs:41`); `DeviceImage` and
  `kf_format_check` (`walk.rs:45`); the display handles (`display.rs:21`); and the layout tables
  `KF_ARGS_LAYOUT`, `KF_WIN_LAYOUT` (`abi.rs:250`). None of them carries an address. The raw tier is the inline
  `mod raw` at `driver_unsafe.rs:122`; its fields are private to it, so neither the safe files nor
  the children can build or read an address. The children reach the driver only through `raw`'s
  `pub(in crate::driver_unsafe)` methods, each of which calls its pure validator first.
- **Safe files** (`walk.rs`, `display.rs`, `abi.rs`, `capacity.rs`, …) hold offsets, lengths,
  geometry and opaque handles (`DeviceImage`, `ConsoleFrame`, `Composer`, `StaticSpan`). G1 and
  G1d report zero offenders in them.
- **kf-qemu.** The console carries `Option<StaticSpan>` plus a span id (`display.rs:457`); the span
  table is append-only (`register`, `display.rs:582`). The address is opened only inside
  `kf3_display_frame`'s existing `*out =` block (`ffi_unsafe.rs:582`), after `check_frame`
  (`:565`).
- **C.** `kf3_gfx_update` (`kf3.c:699`) re-checks with `kf3_frame_ok` (`kf3.h:58`), the same
  predicate as `check_frame_geometry` (`ffi_unsafe.rs:532`); T15 runs one grid through both.
- **Failure policies.** F1: `drop_disposition` (`driver_unsafe.rs:819`) frees only after a drain
  succeeded, otherwise forgets and counts (`driver_leaks`, `:113`). F2: a failed completion query
  poisons the walker (`WalkGpu::poll`, `walk_gpu_unsafe.rs:1209`). F3: at most 16 console frames
  of at most 64 MiB each (`display_gpu_unsafe.rs:30`).
- **The validation sites** are listed with their locations in the last column of §2.5.

### 11.3 Tests and the mutation run against each

| Row | Test (file) | Mutation, each red |
|---|---|---|
| T1 (V1) | `a_range_is_inside_its_allocation_or_refused` (`tests_unsafe.rs`) | `checked_add` → `wrapping_add`; `<=` → `<` |
| T1b (V1b) | `an_async_operation_stays_in_one_context` (`tests_unsafe.rs`) | `ctx_matches` always true |
| T2 (V2) | `an_import_maps_at_least_one_byte` (`tests_unsafe.rs`); a negative fd is not expressible (trybuild `import_takes_a_borrowed_fd`, renamed `import_takes_an_rm_export` in §11.7) | the `len == 0` arm deleted |
| T3 (V3) | `a_launch_is_refused_unless_every_argument_matches_the_pinned_signature` (`tests_unsafe.rs`) | count `!=` → `>`; the kind loop deleted; the capacity pairing dropped |
| T4 | `the_kernel_table_is_the_ptx` (`tests_unsafe.rs`) | one `KERNEL_SIGS` entry edited (`kf_compose`'s last `I32` → `U32`) |
| V4 | `the_stage_belongs_to_the_gpu_until_a_proof_returns_it` (T21), `the_stage_layout_is_aligned_disjoint_and_bounded` (`tests_unsafe.rs`) | an error clears the flight; the 64-byte alignment dropped; the empty-region refusal dropped; the report header made host-writable |
| T5 (V5) | `the_configuration_and_the_requirement_table_are_bounded` (`walk_gpu_unsafe.rs`) | the scratch bound loosened; a requirement check loosened by one byte |
| T6 | `the_walk_constants_are_the_cus` (`walk_gpu_unsafe.rs`; compiles `kf_walk.cu`'s structs with the host C++ compiler for `sizeof`) | `KF_PAR_GRID` edited |
| T7 (V6) | `a_stage_is_refused_past_any_pool_on_any_row` (`walk_gpu_unsafe.rs`) | rows checked only for `t < npdb` |
| T8 (V7) | `a_slot_move_stays_in_the_pool_and_never_overlaps` (`walk_gpu_unsafe.rs`) | the disjointness check deleted |
| T9 (V8) | `a_read_back_stays_inside_what_was_allocated` (`walk_gpu_unsafe.rs`) | the run clamp removed; the pool bound loosened by one run |
| T10 (V9) | `a_composition_is_bounded_on_both_sides` (`display_gpu_unsafe.rs`) | `> 16384` → `> 16385` |
| T11 (V10) | `a_compose_launch_is_bounded_before_it_is_queued` (`display_gpu_unsafe.rs`) | the `ox + width ≤ fw` arm dropped |
| T12 (S1-05 b) | `the_compose_address_function_stays_inside_every_accepted_layer` (`display_gpu_unsafe.rs`; seeded sweep plus boundary layers, CPU model of `kf_scanout.ptx` with `kf_disp::scanout::bl_offset`) | `need` one byte short |
| T13 (V11) | `a_composition_finishes_only_into_a_frame_that_holds_it` (`display_gpu_unsafe.rs`) | `>` → `>=`; the context compare dropped; the bound loosened by one byte |
| T14 (V13) | `the_frame_check_is_one_predicate_on_both_sides` (kf-qemu `tests/wire_mirror.rs`) | the `len` arm deleted; the old predicate (stride in pixels) |
| T15 (V14) | the same test, compiling `kf3.h` with `-Wall -Wextra -Werror` | the C `stride % 4` arm dropped |
| T16 | `the_console_is_handed_only_a_geometry_its_frame_holds` (kf-qemu `display.rs:2671`) | publish the geometry without the id; `check_frame` ignores the span length |
| T17 | `every_launch_struct_encodes_to_the_c_compilers_bytes`, `the_rust_ver2/ver3_descriptor_matches_the_cu_byte_for_byte` (`tests/walk_abi_matches_the_cu.rs`) | the encoder writes big-endian |
| T18 | `no_handle_derives_a_pointer_printing_debug` (`tests_unsafe.rs`); `no_debug_output_carries_the_address` (kf-linux-raw) | `derive(Debug)` on `ConsoleDst`; `HostSpan`'s `Debug` prints the base |
| T19 | `address_gate.py --self-test`, `test_address_gate.py`, `address_clippy.sh` self-test, the trybuild `REQUIRED_ROWS` tests, `the_census_finds_an_unlisted_export` | G1 P1 removed; P7 removed; the G1b `NonNull` arm removed; `AtomicPtr` dropped from G1c; G1d: the opt-out removed from `mapping_unsafe.rs` (13 `disallowed_*` errors); G3: a row deleted, an export added without a row, a row's test renamed |
| T20 (V5) | `the_format_check_refuses_every_arm_of_the_cus` (`walk_gpu_unsafe.rs`) | the `ap_bits` arm deleted |
| T21 (V4) | `the_stage_belongs_to_the_gpu_until_a_proof_returns_it` (`tests_unsafe.rs`) | an error clears the flight |
| T22 (F1) | `a_failed_drain_leaks_and_never_frees` (`tests_unsafe.rs`) | free on a failed drain |
| T23 (F3, V12) | `console_frames_are_capped_in_count_and_size` (`display_gpu_unsafe.rs`) | the frame-count cap dropped |
| T24 | `keys_follow_the_text_and_the_line_not_an_address` (kf-util `textkey.rs`) | `site_key` keyed on the line only |
| V15 | `only_owned_private_anonymous_memory_becomes_a_static_span` (kf-linux-raw `mapping_unsafe.rs`) | `&&` → `\|\|` in `static_span_allowed` |

Compile-time rows (G2), all in CI's test step: kf-cuda `tests/ui/` (10 rows, §6 G2), kf-linux-raw
`host_span_not_forgeable`, `span_address_needs_an_unsafe_block`, `span_is_not_hash`,
`span_is_not_ord`, `static_span_needs_static`, and kf-qemu `frame_view_has_no_address`.
`tests/walk_never_synchronizes.rs` pins the walker's drains to exactly two sites (the probe and
the failed-enqueue recovery).

### 11.4 Counts

⊘ **After the second review (§11.7):** `unsafe` unchanged (75 / 64 / 46); `PERIMETER` kf-linux-raw
4810 → **4836**, kf-cuda 4493 → **4864**, kf-qemu **658**, each step itemised in `ci.yml`
(`PERIMETER` at `:1980`). The table below is the first implementation's.

| Crate | `unsafe` (CI regex) before → after | Perimeter lines (`PERIMETER`) before → after |
|---|---|---|
| kf-linux-raw | 75 → **75** | 4596 → **4810** |
| kf-cuda | 75 → **64** | 1084 → **4493** |
| kf-qemu | 46 → **46** | 571 → **658** |

Both are compared exactly in CI and itemised in `ci.yml` (`AUDITED` at `:1569`, `PERIMETER` at
`:1959`). `walk_gpu_unsafe.rs`, `display_gpu_unsafe.rs` and `tests_unsafe.rs` hold 0 `unsafe`.

### 11.5 Deviations from the design, and why

1. **kf-cuda 64, not 66** (§7 note): two more blocks were avoidable under §R.
2. **The stage's flight is set by `raw`, not by a `begin_flight()` call** (§2.2 note): a call the
   child must remember is a caller precondition; `raw` marks the stage `InFlight` at the launch
   that reads it.
3. **Device memory is reference-counted** (§2.2 note): a recorded graph or an `ArgBlock` names
   memory after the handle that minted it is gone; the clone keeps it alive, and the last drop
   still drains first (F1). `DeviceImage` holds a `DevMem` for the same reason.
4. **`poll` asks the copy event when the fd has fired**, and the done event otherwise, so a stream
   whose host function never runs is named instead of waited on (`walk_gpu_unsafe.rs:1209`).
5. **`Drained` carries its context id**, so a proof from one context cannot end another's flight.
6. **`console_pages` maps the pages itself** (§5 note): the child never holds an unregistered
   mapping, and the register-then-leak order is inside one function.
7. **G1d is its own CI step with its own `clippy.toml`** (§6 G1d note), scoped to the kf-* crates:
   the workspace configuration applies to every crate, including the v2 crates and the firmware,
   which this design does not cover.
8. **Fixtures are `*.rs.txt`** (§6 G1 note), and the kf-linux-raw trybuild row was renamed
   (§6 G2 note): both keep the tree's own gates from reading fixtures as sources.
9. **T18 is a source scan** (§8 note): most handles cannot be constructed without a GPU.
10. **`PERIMETER` kf-cuda is 4493, not about 2,100** (§6 G4 note): the validators' tests sit in
    the perimeter files.
11. **Census rows whose check only a GPU can exercise** name hardware row H4 instead of a unit
    test (`tests/perimeter_exports.rs`); the census still requires every exported function to have
    a row.
12. **`ConsoleFrame::read` returns `Result`** and refuses by name outside the frame, and the
    display counters gained `console_refused` (kf-qemu `display.rs:404`), so a refused frame is
    counted rather than only logged.
13. **kf-util's probe mixing changed with the key** (§3 row said "unchanged"): the new site key
    puts the line in its high bits, which the old probe start ignored, so distinct lines of one file
    collided until the table filled. The probe now starts from the `splitmix64` finaliser of the
    whole key (`textkey.rs:64`).
14. **V2, V8 and V11 were inline checks in the first implementation** (`071afc8e`) and became pure
    functions with their own tests in `cb4627f6`, as §2.5 requires.

### 11.6 Not done

- **Hardware rows H1–H4 (§8) have not run.** ⊘ *Updated after the second review:* H1 is no longer
  a merge blocker (the `RmExport` token is implemented, §2.6 note), and H3 and H4's self-test are
  now steps of `kf-gate9`, so they run wherever the gates do; none of it has run on a GPU yet.
  (Original text:) H1 (the import length, Res-1) is a merge blocker,
  with the `RmExport` fallback specified in §2.6. H2 (registering private-anonymous console pages),
  H3 (`DeviceImage` lifetime in kf-gate9) and H4 (gates 2–9, the self-test, the display and gop
  rows) must run on a real NVIDIA GPU at the exact merge commit (§R testing rule).
- **OPEN rows (§10), listed in the census so they are not read as covered:** `CharDevice::ioctl`
  (S1-40/S1-41), `MappedRegion::addr_at` and `GuestWindow::userspace_addr_at`.
- **The v3-broker conversion (§9)** belongs to whichever branch merges second.

### 11.7 The second review (2026-10-04): findings, fixes, tests, mutations

Two reviewers, 14 findings (one high, four medium, nine low); all fixed on the branch, none shown
wrong. "Red" as in §11: the named test failed with the mutation and passed without it; each
mutation was applied, run with `cargo test` (or the gate's self-test), and reverted.

| # | Finding | Fix (commit) | Test | Mutation, each red |
|---|---|---|---|---|
| 1 (high) | V10's block-linear bound `blocks · pitch · 512·2^bh` was unchecked; release builds wrap it (`y0 = u32::MAX, pitch = 2^26` → 0), so a 4-byte extent was accepted for a read 2^64 − 2^35 bytes past `src` | checked products, and a refusal of rows/columns past the kernel's 32-bit coordinates (`display_gpu_unsafe.rs:170`); kf-disp's two products checked too (`scanout.rs:242`, `:300`) (`721bc69c`) | T11 overflow and coordinate rows; T12's tail (100 000 layers near the top of every type, PTX wrapping model, extents over all of `u64`); kf-disp `an_overflowing_surface_is_refused_not_wrapped` | checked → wrapping (T11, and T12's tail by a read, not a panic); each half of the coordinate arm; each kf-disp product wrapping |
| 2 (med) | the import length was a caller's `u64` | the `kf_host::RmExport` token (§2.6 note) (`951a84a4`, stderr `1b4ffca6`) | kf-host `an_export_length_comes_only_from_the_sessions_record`; trybuild `import_takes_an_rm_export`, `rm_export_is_minted_only_by_kf_host` | the record not dropped on free; `store_bytes` answering any handle; the token's fields `pub`, and the old `(BorrowedFd, u64)` signature (each trybuild case then compiles) |
| 3 (med) | `{:p}` split over lines by `concat!`, `<&u8 as Pointer>::fmt` after `use core::fmt::*`, and `use core::ptr::{hash}` passed G1 (and the first two G1d) | G1 reads statements: joined literals (`stringify!` as a literal), P7c for `env!`/`include_str!` in a format `concat!`, `use` statements whole (`address_gate.py:222`); P6 is the `Pointer` token; G1d refuses `core::fmt::Pointer::fmt` (`8bf5dd94`) | `positive_statements.rs.txt` (one marked line per shape); `test_address_gate.py`; the G1d fixture's 28th entry | each new rule off (the fixture line MISSED); the G1d entry removed (27 of 28) |
| 4 (low) | G1b missed `{graph, exec: usize}`, `handle: u64`, `addr: DevAddr`; T18 likewise | G1b: `DevAddr`, handle names, and in kf-cuda any `…addr`/`…base` integer (`address_gate.py:281`, `:291`); T18's list widened. It found a real one: kf-linux-raw `UserspaceMemoryRegion` derived `Debug` over `userspace_addr`, a host address — now hand-written (`kvm_unsafe.rs:129`) (`8bf5dd94`) | `perimeter_debug{,_cuda}.rs.txt`; `the_memslot_record_never_prints_its_host_address`; T18 | the name widening, the `DevAddr` arm and the kf-cuda rule each off; the memslot `Debug` printing the field; T18 against derives over `{graph, exec}` and `{handle}` |
| 5 (low) | four V10 arms had no row (oy, bh, `x0 % 4`, MAX_SIDE) | one row per arm, each with room everywhere else (`721bc69c`) | T11 | each arm deleted |
| 6 (low) | `ConsoleFrame::read` allocated before its bound, and left completion to its caller | `frame_read_fits` first (`display_gpu_unsafe.rs:127`); the doc states the read is bounds-only with racy content (Res-3) (`6a419c29`) | T26 `a_frame_read_is_bounded_before_it_allocates` | the `end <= len` arm dropped |
| 7, 9 (low, med) | §9 stale against v3-broker | §9 re-derived at `82f98f42` (this commit) | — | — |
| 8 (low) | `Drained` was not tied to the flight it ended | a flight generation: `begin` advances it, `Event::record_flight` stamps the event (a graph re-stamps per launch), `end_flight` refuses an older proof (`driver_unsafe.rs:497`, `:1382`, `:2366`) (`6a419c29`) | T21b `a_completion_proof_of_an_earlier_flight_is_refused` | `proof_covers` always true; `end` without the cover check; `begin` not advancing; `end` without the context check |
| 10 (med) | H3 and H4's self-test had nothing that ran them | both are steps of `kf-gate9` (`:581`, `:653`) (`cd93271d`) | kf-harness `merge_bar_rows` (no GPU); the steps themselves on hardware | `main` without `selftest`; H3 without the drop before its wait |
| 11 (low) | an image its caller dropped was freed inside the next submit, after `cuGraphLaunch`, so its drain waited for that walk | the retained window is swapped before anything is queued (`walk_gpu_unsafe.rs:1153`) (`6a419c29`) | `the_previous_window_is_released_before_the_walk_is_queued`; H3's one-drain check | the release moved after the enqueue; the old assignment restored |
| 12 (low) | a stale graph launch re-set every node; `poll` queries an event per collect | only block-reading or regridded nodes are re-set, the grid recorded after the driver took it (`node_needs_rewrite`, `driver_unsafe.rs:2060`); the query stays — it mints the flight's proof — and its cost is an H4 reading (`6a419c29`) | T25 `a_stale_graph_rewrites_only_the_nodes_that_change` | always re-set; blind to a grid change |
| 13 (low) | the census keys hashed bytes on the trap path | both keys read a fixed number of words (`textkey.rs:49`, `:65`); a standalone `rustc -O` loop on the workstation timed site_key 13.8 → 2.5 ns and str_key 41 → 2.8 ns (loop floor 1.2 ns); gate 8's `trigger_trap` is an H4 reading (`be8ab402`) | `the_reason_key_reads_a_bounded_number_of_bytes`; T24 | `str_key` back to a full-text loop; the site key's tail digest zeroed |
| 14 (low) | the import handle was released right after `cuMemMap`, never run on an RM export | held in `AllocKind::Import` until after the unmap, as before the perimeter (`951a84a4`) | H4 reading (a) | — (hardware) |

Counts after the review are in §11.4. kf-cuda's `unsafe` count did not move (64).
