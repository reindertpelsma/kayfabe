# C2 exact sparse residency and the global-priority fix, on hardware

**STATUS: LIVE, 2026-11-18.** Text evidence for `docs/design/V3_SPARSE_RESIDENCY_RCA.md` §8.10 (the C2
recipe) and for PR #508 (Vulkan global queue priority). Scrubbed text only: GPU UUIDs, IPs and MACs replaced,
large logs cut in the middle (marked), no keys, URLs or images.

## Run

- **Box:** `kf-openguest-180716`, a Vast KVM instance with an RTX 4090 (Ada), host and guest driver
  580.159.04. Rented 07:17Z, destroyed by its own id at about 08:49Z, absence verified. At the start, 0
  `GPPut < WATCHDOG_GPFIFO_ENTRIES` asserts and 0 Xid, both at boot and after the driver loaded.
- **Trees** (branch `claude/hw-confirm-tree-20261118`; no PR):
  - `f5dbe55e`: the base `claude/accept-final-tree-20261118` @ `8c04fbd34`, which did NOT contain #480, so
    #480 was merged explicitly. Plus the C2 stack (#510 → #511 → #512) and #508 @ `c890818f`.
  - `bb5b2c41`: `f5dbe55e` plus #508's tip `0075b1bc` (MAKE_REALTIME `0xa06c0110` and RESTART_RUNLIST
    `0xa06f0111` served as per-VM virtual state), plus the C2 production-forwarding fix `7fb3c05e8`
    (`GpuMirror` and `Target` forward `set_guest_bases`).
- **Order:** one serial chain (`workspace/chain_c2.log`):
  1. Host oracle.
  2. SOFF2 (`f5dbe55e`, properties at their defaults).
  3. SC2B (`f5dbe55e`, `exact-sparse=on`).
  4. `v3_gates`.
  5. Rebuild at `bb5b2c41`.
  6. PRIO.
  7. SC2C (`exact-sparse=on`).
  8. UMON2 (`exact-sparse=on`).
  9. The fast suite (`exact-sparse=on`).
  10. `v3_gates`.

  `unmapped-reads-zero` was never set, so it stayed at `auto`, which is FAULT for a Linux guest.

## Results

**Sparse residency (12 Vulkan CTS cases, each in its own `deqp-vk` process; host oracle: 9 Pass, 3
NotSupported).**

| case | host | SOFF2 (off) | SC2B (on, `f5dbe55e`) | SC2C (on, `bb5b2c41`) |
|---|---|---|---|---|
| `multisampled_image_sparse_residency.rgba16ui.samples_4` | Pass | **Timeout 300 s, 1 Xid 109** | **Timeout 300 s, 1 Xid 109** | **Pass (4 s), 0 Xid** |
| `texel_buffers..._size_2_24_r32ui` | Pass | **Timeout 300 s, 1 Xid 109** | **Timeout 300 s, 1 Xid 109** | **Pass (4 s), 0 Xid** |
| msbind, t16, mip2d, mipcube, imgmut, bindinfo, alias | Pass | Pass | Pass | Pass |
| dgimg, intr, rebind | NotSupported | NotSupported | NotSupported | NotSupported |

Every Xid 109 is `CTX SWITCH TIMEOUT` on channel 0x31 with `Info 0x6c022`. SC2C has 0 host Xid over all 12
cases.

**The S2 control: managed memory with `exact-sparse=on`** (UMON2, `bb5b2c41`). The host bundle's binaries
were copied into the guest: the guest bundle carries no UVM samples. The host passes all three rows.

| row | host | guest, C2 ON |
|---|---|---|
| UnifiedMemoryPerf | PASS | **fails loudly**: host Xid 31 `FAULT_PDE`, CUDA 719 (`cudaErrorLaunchFailure`) |
| UnifiedMemoryStreams | PASS | **fails loudly**: host Xid 31 `FAULT_PTE`, CUBLAS 13, core dump |
| conjugateGradientUM | PASS | **fails loudly**: host Xid 31 `FAULT_PDE`, `FAILED` |

