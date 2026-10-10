# Windows on the production profile, 2026-10-10 (host RTX 4070, driver 595.91.07)

STATUS: RESEARCH — evidence from three runs of the same scripted sign-in + Edge + Shorts harness
(host-only tooling), the integration branch, production behaviour flags only (20 class-B flags of
`V3_FLAG_INVENTORY.md`, zero measurement flags, batching on).

| run | build | extra | outcome (measured from kayfabe's `qemu.log`, host dmesg, guest event log for 260) |
|---|---|---|---|
| 260 | `a6587d6a` | none | bugcheck 0x116 (VIDEO_TDR_FAILURE, param 3 = 0xC000009A) at the first TDR reset; the post-reset restart pair `0xc1d0005c`/`0xc1d0005e` shared one VA space, the second was refused `KernelInUserSpace(1)` (RmAlloc 0x40); after the auto-reboot the adapter was in Code 43 |
| 262 | `a6587d6a` | `KF3_WIN_KERNEL_PID4=1` | READY; 2 TDR resets in ~4 min, no `KernelInUserSpace`, no refused birth, no host Xid, no `gpu_vaspace` assert |
| 263 | `459da55d` | none (PID4 hardwired) | READY; 7 TDR resets in ~8 min (rate not stable), no refused birth/DEAD/POISONED, `unreconciled=0`, no host Xid, no `gpu_vaspace` assert (`run263_summary.txt`) |

Not established: why the first TDR occurs and why it recurs about once a minute; whether Edge/Shorts
playback is usable between resets (the owner was asleep; the harness only checks that the session
is alive). `KF3_WIN_KERNEL_PID4` is hardwired since `459da55d` (`V3_RECOVERY_WALL.md`, dated update).
