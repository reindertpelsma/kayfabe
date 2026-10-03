# libcuda's channels are born USER while QEMU runs as root (2026-10-03)

**STATUS: LIVE, 2026-10-03.** Branch `v3-sec-nonpriv`. Security-policy change: the owner reviews it
before merge.

## The property and what was missing

Host RM stamps a channel's privilege at the channel-alloc ioctl from the calling thread's
`capable(CAP_SYS_ADMIN)` (`ogkm-580: kernel_channel.c:277-291`, `escape.c:304`), and writes the
verdict into the reply: `NVOS04_FLAGS_PRIVILEGED_CHANNEL` (bit 5 of `NV_CHANNEL_ALLOC_PARAMS.flags`)
is set for an `ADMIN` or `KERNEL` channel and left as requested for a `USER` one. kf-host checks the
reply of every channel it births (`../nonpriv_20261003/`). libcuda allocates the channels of kf3's two
CUDA contexts (the walker, C2, and the display, C3) inside libcuda, where kf3 never sees the reply,
and kf3 made those calls on QEMU's own thread, which holds `CAP_SYS_ADMIN` when QEMU runs as root.

The change (`kf_cuda::posture`, `kf_qemu::device::on_cuda_thread`): before a thread's first CUDA call
(library load, `cuInit`, `cuCtxCreate`, `cuCtxSetCurrent`), kf-cuda clears `CAP_SYS_ADMIN` from that
thread's effective set for the rest of its life, and kf3 makes realize's CUDA calls on threads of their
own (`kf3-cuda-walk`, `kf3-cuda-store`, `kf3-cuda-disp`) so QEMU's thread keeps its sets. The
VA-manager and display threads clear it at their first call.

## How it was read

`scripts/bench/sec/chan_alloc_observer.c` is an `LD_PRELOAD` observer loaded into QEMU only (through
a one-line wrapper set as `QEMU_BIN`). It lets every `ioctl` through unchanged and, for each
`NV_ESC_RM_ALLOC` of a GPFIFO channel class on a `/dev/nvidia*` fd, logs the request's and the reply's
`flags`, the calling thread's name and its `CapEff` bit 21. The verdict is reported only when the
request did not ask for bit 5; otherwise it reads `?`. kf-host's handles start at `0xcafe0001`, so the
summary splits kf-host's channels from libcuda's. `scripts/bench/sec/libcuda_channel_census.sh` runs
one guest arm (`--timer`, with the harness's `--probe-launch-dma`) per configuration.

Box 54049598 (`vmb`), RTX 3060, host driver 580.159.04 (open kernel module), QEMU 10.2.4 + kf3 run
as root by `run_fast_guest.sh`, as the bench does.

## Results

| file | kf3 revision | configuration | libcuda channels | kf-host channels |
|---|---|---|---|---|
| `secnp_before3_chanobs.log` | `4b864b5f` (before the change) | walker | 16, **all `PRIVILEGED_CHANNEL=1`** (reply `0x20`/`0x30`), on thread `qemu-system-x86` with `capeff_sys_admin=1` | 2, USER |
| same | `4b864b5f` | `display=on` | 32 (16 walker + 16 display), **all `PRIVILEGED_CHANNEL=1`** | 2, USER |
| `secnp_after_1d71f3db_chanobs.log` | `1d71f3db` | walker | 16, all `PRIVILEGED_CHANNEL=0`, on `kf3-cuda-walk` with `capeff_sys_admin=0` | 2, USER |
| same | `1d71f3db` | `display=on` | 32, all `PRIVILEGED_CHANNEL=0` (`kf3-cuda-walk`, `kf3-cuda-disp`) | 2, USER |
| same | `1d71f3db` | negative control `KF3_NEGCTL_SKIP_CAP_BRACKET=1` | 16, all USER | 1, **`PRIVILEGED_CHANNEL=1`** (reply `0xa0`), refused by kf-host and freed; the arm then fails, as expected |

- Every guest arm in the BEFORE and AFTER walker and display runs passed (`FAST_VERDICT=PASS`).
- Each AFTER run logs one `kf-cuda: cuda thread posture … cap_sys_admin=cleared-for-thread-life` line
  per CUDA thread: `kf3-cuda-walk`, `kf3-cuda-store` and `kf3-vamgr`, plus `kf3-cuda-disp` and
  `kf3-display` with `display=on`.
- All 64 libcuda channel allocs in the AFTER runs happened during realize, inside `cuCtxCreate`, on
  the two bring-up threads. None was allocated later on the VA-manager or display threads in these
  arms; those threads are cleared anyway, so a lazily created channel would be USER too.
- The observer's known-positives: the BEFORE runs (libcuda's channels read `1`) and the negative
  control (a kf-host channel reads `1` on a live reply, which kf-host also refuses by name).

## Instrument notes

- `secnp_before_chanobs.log` is the observer's first version on the same BEFORE binary. It read
  every libcuda channel as `PRIVILEGED_CHANNEL=?`: libcuda passes `paramsSize = 0`, and that version
  required a size. RM ignores the caller's size and copies the class's own parameter struct in and out
  (`ogkm-580: rmapi/alloc_free.c:134-143`), so the reply is there. It reported "unmeasured", not 0;
  the fix is commit `1d71f3db`.
- `secnp_after_job_staletarget.log`: the first AFTER build failed to compile kf-cuda against a
  `kf-linux-raw` without the new API. The worktree's files were created before a build of another
  checkout of the same branch into the same `CARGO_TARGET_DIR`, so cargo took that checkout's
  artifacts as fresh. The rebuild used a target directory of its own (`secnp_after_job.log`).
  `merge_check.sh` creates its worktree immediately before it builds, so its sources are newer than
  any artifact in the shared directory.