**Global priority (#508).**

| | host | SOFF2 (`f5dbe55e`, #508 @ c890818f) | PRIO (`bb5b2c41`, #508 tip) |
|---|---|---|---|
| `create_device_global_priority_khr.basic` | Pass | **Fail** (`VK_ERROR_INITIALIZATION_FAILED`) | **Pass** |
| `create_device_global_priority_query_khr.basic` | Pass | **Fail** | **Pass** |
| `global_priority_transition.high.no_sync.sparse.from_graphics_to_compute` | Pass | Pass | Pass |
| `global_priority_transition.realtime.semaphore.no_modifiers.from_compute_to_transfer` | Pass | NotSupported | **Pass** |
| GSP refusals of `0xa06c0110` and `0xa06f0111` | – | both refused (seq 2009, 1942) | **none** (the other 35 refused controls are identical in both boots) |
| `gap_vkprio` as root | families 0, 2, 5: LOW..REALTIME; 1, 3, 4: LOW..HIGH (REALTIME `-1000174001`) | (not run as root, see below) | **identical to the host** |
| `gap_vkprio` as a plain user | LOW and MEDIUM only (12 accepted) | MEDIUM reported; HIGH and REALTIME `-1000174001` | the same as SOFF2 (a capability effect, not a gap) |

**Regression.**
- `v3_gates`: 9/9 plus gate10 on `f5dbe55e`, and 9/9 plus gate10 on `bb5b2c41`.
- Fast suite with `exact-sparse=on` on `bb5b2c41`: **30/30**. All 30 QEMU command lines carry
  `exact-sparse=on`, and the suite names `rev=bb5b2c41`.

## Verdicts (predictions and falsifiers stated before the run, RCA §8.10)

- **OFF: msresid and t24 time out with Xid 109.** HELD (SOFF2; SC2B too, for the reason below).
- **C2 ON: the 12 cases match the host, and 0 Xid 109.** HELD on `bb5b2c41` (SC2C). The falsifier ("Xid 109
  or a timeout with C2 ON") was observed on `f5dbe55e` (SC2B). The cause was a C2 wiring gap: the production
  `MapTarget` chain (`GpuMirror`, `Target`) did not forward `set_guest_bases`, so the trait default refused
  every SPARSE run. The GPU-free model forwarded it. Fixed in `7fb3c05e8`, with a source test that fails
  without the fix.
  - ⚠ The SC2B diagnosis was first quoted from the kf3 log line "SPARSE run refused: this target has no
    exact tiling". The same 16 lines (one VasKey, about 25 s into the boot, rate-limited) appear
    identically in SC2C. So that quote does not prove the gap. The gap is real in the code (`GpuMirror` and
    `Target` lacked the forward), but that it caused the SC2B timeouts is INFERRED: SC2B and SC2C differ by
    that fix and by #508's priority change, and neither was tested alone.
- **S2: managed memory still fails LOUDLY with C2 ON.** HELD: all three rows show host Xid 31 and a guest
  CUDA, CUBLAS or result failure. The falsifier (a pass, or a mismatch without an Xid) was not observed.
- **#508: all four priority cases pass, and the root probe accepts every priority the host offers.** The
  falsifier was HIT on `c890818f` (pcreate -3, refused `0xa06c0110` and `0xa06f0111`). Both predictions HOLD
  on the tip `0075b1bc`.

## Harness artifacts, named

- **The first SOFF and SC2 boots ran no case.** The hook `ogr_hook2.sh` lost its execute bit in transit
  (`HOOK_RC=126` in `workspace/apps/results/SOFF/boot_SOFF.probe.log`). They were re-run as SOFF2 and SC2B.
- **The SOFF2 `gap_vkprio` ran WITHOUT sudo**, while the CTS cases and the host reference ran as root. HIGH
  and REALTIME are gated by CAP_SYS_NICE in the guest's own kernel, so its "only MEDIUM" reading is a
  capability effect. The hook now runs the probe as root and keeps the plain-user run as the control (PRIO).
- **The app-matrix managed-memory runs (`c2um_off`, `c2um_on`) are VOID.** The guest bundle has no UVM
  samples (the apps stage marks them "unsupported (UVM)"), so the result was "No such file". UMON2 replaced
  them with the host's binaries.
- **NOT_RUN, stopped at the coordinator's wrap-up:**
  - the property-off fast suite on `f5dbe55e`, which was interrupted mid-run;
  - the property-on fast suite on `f5dbe55e`;
  - UMOFF and UMON on `f5dbe55e`.

  There is no managed-memory guest run with C2 OFF in this trace. The OFF behaviour is the one
  `V3_BUILD.md` C documents.

## Layout

- `workspace/chain_c2.log`: the chain, one line per step.
- `workspace/steps/`: the step logs.
- `workspace/apps/results/<boot>/`: per case, the guest CTS log, the host and guest dmesg lines it added, and
  for the key cases the kf3 log window.
- `workspace/bench/run_<boot>_probe.log`: the boot probes, with the `OGR_*` and `UM_*` lines.
- `workspace/ogr_out/`: the host oracle and the host `gap_vkprio` runs (root and non-root).
- `extracts/`: the kf3 lines with `exact-sparse`, `SPARSE run refused`, `GSP REFUSED`, Xid and tiling per
  boot, and every host Xid line of the box.
- `root/prov/`: provisioning.
