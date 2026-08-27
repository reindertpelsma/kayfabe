# Mode-1 os-event poll relay (blocking-sync delivery) — implementation plan

**Branch:** mode-1-blocking-sync (forked from master). Merge to master once
validated with real apps + a forced blocking-poll test, then merge master ->
mode-2 (shares the infra for Mode-2 interrupt delivery).

## The gap (confirmed 2026-06-04)
CUDA's blocking-sync path (CU_CTX_SCHED_BLOCKING_SYNC — inference servers set it
to avoid 100% CPU spin; blocking cuEventSynchronize/cuStreamSynchronize; NCCL)
waits via poll() on an NV01_EVENT_OS_EVENT fd. In Mode-1 that wakeup never
arrives promptly: the guest blocks on ctx->poll_wq and the producer is missing,
so libnvidia falls back to a ~18 ms poll-timeout-then-recheck per completion
(severe latency; potential hang if it polls without timeout). The 20 passing
apps use the spin/yield mapped-semaphore fast path and never hit this.

## What already exists (do NOT rebuild)
- Guest (src/guest/nvkvm_virtio.c, nvkvm_main.c): vq_evt with 16 pre-posted
  `struct nvkvm_evt_poll {isolate_id, handle_id, events, reserved}` buffers;
  nvkvm_evt_callback -> nvkvm_evt_deliver(isolate_id, handle_id, events) ->
  atomic_or(poll_events) + wake_up_interruptible(poll_wq). FULLY WIRED.
- Protocol: nvkvm_req_poll_on_isolate {isolate_id, handle_id, events} (guest->QEMU);
  isolate_cmd_poll {handle_id, events} / isolate_cmd_unpoll {handle_id}
  (QEMU->stub); isolate_resp_poll_event {handle_id, revents} (stub->QEMU async).
- QEMU forwards POLL_ON_ISOLATE/UNPOLL_ON_ISOLATE to the stub (virtio_nvgpu.c:825).

## The two TODOs to implement
### 1. Stub (src/stub/nvkvm_stub.c) — background poll, freestanding
- Maintain a small table {handle_id -> host fd, events} (fd from the handle_id
  registry). ISOLATE_CMD_POLL adds; ISOLATE_CMD_UNPOLL removes. (handler ~2386,
  currently `send_ok(); /* TODO */`.)
- Integrate the fds into the worker loop's idle socket-service point
  (ring_loop_poll_socket / the main reader ppoll). The stub already loops on the
  ring + services the control socket when idle; extend that ppoll set to include
  the registered os-event fds. On an fd readable -> send isolate_resp_poll_event
  {handle_id, POLLIN}. (No new thread — the stub is nostdlib; reuse the existing
  loop + __NR_ppoll, already in the seccomp allowlist.)
- Edge: level-triggered fd stays ready until the guest drains the event via the
  RM ioctl; send one resp_poll_event per readable edge and let UNPOLL or the next
  ALLOC re-arm (matches the guest recycling its evt buffer). Avoid a busy spin:
  after sending, suppress re-notify for that handle until the guest re-polls
  (UNPOLL then POLL), OR drain the eventfd if it is an eventfd-typed handle.

### 2. QEMU (src/qemu/nvkvm_isolate.c:553 + virtio_nvgpu.c) — push vq_evt
- nvkvm_isolate.c ISOLATE_RESP_POLL_EVENT (currently "TODO: forward to virtio EVT
  queue"): call a new virtio_nvgpu helper nvkvm_virtio_push_evt(iso->id /*isolate
  _id*/, poll_event.handle_id, poll_event.revents).
- virtio_nvgpu.c: nvkvm_virtio_push_evt() pops a pre-posted elem off nv->vq_evt,
  fills struct nvkvm_evt_poll {isolate_id, handle_id, events, 0}, virtqueue_push
  + virtio_notify. Guard with the bh/iothread lock as other vq pushes do; if no
  evt buffer is available, drop (guest re-arms) and log.

## Test (acceptance — "Mode-1 really done")
- Mode-1 VM (ssh port 2222 on vh; deploy /workspace/nvkvm).
- (a) A real app from the matrix still passes (no regression).
- (b) FORCE the blocking path: cuCtxCreate(CU_CTX_SCHED_BLOCKING_SYNC=0x4) +
  a kernel that runs long enough (>~1-2 ms, past CUDA's spin threshold) so
  libcuda sleeps the thread on the os-event fd; cuStreamSynchronize must return
  promptly (sub-ms wake, not the ~18 ms fallback) and correctly. Measure wakeup
  latency before/after to prove the relay fires.
