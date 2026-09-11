# ⊘⊘⊘ SUPERSEDED BY `RESUME_HERE_w417.md` (2026-09-11) — read that first

Its central claim, *"there is NO general operand path"*, is **half right, and the other half
was the bug**: the path exists (`measure_guest_ram_pin_rate`) and was switched OFF by the
publication worker forcing `VasPublishArm::Publish`, then rate-limited to a 256-row sample.
Both fixed (w415 `05df1d6b`, w416 `cfd6db36`), with the fault changing kind each time.

# RESUME — w414, overnight 2026-09-11

**STATUS: LIVE.** Written for a compacted session. Read this first, then
`gpga_is_one_reserved_object.md`.

## Where things stand

★ **The raw client PASSES** at `threads:8 rounds:8`, twice (`w407`/`w407b`), and again on HEAD
after the capability change (`w422`). One host Xid, the long-known `CE0 @ 0xa0_00000000`.

⊘ **The LLM does NOT pass.** It hangs in CUDA setup with **ZERO doorbells** — the guest never
submits. Every passing LLM run in history had `VAS-PUBLISH arm=drain`; my run had `arm=off`,
and that is the only difference across three runs. **The comparison is unclean, not the answer.**

## The three commands

```
# client, the gate that means something
PREFIX=<tag> bash scripts/bench/w401_trigger_boot.sh
#   grade: W392D_MEAN_CONFIG=threads:8 rounds:8 must print, then P1/P2/P3 VERIFIED over 8,
#   THREADS 8 of 8, MEAN_FALSIFIER=PASS. Host Xids must be 1 (the CE0 bystander only).
# LLM
PREFIX=<tag> bash scripts/bench/w409_llm_boot.sh
#   grade: the GPU text must MATCH the same-boot CPU oracle. A token count is NOT a pass.
# host-side, no VM, seconds
./target/release/kayfabe-rm-ladder --gpu 0 --gpga-reserve-probe
```

⚠ Build the device with
`KAYFABE_SHIM_FEATURES=host-isolates bash scripts/build_qom_shim.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build`
— it needs **both** arguments and exits 1 without them **while the boot proceeds on the OLD
binary**. Verify by CONTENT (`strings | grep`), never by a revision stamp.

## ⊘⊘ THE TRAP THAT COST SIX BOOTS TODAY

The guest's kernel was upgraded by `unattended-upgrades` to `6.8.0-139` while its NVIDIA module
was built for `6.8.0-138`. Every boot then wedged **identically on every commit**, including
ones that had passed, because `modprobe` failed and no device appeared — so the dmesg capture
was empty and the harness refused the tag. It looked exactly like a deterministic code
regression.

★ Fixed: guest pinned to `6.8.0-138` via `GRUB_DEFAULT`, `unattended-upgrades` disabled, kernel
packages held. **If boots start wedging again with an empty dmesg, read
`run_<tag>_probe.log` FIRST** — it prints `nvidia-smi` output and `MODPROBE_RC`, and it said so
the whole time while I read the QEMU log, the serial log, the host GPU and the disk image.

## What was built today, and what is wired

| built | wired? |
|---|---|
| `OffVcpu` capability — publishing/refreshing is a **compile error** off the worker | ✅ yes |
| `dropped::DroppedSignals` — a full queue arms a full rescan / full emulated sweep | ✅ yes |
| `dbtable::DoorbellTable` — one atomic word per token, 77 lines, no lock | ❌ **no** |
| `promotion::Promotions` — the promote/demote ledger | ❌ **no** |
| `refresh.rs` — the VA-refresh fixpoint with provenance | ❌ no (older) |
| `gpgaview::GpgaViews` — the CPU/GPU view registry | ❌ no (older) |
| `reserve_gpga` + `--gpga-reserve-probe` | ✅ probe only |

## The measurements that decide the design

| | |
|---|---|
| largest single vidmem reservation | **6144 MiB** — we advertise **12288**, so it must be DERIVED |
| CPU read of vidmem | **~48 MiB/s, FLAT** 64 B → 16 MiB |
| same loop, ordinary RAM | **3674 MiB/s** (244x) |
| BAR1 (CPU window into vidmem) | **256 MiB total**, shared with the host driver |
| GPU VA mappings | **not** aperture-bound — a scratchpad may map all of GPGA |
| page-table footprint | **1 part in 500** of what it maps; 1872 pages ≈ 7.3 MiB observed |

