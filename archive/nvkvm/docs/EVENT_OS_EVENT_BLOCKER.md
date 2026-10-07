# NV01_EVENT_OS_EVENT (hClass=0x79) blocker — 2026-05-26

cuCtxCreate fails with `CUDA_ERROR_INVALID_HANDLE` (400) because the
RM_ALLOC for `NV01_EVENT_OS_EVENT` returns `nvstatus=0x23`
(`NV_ERR_OBJECT_NOT_FOUND`).

## What we know

The driver source confirms there is exactly **one** place in the
NV01_EVENT_OS_EVENT alloc path that can return
`NV_ERR_OBJECT_NOT_FOUND`:

```c
// src/nvidia/arch/nvalloc/unix/src/os.c:1604
NV_STATUS osUserHandleToKernelPtr(NvHandle hClient, NvP64 hEvent, NvP64 *pEvent)
{
    nv_state_t *nv = nv_get_ctl_state();
    NvU32 fd = (NvU64)hEvent;
    ...
    nv_event_t *e = nv->event_list;
    while (e != NULL)
    {
        if (e->fd == fd && e->hParent == hClient)
            break;
        e = e->next;
    }
    if (e != NULL) result = NV_OK;
    else           result = NV_ERR_OBJECT_NOT_FOUND;  // <-- here
}
```

The event list is populated by `allocate_os_event()` (osapi.c:505),
which stores `e->fd = pApi->fd` and `e->hParent = pApi->hClient` from
the `NV_ESC_ALLOC_OS_EVENT` (NR=0xCE) ioctl.

The osUserHandleToKernelPtr call uses `pRsClient->hClient` as `hClient`
(eventConstruct_IMPL, event.c:179), and `pNv0050AllocParams->data` as
`hEvent`.

## What we see in our traces

Every observable field matches.

```
nvkvm_stub: ALLOC_OS_EVENT hClient=0xc1d00d4d hDevice=0x5c000003 handle=22 -> local_fd=22
nvkvm_stub: NV01_EVENT_OS_EVENT alloc hParentClient=0xc1d00d4d handle=22 -> local_fd=22
nvkvm: RM_ALLOC failed: hClient=0xc1d00d4d hParent=0x5c000003 hObjNew=0x5c00003a hClass=0x79 nvstatus=0x23
```

- ALLOC_OS_EVENT stored: `e->fd = 22, e->hParent = 0xc1d00d4d`
- RM_ALLOC lookup uses: `fd = 22, hClient = 0xc1d00d4d` (from pRsClient
  → outer h_root)

These should match. The lookup fails anyway.

## Suspects (in rough order of likelihood)

1. **pRsClient->hClient ≠ outer h_root**.  The RM resource server may
   create a different internal client handle if libcuda's
   `NV01_ROOT_CLIENT` alloc didn't register correctly with our object
   tracker.  Worth verifying by tracing the actual RsClient lookup
   path — needs driver-side instrumentation we don't have.
2. **A second `osUserHandleToKernelPtr` call** in a non-OS-EVENT path
   that triggers earlier and is the real culprit.  Searching the
   driver source found only the one call site in event.c, so this
   seems less likely.
3. **fd cast mismatch**: e->fd is u32, lookup does `(NvU64)hEvent →
   NvU32`.  Both should be 22 (low 32 bits of u64).  Hard to see
   how this corrupts.
4. **Spinlock / ordering**: ALLOC_OS_EVENT happens-before the lookup
   in the same thread on the same CPU — no race that produces a
   "not found".

## Next steps for tomorrow

The cleanest way forward is one of:

a. **kprobe the driver** to log the actual `e->fd, e->hParent`
   values being compared against `fd, hClient`.  Confirms whether
   the mismatch is real and which field.
b. **Read the full RM API session creation** to find out exactly how
   pRsClient is initialised and whether pRsClient->hClient differs
   from libcuda's h_root in our forwarded model.  This is in
   `src/nvidia/src/kernel/rmapi/client.c` or similar.
c. **Skip cuCtxCreate's event-using code path**.  If libcuda has a
   fallback when this alloc fails (e.g., it just disables CPU/GPU
   event sync), cuMemAlloc / cuMemcpy might still work without
   addressing this blocker.

Option (a) is most direct but requires getting kprobe access on the
remote host's running kernel.  Option (c) is the lowest-effort sanity
check — try cuMemAlloc directly after a forced-success of the event
alloc and see if the rest of cuCtxCreate proceeds.

## Today's commits in this area

- `f635bd3` — eventfd-on-isolate handle (wrong approach; gVisor
  confirmed Data must be a frontendFD, not an eventfd).
- `aaa8245` — Switched to handle_id translation for both
  NV_ESC_ALLOC_OS_EVENT and NV01_EVENT_OS_EVENT.Data, so the
  values that reach the driver match each other.  This is the
  correct architectural pattern per [[user advice 2026-05-26]] —
  it just doesn't fix the blocker because the mismatch (if any)
  is elsewhere.
