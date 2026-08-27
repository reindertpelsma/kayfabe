# nvkvm Security Audit — 2026-05-29 (branch security-fixes @ 290d7c0)

Automated audit (read-only). Per-VM tables (one QEMU per VM); "cross-session" = cross-process within one VM. nvkvm_frontend.c/nvkvm_dispatch.c are DEAD code (legacy).

## CRITICAL
- **C-1 guest→VMM OOB R/W**: live IOCTL_ON_ISOLATE / WRITE+READ_MEMORY_HANDLE / READ_HOST_FILE / REALIZE handlers validate slot_valid() but NOT size<=slot_size. param_size/aux_size/req->size up to MAX_PARAM_SIZE(256K) vs slot_size(64K) → sock_send_full reads past slot (disclosure/DoS); recv writes response past slot (OOB write); READ_MEMORY_HANDLE pread writes req->size into slot. Legacy handle_ioctl had the check (virtio_nvgpu.c:326); not carried to the live thread-pool path. FIX: reject size>slot_size && slot*slot_size+size>shm_size in every live handler.
- **C-2 handle-table TOCTOU**: nvkvm_handle_get returns bare ptr after dropping lock; IOCTL worker (thread pool) derefs h->fd while CLOSE_HANDLE on another thread close()s+reuses fd → ioctl on wrong/closed fd. FIX: copy fd/dev_id under lock + in-flight refcount.

## HIGH
- **H-1 no session ownership check**: handle_id/isolate_id taken from guest, looked up in global per-VM tables, never checked against requesting session_id (guest-set, untrusted). Intra-VM: process A drives B's isolate/handles. FIX: record owning session_id, reject foreign.
- **H-2 Path-α TYPE_ALL host-wide DUP grant** (known, Phase 4): every RM_ALLOC shared host-wide → other tenants/host procs can DUP guessed handles. FIX: TYPE_CLIENT to UVM kernel client.
- **H-3 DUP gate is only cross-object check**: RM_CONTROL/MAP/FREE foreign hClient/hObject fields ungated. FIX: vet hClient on every forwarded 'F' ioctl against client_allow[].
- **H-4 no teardown on guest death/SIGKILL** → resource exhaustion DoS. nvkvm_handle_close_session / nvkvm_isolate_kill_session exist but are NEVER called. Leaks isolates/fds/KVM slots/GPA window. (Matches observed SIGKILL fd creep.) FIX: wire guest mm-release → teardown request; add GPA free-list.

## MEDIUM
- M-1 MMAP_ON_ISOLATE: guest offset/length/prot unbounded; prot not masked (REALIZE masks). FIX: prot&=R|W, bound len.
- M-2 stub overwrites param_buf+16 for ANY ioctl with aux_size>0&&param_size>=24. FIX: gate on specific NRs.
- M-3 stub seccomp: no W^X (mprotect PROT_EXEC allowed), unrestricted ioctl/openat. FIX: arg filters.
- M-4 iov_to_buf return unchecked for request bodies (zero-init mitigates).
- M-5 iso_mmap token reuse (no generation counter) + owner check.

## Prior audit status
- C1 TOCTOU: NOT fixed (C-2). C3/C4 cross-session: NOT fixed (H-1). C6 seccomp: partially (allowlist yes, arg filters no, M-3). M6 stub KVM fd: FIXED (closefrom+empty env+fexecve+ns/cap lockdown).

## Fix order: C-1, C-2, H-1/H-3, H-2, H-4, then M-*.