⇒ Promotion into host RAM is **the difference between booting and not**, not a percentage gain.

## The next step, and its gate

1. Re-run the LLM with `KAYFABE_VAS_PUBLISH=drain` to make the comparison CLEAN. If it passes,
   the drain arm is load-bearing for CUDA and that is the thing to explain. ⚠ It is a deleted
   path's arm; a pass there is a diagnosis, not a fix.
2. Wire `DoorbellTable` — it deletes the 481-line `ring_inline`, and the doorbell then **cannot**
   publish or execute because it does not hold the device.
3. Then the reservation, keeping the invented framebuffer as the read path.

⚠ **The client is the only gate that means anything and it has a measured 1-in-5 false-negative
rate on a single boot.** n=1 is not a grade.

## ⚠ The rented box and its deadman switch

Instance **50376491**, label `kayfabe-w393`, RTX 3060, `ssh w393`.

A deadman switch runs locally at `.deadman/watch.sh`: it destroys the instance if
`.deadman/beat` goes **3 hours** stale, and logs to `.deadman/beat.log`. ⊘ Touch the beat file
while working — a short window would kill the box mid-measurement (a boot is 5 minutes, an LLM
run 20+), which is worse than an hour of billing.

```
touch /workspace/kf-master/.deadman/beat        # postpone
vastai destroy instance 50376491 -y             # ★ -y skips the confirmation prompt
```

## ★★★★★ THE DEBUG TOOLING ALREADY EXISTS — use it before guessing at CUDA

Owner, 2026-09-11: *"do not forget how valuable your raw client is to get a pass and debug with
before looking at cuda itself, if you can reproduce raw"*, and that both older trees carry a
long history of libcuda debugging.

### `/workspace/nvidia-gpu-passthrough/tests/mode2/nvdiff/` — the live differential oracle

| file | what it is |
|---|---|
| `nvdiff_shim.c` | **LD_PRELOAD ioctl/mmap recorder**, JSONL. Per call: ordering + thread, the decoded request word, the raw header **BEFORE and AFTER**, and for `RM_CONTROL`/`RM_ALLOC` the out-of-line parameter **pointer, declared size, and bytes on both sides**. ⇒ the nested-pointer case, solved. |
| `uvm_sizes.h` (+ `gen_uvm_sizes.sh`) | ⚠ **THE SIZE BUG.** UVM ioctl numbers are RAW INTEGERS, not size-encoded: `_IOC_SIZE(0x30000001)` = **12288** against a **16-byte** struct. A first cut recorded ~500 bytes of unrelated stack as "the parameter" and produced **2672 phantom divergences**. Sizes come from the driver headers, never from the request word. |
| `nvdiff.py` | align + classify — MISSING / EXTRA / SIZE / STATUS / VALUE |
| `nvd_selftest.sh` | ⚠ **RUN IT BEFORE TRUSTING ANY DIFF.** Checks the differ *detects*, offline, no GPU: it must find exactly **479** divergences (`dev` vs `ctx`) and **5** (`ctx` vs `alloc`). |
| `nvd_capture.sh`, `nvd_apis_capture.sh`, `nvd_fault_run.sh` | drive a capture |
| `nvd_phases.py`, `nvd_progress.py`, `nvd_census.py`, `nvd_dma_census.py` | phase / progress / census decoders |

★ Noise floor measured **ZERO** over 12 pairings and 6 stages. ⚠ **Rank divergences by KIND,
never by index** — the first by index was environmental (the reference host is a five-GPU rig on
the closed driver).

### `/workspace/nvkvm-pv/tools/` — the Mode-1 sibling's set

`nv_ioctl_trace.c`, `uvm_ioctl_shim.c`, `uvm_backing_probe.cu`, `uvm_sysmem_probe.c`,
`uvm_va_probe.c`, `ogkm_ctrl_audit.sh`, `abi_derive.sh`. ⊘ Read-only — another agent owns that
repo.

### The order to work in

1. **Reproduce in the raw client first.** The LLM hangs in CUDA setup with **ZERO doorbells**,
   and the client already drives channels, memory objects, mappings and copies to a pass. So the
   gap is what CUDA does that the client does not — a rung, not a hang inside a runtime.
2. The tree already records the wall sits **above the ioctl boundary**, in a handful of controls
   only the CUDA runtime issues (~105 ioctls vs the passing driver). That is the list to extend
   the client with.
3. Only then reach for `gdb` on libcuda, or debug prints in `ogkm` — both are allowed and both
   have precedent here.
