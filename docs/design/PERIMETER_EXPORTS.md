# Perimeter exports — the reviewed table (OWNER_RULINGS §R gate 3)

STATUS: LIVE, 2026-10-04. Generated inventory must equal this table (`scripts/ci/perimeter.py exports`).
<!-- perimeter-exports format=2 rustdoc-format=61 -->

Every item a perimeter file (`*_unsafe.rs` of a class U crate in kf3's graph) exports to safe code is a row: the checks it performs on its own inputs, and the test that shows each check (`label=t:<file>::<test fn>`). Grammar, rules E1-E14 and how a row becomes `OK`: `docs/design/V3_SEC_PERIMETER.md` §3. The OPEN count is printed by the gate, never stored.

**Crates covered:** `crates/kf-cuda`, `crates/kf-linux-raw`, `crates/kf-qemu`. The grader crates and the guest firmware are not linked into kf3 (scripts/ci/dependencies.py); their perimeter files are counted in `scripts/ci/perimeter/sizes.tsv` and have no rows here.

## crates/kf-cuda/src/driver_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `<CompletionFd as AsFd>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<CompletionFd as AsFd>::as_fd` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<CtxHandle as Clone>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<CtxHandle as Clone>::clone` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<CtxHandle as Copy>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Cuda as Send>` | default | unsafe trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Cuda as Sync>` | default | unsafe trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<CudaError as Clone>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<CudaError as Clone>::clone` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<CudaError as Display>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<CudaError as Display>::fmt` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<EventHandle as Clone>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<EventHandle as Clone>::clone` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<EventHandle as Copy>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Func as Clone>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Func as Clone>::clone` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Func as Copy>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<GraphExecHandle as Clone>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<GraphExecHandle as Clone>::clone` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<GraphExecHandle as Copy>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<GraphHandle as Clone>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<GraphHandle as Clone>::clone` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<GraphHandle as Copy>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<GraphNode as Clone>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<GraphNode as Clone>::clone` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<GraphNode as Copy>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<ModuleHandle as Clone>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<ModuleHandle as Clone>::clone` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<ModuleHandle as Copy>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<StreamHandle as Clone>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<StreamHandle as Clone>::clone` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<StreamHandle as Copy>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CUDA_ERROR_NOT_READY` | pub | const |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CUDA_SUCCESS` | pub | const |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CUdeviceptr` | pub | type alias |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CUresult` | pub | type alias |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CompletionFd` | pub | owning handle |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CompletionFd: Send` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CompletionFd: Sync` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CompletionFd::drain` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CompletionFd::new` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CompletionFd::raw` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CompletionFd::signal` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CompletionFd::wait_readable` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CtxHandle` | pub | owning handle |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CtxHandle: Send` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CtxHandle: Sync` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CtxHandle::is_null` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CtxHandle::null` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda` | pub | owning handle |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::capture_leaf` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::check` | pub(crate) | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::ctx_create` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::ctx_destroy` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::ctx_set_current` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::ctx_sync_calls` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::ctx_synchronize` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::device_by_pci_bus_id` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::device_count` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::device_get` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::device_name` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::event_create` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::event_destroy` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::event_elapsed_us` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::event_query` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::event_record` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::event_record_external` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::export_device_allocation` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::graph_destroy` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::graph_exec_destroy` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::graph_exec_kernel_set` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::graph_instantiate` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::graph_launch` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::graph_upload` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::has_graph_api` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::has_vmm_api` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::import_and_map` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::init` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::launch` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::launch_args` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::launch_host_signal` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::launch_raw` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::mem_alloc_zeroed` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::mem_free` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::memcpy_d2d_async` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::memcpy_d2h` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::memcpy_d2h_async` | pub(crate) | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::memcpy_h2d` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::memcpy_h2d_async` | pub(crate) | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::memset_d8` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::memset_d8_async` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::module_function` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::module_load` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::open` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::open_soname` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::pinned_alloc` | pub(crate) | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::pinned_device_ptr` | pub(crate) | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::pinned_free` | pub(crate) | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::stream_begin_capture` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::stream_create` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::stream_destroy` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Cuda::stream_end_capture` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `CudaError` | pub | plain data |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `EventHandle` | pub | owning handle |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `EventHandle: Send` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `EventHandle: Sync` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `ExportedAllocation` | pub | owning handle |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `ExportedAllocation: Send` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `ExportedAllocation: Sync` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Func` | pub | borrowed view |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Func: Send` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Func: Sync` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `GraphExecHandle` | pub | owning handle |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `GraphExecHandle: Send` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `GraphExecHandle: Sync` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `GraphHandle` | pub | owning handle |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `GraphHandle: Send` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `GraphHandle: Sync` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `GraphNode` | pub | borrowed view |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `GraphNode: Send` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `GraphNode: Sync` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `KernelNodeParams` | pub(crate) | FFI struct |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `LIBCUDA_SONAME` | pub | const |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `ModuleHandle` | pub | owning handle |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `ModuleHandle: Send` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `ModuleHandle: Sync` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `PinnedBuf` | pub(crate) | owning handle |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `PinnedBuf: Send` | pub(crate) | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `PinnedBuf: Sync` | pub(crate) | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `PinnedBuf::addr` | pub(crate) | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `PinnedBuf::len` | pub(crate) | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `PinnedBuf::read` | pub(crate) | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `PinnedBuf::write` | pub(crate) | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `PollFd: Send` | pub(in ::driver_unsafe) | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `PollFd: Sync` | pub(in ::driver_unsafe) | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `StreamHandle` | pub | owning handle |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `StreamHandle: Send` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `StreamHandle: Sync` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `StreamHandle::legacy` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `completion_hostfn` | pub(crate) | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `view_bytes` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `zeroed` | pub | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |

## crates/kf-linux-raw/src/affinity_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `current_cores` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `pin_current_thread` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |

## crates/kf-linux-raw/src/chardev_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `<DevAccess as Clone>` | default | trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `<DevAccess as Clone>::clone` | default | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `<DevAccess as Copy>` | default | trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `CharDevice` | pub | owning handle |  |  | OPEN: 2026-10-04: unreviewed |
| `CharDevice: Send` | pub | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `CharDevice: Sync` | pub | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `CharDevice::adopt` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `CharDevice::as_fd` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `CharDevice::fd_number` | pub | safe fn |  |  | OPEN: 2026-10-04: returns the descriptor as an i32 (S1-10); `BorrowedFd` at P2, after v3-sec-nonpriv, v3-broker and v3-dispsw-exp (13 callers, §4.3) |
| `CharDevice::ioctl` | pub | safe fn |  |  | OPEN: 2026-10-04: request-agnostic; argument bytes may carry kernel-dereferenced address fields no Indirect patched (finding ii); closes with the typed RM layer (V3_SEC_PERIMETER.md §5.2); `declared == 0` refused outside LEGACY_SIZES at a1 |
| `CharDevice::openat` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `CharDevice::openat_mode` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `CharDevice::surrender` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `DevAccess` | pub | plain data |  |  | OPEN: 2026-10-04: unreviewed |
| `DevDir` | pub | owning handle |  |  | OPEN: 2026-10-04: unreviewed |
| `DevDir: Send` | pub | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `DevDir: Sync` | pub | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `DevDir::as_fd` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `DevDir::can_reach` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `DevDir::from_fd` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `DevDir::open` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `DevDir::try_clone` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `Indirect` | pub | borrowed view |  |  | OPEN: 2026-10-04: unreviewed |
| `Indirect::at` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `Indirect::describing` | pub | safe fn | zero-len; range-in-region | zero-len=t:crates/kf-linux-raw/src/chardev_unsafe.rs::a_described_range_past_the_region_is_refused_at_construction; range-in-region=t:crates/kf-linux-raw/src/chardev_unsafe.rs::a_described_range_past_the_region_is_refused_at_construction | OK |
| `Indirect::is_empty` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `Indirect::len` | pub | safe fn |  |  | OPEN: 2026-10-04: carries a caller contract (`chardev_unsafe.rs:365-377`, S1-40); removed by P2-a2 |
| `Indirect::nested` | pub | safe fn |  |  | OPEN: 2026-10-04: the nested size field is caller-declared like Indirect::new's (S1-40) |
| `Indirect::new` | pub | safe fn |  |  | OPEN: 2026-10-04: the size field's value and location are caller-declared (S1-40); the value half closes with P2-a2 after v3-broker, the location half with §5.2 |
| `POINTER_FIELD_WIDTH` | pub | const |  |  | OPEN: 2026-10-04: unreviewed |

## crates/kf-linux-raw/src/epoll_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `<PollTimeout as Clone>` | default | trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `<PollTimeout as Clone>::clone` | default | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `<PollTimeout as Copy>` | default | trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `<ReadyTokens as Default>` | default | trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `<ReadyTokens as Default>::default` | default | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `MAX_READY_BATCH` | pub | const |  |  | OPEN: 2026-10-04: unreviewed |
| `PollTimeout` | pub | plain data |  |  | OPEN: 2026-10-04: unreviewed |
| `Poller` | pub | owning handle |  |  | OPEN: 2026-10-04: unreviewed |
| `Poller: Send` | pub | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `Poller: Sync` | pub | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `Poller::create` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `Poller::unwatch` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `Poller::wait` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `Poller::watch` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `ReadyTokens` | pub | plain data |  |  | OPEN: 2026-10-04: unreviewed |
| `ReadyTokens::is_empty` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `ReadyTokens::iter` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `ReadyTokens::len` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `ReadyTokens::new` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |

## crates/kf-linux-raw/src/host_fd_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `Notifier` | pub | owning handle |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Notifier: Send` | pub | auto trait |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Notifier: Sync` | pub | auto trait |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Notifier::as_source_fd` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Notifier::create` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Notifier::drain` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Notifier::signal` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Notifier::signal_under_lock` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `SharedRam` | pub | owning handle |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `SharedRam: Send` | pub | auto trait |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `SharedRam: Sync` | pub | auto trait |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `SharedRam::as_backing_fd` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `SharedRam::create` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `SharedRam::create_named` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `SharedRam::dup_for_export` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `SharedRam::len_bytes` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `adopt_fd` | pub(crate) | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `descriptor_budget` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |

## crates/kf-linux-raw/src/kvm_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `<KvmMemslot as Drop>` | default | trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `<KvmMemslot as Drop>::drop` | default | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `CoalescedZone: Send` | pub(in ::kvm_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `CoalescedZone: Sync` | pub(in ::kvm_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `IoEventFd: Send` | pub(in ::kvm_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `IoEventFd: Sync` | pub(in ::kvm_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `Kvm` | pub | owning handle |  |  | OPEN: 2026-10-04: unreviewed |
| `Kvm: Send` | pub | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `Kvm: Sync` | pub | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `Kvm::borrow_fd` | pub(crate) | safe fn |  |  | OPEN: 2026-10-04: a borrowed descriptor for ioctl_arg (a3), crate-private; not yet mutation-proved |
| `Kvm::create_vm` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `Kvm::open` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmMemslot` | pub | owning handle |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmMemslot: Send` | pub | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmMemslot: Sync` | pub | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmMemslot::gpa` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmMemslot::install` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmMemslot::len_bytes` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmMemslot::slot` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmMemslot::window` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVm` | pub | owning handle |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVm: Send` | pub | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVm: Sync` | pub | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVm::adopt` | pub | safe fn | is-a-vm | is-a-vm=t:crates/kf-linux-raw/src/kvm_unsafe.rs::adopting_a_descriptor_that_is_not_a_vm_is_refused | OK |
| `KvmVm::borrow_fd` | pub(crate) | safe fn |  |  | OPEN: 2026-10-04: a borrowed descriptor for ioctl_arg (a3), crate-private; not yet mutation-proved |
| `KvmVm::check_extension` | pub(crate) | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVm::clear_memslot` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVm::discover_in_this_process` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVm::has_ioeventfd` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVm::ioeventfd` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVm::max_memslots` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVm::register_coalesced_mmio` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVm::try_clone_descriptor` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `UserspaceMemoryRegion: Send` | pub(in ::kvm_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `UserspaceMemoryRegion: Sync` | pub(in ::kvm_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `ioctl_arg` | pub(crate) | safe fn | io-only; negative-is-error | io-only=t:crates/kf-linux-raw/src/kvm_unsafe.rs::an_ior_encoded_request_is_refused_before_any_syscall; negative-is-error=t:crates/kf-linux-raw/src/kvm_unsafe.rs::an_ior_encoded_request_is_refused_before_any_syscall | OK |

## crates/kf-linux-raw/src/mapping_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `<AtomicU32 as AtomicWord>` | default | trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<AtomicU64 as AtomicWord>` | default | trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Backing as Clone>` | default | trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Backing as Clone>::clone` | default | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Backing as Copy>` | default | trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<HostProt as Clone>` | default | trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<HostProt as Clone>::clone` | default | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<HostProt as Copy>` | default | trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<HostSpan as Clone>` | default | trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<HostSpan as Clone>::clone` | default | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<HostSpan as Copy>` | default | trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<HostSpan as Send>` | default | unsafe trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<HostSpan as Sync>` | default | unsafe trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<HugePageReport as Clone>` | default | trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<HugePageReport as Clone>::clone` | default | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<HugePageReport as Copy>` | default | trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<MappedRegion as Send>` | default | unsafe trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<PlacementId as Clone>` | default | trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<PlacementId as Clone>::clone` | default | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<PlacementId as Copy>` | default | trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<VolatileRegion as Send>` | default | unsafe trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `<VolatileRegion as Sync>` | default | unsafe trait impl |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Backing` | pub | borrowed view |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Backing: Send` | pub | auto trait |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Backing: Sync` | pub | auto trait |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Backing::attainable_cache_policy` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `HostProt` | pub | plain data |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `HostSpan` | pub | borrowed view |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `HostSpan::as_ptr` | pub | unsafe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `HostSpan::is_empty` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `HostSpan::len` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `HostSpan::within` | pub(crate) | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `HugePageReport` | pub | plain data |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `MappedRegion` | pub | owning handle |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `MappedRegion::addr_at` | pub(crate) | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `MappedRegion::cache_policy` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `MappedRegion::is_writable` | pub(crate) | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `MappedRegion::len_bytes` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `MappedRegion::map` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `MappedRegion::read_into` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `MappedRegion::reprotect` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `MappedRegion::request_huge_pages` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `MappedRegion::slice` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `MappedRegion::stitch` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `MappedRegion::write_from` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Placement: Send` | pub(in ::mapping_unsafe) | auto trait |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `PlacementId` | pub | plain data |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Reservation` | pub | owning handle |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Reservation::len_bytes` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Reservation::map_fixed_in` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Reservation::new` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `Reservation::placement` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `VolatileRegion` | pub | owning handle |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `VolatileRegion::cache_policy` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `VolatileRegion::copy_out` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `VolatileRegion::host_span` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `VolatileRegion::len_bytes` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `VolatileRegion::load_u32` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `VolatileRegion::load_u64` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `VolatileRegion::map` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `VolatileRegion::store_u32` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `VolatileRegion::store_u64` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |
| `release_fence` | pub | safe fn |  |  | LANE:v3-broker: 2026-10-04: unreviewed; this file is edited by that lane |

## crates/kf-linux-raw/src/signal_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `<ThreadId as Clone>` | default | trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `<ThreadId as Clone>::clone` | default | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `<ThreadId as Copy>` | default | trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `BREAK_SIGNAL` | pub | const |  |  | OPEN: 2026-10-04: unreviewed |
| `ThreadId` | pub | plain data |  |  | OPEN: 2026-10-04: unreviewed |
| `ThreadId::raw` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `current_thread_id` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `install_break_handler` | pub | safe fn |  |  | OPEN: 2026-10-04: installs on SIGUSR1, which is QEMU's SIG_IPI (qemu-10.2.4 include/qemu/main-loop.h:32); no kf-* caller (a11) |
| `interrupt_thread` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |

## crates/kf-linux-raw/src/sysconf_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `sc_pagesize` | pub(crate) | safe fn |  |  | OPEN: 2026-10-04: unreviewed |

## crates/kf-linux-raw/src/vcpu_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `<KvmVcpu as Send>` | default | unsafe trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `<VcpuExit as Clone>` | default | trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `<VcpuExit as Clone>::clone` | default | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `<VcpuExit as Copy>` | default | trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmDtable: Send` | pub(in ::vcpu_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmDtable: Sync` | pub(in ::vcpu_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmMmioExit: Send` | pub(in ::vcpu_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmMmioExit: Sync` | pub(in ::vcpu_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmSegment: Send` | pub(in ::vcpu_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmSegment: Sync` | pub(in ::vcpu_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmSregs: Send` | pub(in ::vcpu_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmSregs: Sync` | pub(in ::vcpu_unsafe) | auto trait |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVcpu` | pub | owning handle |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVcpu::complete_mmio_read` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVcpu::create` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVcpu::enter_flat_protected_mode` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVcpu::run` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `KvmVm::set_tss_addr_if_supported` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `VcpuExit` | pub | plain data |  |  | OPEN: 2026-10-04: an MMIO exit longer than its 8-byte data is a refused exit since a6 (`mmio_exit`, test `an_mmio_length_beyond_the_data_array_is_a_refused_exit`); not yet reviewed as a type |

## crates/kf-linux-raw/src/window_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `<GuestWindow as Drop>` | default | trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `<GuestWindow as Drop>::drop` | default | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `<GuestWindow as Send>` | default | unsafe trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `<GuestWindow as Sync>` | default | unsafe trait impl |  |  | OPEN: 2026-10-04: unreviewed |
| `GuestWindow` | pub | owning handle |  |  | OPEN: 2026-10-04: unreviewed |
| `GuestWindow::create` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `GuestWindow::host_span` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `GuestWindow::len_bytes` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `GuestWindow::page_size` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `GuestWindow::place` | pub | safe fn |  |  | OPEN: 2026-10-04: a8 re-plugs or poisons after a failed placement and a10 refuses a placement past end-of-file (tests in this file); OPEN residuals: a file truncated after placement (needs F_SEAL_SHRINK), and a foreign anonymous mapping merged into the filler is indistinguishable from it; not yet mutation-proved |
| `GuestWindow::place_device_view` | pub | safe fn |  |  | OPEN: 2026-10-04: a9 refuses a read-only view and a8 re-plugs or poisons after a failed placement; whether NVIDIA device mappings survive mremap is unmeasured (box bar), so device views keep the single MAP_FIXED; not yet mutation-proved |
| `GuestWindow::read_into` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `GuestWindow::restore` | pub | safe fn |  |  | OPEN: 2026-10-04: a8 re-plugs or poisons after a failed MAP_FIXED; not yet mutation-proved |
| `GuestWindow::store_u32` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |
| `GuestWindow::userspace_addr_at` | pub(crate) | unsafe fn |  |  | OPEN: 2026-10-04: an `unsafe fn` since a7, its `# Safety` stating the lifetime contract; its only test is KVM-gated (E3d), so CI holds no evidence for it |
| `GuestWindow::write_from` | pub | safe fn |  |  | OPEN: 2026-10-04: unreviewed |

## crates/kf-qemu/src/ffi_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `<Kf3Frame as Clone>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Kf3Frame as Clone>::clone` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Kf3Frame as Copy>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Kf3Identity as Clone>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Kf3Identity as Clone>::clone` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Kf3Identity as Copy>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Kf3Identity as Default>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Kf3Identity as Default>::default` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Kf3Region as Clone>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Kf3Region as Clone>::clone` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Kf3Region as Copy>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Kf3Region as Default>` | default | trait impl |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `<Kf3Region as Default>::default` | default | safe fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `KF3_ABI` | pub | const |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Kf3Frame` | pub | FFI struct |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Kf3Identity` | pub | FFI struct |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Kf3Region` | pub | FFI struct |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Kf3Region: Send` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `Kf3Region: Sync` | pub | auto trait |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_abi_version` | pub | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_bar0_read` | pub | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_bar0_write` | pub | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_bar1_follows_guest` | pub | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_bar1_overlay_done` | pub | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_bar1_usermode_write` | pub | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_bar_ram` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_config_word` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_display_frame` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_doorbell_page_offset` | pub | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_doorbell_site` | pub | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_identity` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_irq_fd` | pub | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_memory_map` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_option_rom` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_ram_add` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_ram_del` | pub | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_realize` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_set_bar1_overlay` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_set_ioeventfd` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_shadow_attach` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_shadow_seal` | pub | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_status` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_unrealize` | pub | safe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |
| `kf3_usermode_view` | pub | unsafe extern fn |  |  | LANE:v3-sec-rawaddr: 2026-10-04: unreviewed; this file is edited by that lane |

## crates/kf-qemu/src/raw_unsafe.rs

| item | vis | kind | checks | tests | status |
|---|---|---|---|---|---|
| `<BackendFd as Clone>` | default | trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<BackendFd as Clone>::clone` | default | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<BackendFd as Copy>` | default | trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<IoeventfdHook as Clone>` | default | trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<IoeventfdHook as Clone>::clone` | default | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<IoeventfdHook as Copy>` | default | trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<IoeventfdHook as Ioeventfd>` | default | trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<IoeventfdHook as Ioeventfd>::set` | default | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<IoeventfdHook as Send>` | default | unsafe trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<IoeventfdHook as Sync>` | default | unsafe trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<OverlayHook as Clone>` | default | trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<OverlayHook as Clone>::clone` | default | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<OverlayHook as Copy>` | default | trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<OverlayHook as Send>` | default | unsafe trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<OverlayHook as Sync>` | default | unsafe trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<RawRegion as Clone>` | default | trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<RawRegion as Clone>::clone` | default | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<RawRegion as Copy>` | default | trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<RawRegion as Send>` | default | unsafe trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `<RawRegion as Sync>` | default | unsafe trait impl |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `BackendFd` | pub | borrowed view |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `BackendFd: Send` | pub | auto trait |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `BackendFd: Sync` | pub | auto trait |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `BackendFd::adopt` | pub | unsafe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `BackendFd::borrow` | pub | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `IoeventfdFn` | pub | type alias |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `IoeventfdHook` | pub | borrowed view |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `IoeventfdHook::adopt` | pub | unsafe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `OverlayFn` | pub | type alias |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `OverlayHook` | pub | borrowed view |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `OverlayHook::adopt` | pub | unsafe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `OverlayHook::submit` | pub | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `RawRegion` | pub | borrowed view |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `RawRegion::adopt` | pub | unsafe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `RawRegion::is_empty` | pub | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `RawRegion::len` | pub | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `RawRegion::load_u32` | pub | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `RawRegion::read_into` | pub | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `RawRegion::store` | pub | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `RawRegion::store_u32` | pub | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |
| `RawRegion::write_from` | pub | safe fn |  |  | LANE:v3-p1p2: 2026-10-04: unreviewed; this file is edited by that lane |

## Validation dependencies

Same-crate, non-perimeter files that define an item a perimeter file names (E11, derived by `perimeter.py exports`). VALIDATES: a perimeter memory-safety argument relies on it; the file is in `scripts/ci/perimeter/sizes.tsv` and gets perimeter review. USES: anything else.

| file | role | why |
|---|---|---|
| `crates/kf-linux-raw/src/bounds.rs` | VALIDATES | HostOffset, checked_span: the range checks perimeter accessors rely on |
| `crates/kf-linux-raw/src/cache.rs` | VALIDATES | CachePolicy, require_attainable: the caching a mapping may request |
| `crates/kf-linux-raw/src/census.rs` | USES | note: records an ioctl after it returned; no perimeter argument relies on it |
| `crates/kf-linux-raw/src/error.rs` | USES | RawError, last_syscall_error: error values |
| `crates/kf-linux-raw/src/geometry.rs` | VALIDATES | require_aligned: alignment checks before mmap/mremap |
| `crates/kf-linux-raw/src/ioctl.rs` | VALIDATES | MAX_IOCTL_SIZE, declared_size: the size bound CharDevice::ioctl checks against |
| `crates/kf-linux-raw/src/ioctltrace.rs` | USES | record: a diagnostic trace after the call |
| `crates/kf-linux-raw/src/page_size.rs` | VALIDATES | HostPageSize: the page size every placement is checked against |
| `crates/kf-linux-raw/src/view.rs` | VALIDATES | RegionView: the bounded view accessors return |
| `crates/kf-qemu/src/chan.rs` | USES | completion_probe_ms: a configuration knob |
| `crates/kf-qemu/src/device.rs` | USES | Config, Device: the safe device the FFI hands off to; the handle's validity is the C side's contract (S1-08) |
