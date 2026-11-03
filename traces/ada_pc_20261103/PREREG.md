# Ada PC regression of `next` 70e3fabd8 — preregistration (written 2026-11-03T11:32Z, before any run)

Host: the owner's trusted PC (RTX 4070, AD104, `10de:2786`), host driver **595.91.07** (not 580.159.04; read
with `nvidia-smi` at 12:21 CET), kernel 7.0.0-34, IOMMU group 11 `DMA-FQ`, no QEMU running, 27 GiB free RAM.
Revision under test: `next` head `70e3fabd8` (merge of #239, the Blackwell merge).

## H1 — GPU-free
`cargo test --no-fail-fast -p kf-*` (every kf-* crate) on the host. **Falsifier:** any failed test.

## H2 — Linux guest on Ada (the host's recorded validate.sh recipe: unit, `build_kf3.sh`, `v3_gates.sh`,
raw client, host-mode fast guest from `/workspace/bench/guest.qcow2`, `fast_suite.sh` 30 arms, budget 180 s)
**Falsifier:** gates other than 9/9 (`V3_GATES_SUMMARY`), any fast-suite arm not PASS (30/30), or a new host Xid.

## H3 — Windows guest on Ada
Image: a fresh CoW overlay on the read-only `/data/hp1-20261025/baseline3/windows.qcow2` (owner's Windows 11 Pro
26200 apps chain, NVIDIA 580.88, GSP keys set; last used 2026-10-25 by the hc/hp1 campaigns), the owner's
production argv+flags of run 4501 (19 class flags + 2 inert lifecycle counters), only the binary, paths and
broker socket changed; Wayland broker window (display on); loopback PowerShell agent. Probes cross-built from
`70e3fabd8` source with mingw-w64 (cup8, kf_dxprobe, kf_glgears).
Pass = desktop signed in (dwm+explorer), NVIDIA adapter Code 0, `nvidia-smi -L/-q`, `cup8` PASS bad=0 and
`kf_dxprobe d3d11` RESULT OK before AND after a 600 s idle, a windowed GL app presenting through DWM for 90 s
before and after the idle at a sustained rate, and reset counters since boot all zero (Display 4101,
nvlddmkm 153, Kernel-Power 41, 6008, BugCheck 1001, LiveKernelEvent other than 193; 193 reported apart).
**Falsifiers:** Code != 0; any probe failure/timeout; any reset counter > 0; a new host Xid; **flip stall:** the
host `kf3: display fps[...]` lines show the kms (flip) rate dropping to 0 while gears runs, or gears' own
frame count below 10 fps over 90 s, or a `channel ... STOPPED` display line. At most 3 negative boots.

## H4 — walk completion change (host callback -> event query polled at 1 ms while a walk is in flight)
INFERRED expectation: the per-walk latency `vat[walk_avg_us]` rises from the pre-change ~145-150 us (Windows,
same image and host, rev 714ec29d, `/data/hp1-20261025/baseline3/qemu.log`) towards 0.5-1.1 ms, with
`gpu_avg_us` unchanged (~100 us); `kf3-vamgr` CPU stays small (< 5 % of one core idle).
**Falsifier for "the latency rises":** `walk_avg_us` stays <= 200 us on a comparable window. Measured, not inferred.
