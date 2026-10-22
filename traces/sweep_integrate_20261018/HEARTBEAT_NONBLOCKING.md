# Heartbeat path and blocking audit

STATUS: LIVE, 2026-10-19. CODE READ; hardware UNRUN.

Approved scope: only the GSP mailbox heartbeat clock operation in the drainer,
not the unrelated existing drainer parking, register service or diagnostics.
The owner requires that this new operation never wait, including on a lock a
vCPU takes. The old submitted proposal did not satisfy that requirement.

Call path: Device::drainer_loop (existing500ms eligibility) ->
heartbeat::try_sample -> gsp.try_lock -> current-life fn72 flag + exact policy ABI
-> SystemInfoCell::try_with_latest -> cell.try_lock -> offset_ns generated ABI
lookup + bounded8-byte load -> sample -> TimerWindow::read_ns ->
kf_abi::submit::ptimer_sample (maximum3 hi/lo/hi rounds,9 mapped reads).
The existing bounded shadow mailbox stores publish the two u32 values.

No RM ioctl, blocking mutex acquisition, condition variable, sleep, retry-until-
success loop, copied Vec or allocation enters this clock operation. A contended
or poisoned lock returns None. Missing/invalid fn72 or timer returns None. The
ordinary console latest() clone is unchanged and is not called by heartbeat.
The state guard can coexist with the fn72 guard only after both try-locks succeed;
neither acquisition waits. Mailbox shadow_store/piece_for uses fixed mappings
and atomics; it waits on no mutex or vCPU lock. No guest work completion is forged.

An existing FSM constructor reset clears the new readiness bit. The existing
fresh-vs-same queue-identity decision also clears only this bit on a fresh queue.
No console cell is cleared, no reset decision is changed, no scrub is changed.
GOP off retains fn72 clock metadata but still creates no console seat. A guest-
chosen offset changes only bounded u32 clock arithmetic, never a host RM action.

MEASURED initial23 tests green, then two fresh-queue tests independently failed
and were fixed. Contention tests hold each mutex on one thread and require the
reader's result while that lock remains held; watchdog expiry fails the test,
then releases and joins the worker. Two distinct tests target each blocking
mutation. The actual Device call-site guard also pins read_ns and500ms and rejects
lock(), latest() and gpu_time_ns(). Mutation receipts and full counts are separate.
