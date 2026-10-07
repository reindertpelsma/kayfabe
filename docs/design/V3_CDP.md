# V3 CDP — CUDA dynamic parallelism: why the child grid never ran, and the fix

**STATUS: ANSWERED + FIXED, 2026-09-30 — branch `v3-cdp`, merge bar passed at `2830988f` (§5.3).**
Fix commit `46509dce`; every CDP number below was measured with the kf3 binary of `090b20d9`
(= `46509dce` + harness scripts, no code change) unless it names another revision. Box: vast 53004208
(ssh `v3060`), RTX 3060 GA106, nested KVM, host and guest driver 580.159.04 (open). Evidence:
`traces/v3_cdp/` (its README names every run and revision).

Every NVIDIA source citation is to `ogkm-580` (`research_clones/ogkm-580.159.04`, the bench's own
driver); paths start at that tree's root.

Supersedes, for CDP: `V3_APP_MATRIX.md` §R3's *"2026-09-28 recovery correction"* (the bisect and the
probe it cites stay true; their cause is §3 here) and the *"CDP child launch"* item queued in
`STATUS_AND_HANDOFF.md` §0.

## 0. The answer

libcuda gives every context one 4 KiB **SKED-reflected page** (`UVM_MAP_DYNAMIC_PARALLELISM_REGION`;
`[measured]` in every probe run including mode 0, and 109 placements in one boot of the 71-row app
matrix — §5.4 — so every CUDA context, not only CDP programs, carries one).
A device-side launch is a GPU *write through that page*: its PTE carries the message kind
`SMSKED_MESSAGE`, and the GMMU delivers the write to the scheduler (SKED) instead of memory. The guest's
UVM writes that PTE into the guest's page tables; kayfabe's walker carried it (kind included) to the
host, and `kf_mem::apply` turned it into an **ordinary memory row** — the store at the leaf's address
(0), mapped with kind **PITCH**, because `host_pte_kind` maps every kind it does not know to PITCH.
On the host twin the device runtime's launch was therefore a plain store into guest vidmem: no child
grid ever started, the parent grid (which completes only after its children) never completed, and
every wait on its stream hung — silently: no fault, no RC, no host Xid (the host kernel log is empty),
and no non-OK RM status beyond two unrelated, already-classified ones that the passing mode 0 shows too.

The fix places a SKED-reflected leaf as what it is: a host mapping with the message kind
(`MapTarget::map_sked`, §4). With it, every launch shape of the probe runs its child with correct
output, and `cdpSimpleQuicksort` validates at 128, 1 000 and 10 000 elements (§5).

## 1. The symptom, as handed over

- `cdpSimpleQuicksort` passed in R2 only because kf3 answered `MC_SERVICE_INTERRUPTS` with `0x56`,
  which the CUDA waiter read as "wait over" — a forged completion, removed in `56032c46`
  (`V3_REFUSAL_AUDIT.md` §1). Since then it timed out (`V3_APP_MATRIX.md` §R3).
- The recovered probe (`scripts/cdp/cdp_probe.cu`, unchanged from the recovery bundle) showed the parent
  running and ending, `child_ran=0`, and `cudaDeviceSynchronize` hitting its 20 s watchdog; the same
  probe on bare metal ran the child (`traces/recovery_20260928/`).
- Every CDP boot logged `kf3_refusals=4` — the first lead (§2.3).

## 2. Measurements, bare metal first

### 2.1 Bare metal (the control)

`scripts/cdp/cdp_host.sh` on the box's host GPU, 2026-09-30 17:55 and 18:13 UTC
(`traces/v3_cdp/bare_metal/`): every probe run `RESULT … OK` with `child_ran=1 out[1]=0xc0ffee` —
modes 1 (NULL stream), 2 (fire-and-forget), 3 (tail launch), 4 (device-created non-blocking stream;
auto, spin and block scheduling) — mode 0 (no device launch) OK, and `cdpSimpleQuicksort`
`Validating results: OK` at 128, 1 000 and 10 000 elements. No host kernel message in either window
(`journalctl -k`).

### 2.2 The host-vs-guest ioctl diff

Every `/dev/nvidia*` ioctl of the probe was recorded with the nvdiff shim (before/after parameter bytes;
`scripts/cdp/nvdiff/`, copied unchanged from the pre-v3 tree) on bare metal and in a fresh kf3 guest
running master's code (`3f67ed95`, `kf3-bin-rev:3f67ed95`), then aligned with `nvdiff.py`
(`traces/v3_cdp/nvdiff/`):

| pair | records | aligned | divergences that are not environment (fds, UUID, tokens) |
|---|---|---|---|
| host mode 0 vs host mode 4 (noise floor) | 443 / 443 | 1.000 | **none** — a device-side launch costs **zero** ioctls |
| host mode 0 vs guest mode 0 | 443 / 443 | 1.000 | 2 STATUS: `0x20810108` (NV2081 BINAPI) and `0x2080200a` (`PERF_BOOST`) answered `0x56` |
| host mode 4 vs guest mode 4 (`3f67ed95`) | 443 / 457 | 0.984 | the same 2 STATUS, then **14 × `0x20801702` `MC_SERVICE_INTERRUPTS`** — the waiter polling a stream that never completes (bare metal: 0) |
| host mode 4 vs guest mode 4 (fix) | 443 / 443 | 1.000 | the same 2 STATUS only |

⇒ The control plane runs in lockstep with hardware up to the wait; the two STATUS rows are present in
the passing mode 0 too and are already classified (`V3_REFUSAL_AUDIT.md`: `PERF_BOOST` performance
only, BINAPI unknown-but-harmless). The failure is **not** an RM control or allocation: it is on the GPU.
Record #359 of both traces is `UVM_MAP_DYNAMIC_PARALLELISM_REGION`, 4 KiB at a CPU-style VA
(`0x7aa3dec00000` in the guest), `rmStatus 0` on both sides.

### 2.3 The four refusals — not CDP's

The `kf3_refusals=4` of every CDP boot are, verbatim, two `GSP command service REFUSED: QueueNotBound`
(command doorbells rung before the GSP's queues are bound, at each GPU init — no persistence mode means
one init per CUDA process), `RPC-REFUSED AllocClassNotPermitted 0xC36F` (`VOLTA_CHANNEL_GPFIFO_A`, the
guest RM's RC-watchdog channel) and `RPC-REFUSED … NV40_I2C` (`NoPhysicalBoardBus`). The same four,
in the same order, are in passing apps' windows (`vectorAdd`, `matrixMul`, R3 `m20`); the two
allocations are classified in `V3_REFUSAL_AUDIT.md` (benign; correct to refuse). The count rose to 11
in the fixed run's app-matrix window because the refusal census now also prints its once-per-process
`GSP REFUSED fn76/…` lines (all seven classified there). ⇒ The lead was measured and is negative.

## 3. The leaf, and the one-line A/B

The mechanism is in the open driver:

- `UvmMapDynamicParallelismRegion` (`kernel-open/nvidia-uvm/uvm.h:2129-2195`): *"Creates a special
  mapping required for dynamic parallelism. The mapping doesn't have any physical backing, it's just a
  PTE with a special kind."* On Turing, Ampere and Ada the PTE is `VALID | KIND=SMSKED_MESSAGE`,
  aperture VIDEO, address 0 (`make_sked_reflected_pte_turing`, `uvm_turing_mmu.c:111-119`); Hopper and
  Blackwell use SYSTEM_NON_COHERENT (`uvm_hopper_mmu.c:220-231`).
- RM builds the same page for every compute object (`kgrobjSetComputeMmio_IMPL`,
  `src/nvidia/src/kernel/gpu/gr/kernel_graphics_object.c:405-474`), mappable with
  `NV_ESC_RM_MAP_MEMORY_DMA`: *"The address field is completely ignored for these mappings"* (`:444`).

**Experiment `exp1`** (`6498628f` on branch `v3-cdp-exp1`, not for merge): keep kind `0xF` on a vidmem
leaf's host map, and log every leaf of that kind. One fresh boot, 2026-09-30 18:00 UTC:

```
kf-mem: EXP1 SKED-kind leaf 0x75b470c00000+0x1000 ap=0 at=0x0 kind=0xf priv=false -> host kind 0xf
CDPP flags parent_started=1 child_ran=1 parent_end=1 | … t_streamquery_ok=0.041 …
CDPP RESULT mode=4 sched=auto OK            (modes 1, 2, 3, 4:spin, 4:block: OK)
QS Validating results: OK
```

⇒ The walker delivered exactly UVM's leaf (VIDEO, address 0, kind `0xF`); on master the apply mapped it
PITCH; the kind alone decides whether the child runs. `[inferred, not measured]` that each launch
then wrote guest vidmem page 0: that is what a PITCH PTE naming store offset 0 does — nobody read the
page back.

## 4. The fix (`46509dce`)

- **`kf_chip::sked`** — the message-kind decode, the hardware's own: kind `SMSKED_MESSAGE` over VIDEO or
  SYSTEM_NON_COHERENT is SKED-reflected; over SYSTEM_COHERENT it is internal MMIO (Hopper+'s usermode
  page, `kf_chip::usermode`). The kind value and the four aperture codes are held to every die group's
  ogkm header (`hwref_check`), so there is no family row: Turing … Blackwell share them.
- **`kf_mem::apply`** — a SKED-reflected run is never a `Desired` row. `apply_sked` bounds it like any
  row (whole pages, inside the store, a CPU window's extent, never over one of OUR placements or a
  refused unmap), applies v3-roperm's privilege policy, and hands a `SkedRow` to the new verb
  **`MapTarget::map_sked`**. The trait default **refuses by name**: answering "satisfied" with no
  mapping is exactly this bug. The apply's single invalidate now also follows a SKED placement.
  ⊘ Only SKED-reflected leaves are diverted: a SYS_COH message leaf on a family with no usermode MMIO
  (Turing … Ada; no producer known) keeps its previous path, unchanged.
- **`HostVas::map_sked`** — kf's own store slice, mapped FIXED at the guest VA with the kind overridden
  to `SMSKED_MESSAGE` (`NVOS46_FLAGS_PAGE_KIND_OVERRIDE_YES`). For a VIDEO leaf the slice is the guest's
  own address; for a SYS_NONCOH leaf it is the store's first page, so the host PTE never names host or
  guest-RAM physical memory (the address is ignored either way).
- **`kf_qemu::mem::GpuMirror`** — SKED placements are kept **out of** the resolvable rows (a Translated
  channel must never read "through" a SKED page), and are taken down by `unmap`, `unmap_range` and the
  retire (a recycled spare space must not carry one). `Target` forwards `map_sked` explicitly; the
  stats line counts `sked=<placed>/<held>held`.

**Why a store slice with a kind override, not RM's compute-object map.** Both write the same PTE (VIDEO
+ message kind + an ignored address). The compute-object map ties the page's lifetime to a host
compute object that belongs to a guest channel: freeing it auto-unmaps every DMA mapping of it
(`rs_client.c:1342-1395`), so a guest freeing one channel would silently strip the SKED page from a
space other channels still launch from. The store slice lives exactly as long as kf's own placement.
Host UVM (`UvmMapDynamicParallelismRegion`) would need kf's host spaces registered with host UVM — the
b3 decision's territory, not this bug's.

**The owner rules, checked.** *Authored, not forwarded:* the host verb is kf's own map of kf's own
store, with a kind derived from ogkm; no guest byte reaches it. *Completions are host events:* the fix
adds none — the child grid and the parent's completion are the host GPU's own. *Hostile guest:* a SKED
page launches work only in the writing context (the guest's own twin); any unprivileged host process
with a compute object can create one (`kgrobjGetMemInterMapParams_IMPL` is not privileged); a
privileged guest SKED leaf is withheld from a user twin like any privileged leaf. *No blocking on a
vCPU:* the map runs on the VA thread, inside the apply that already runs there; the SKED book's lock
is never held across a host call.

## 5. Results

### 5.1 The probe and `cdpSimpleQuicksort` (kf3 `090b20d9`, one fresh boot, 18:13 UTC)

| run | result |
|---|---|
| mode 4 auto / spin / block (device-created stream) | `child_ran=1`, `out[1]=0xc0ffee`, `RESULT OK`; the stream query turns OK 45–60 ms after the launch (bare metal, same box: 9–11 ms) |
| mode 1 (NULL stream), 2 (fire-and-forget), 3 (tail launch) | `RESULT OK` |
| mode 0 (no device launch) | `RESULT OK` |
| `cdpSimpleQuicksort` 128 / 1 000 / 10 000 elements | `Validating results: OK`, rc 0 |
| host kernel log (`journalctl -k`) over the boot | no entries |
| kf3 | `sked=10/0held` — one SKED placement per CUDA process, none refused |

The ioctl trace of the fixed mode-4 run is in lockstep with bare metal (§2.2, last row).

### 5.2 The app-matrix lane (the harness R3 used)

`apps_matrix.sh guest cdpfix_090b20d9 cdpSimpleQuicksort`, fresh boot, 18:17 UTC:
`APPRES side=guest app=cdpSimpleQuicksort verdict=PASS rc=0 secs=7 … guest_xid=0`.

### 5.3 The merge bar

**Passed at `2830988f`** (the fix + harness scripts + this note's first version), 2026-09-30
18:23–18:49 UTC, same box: crate tests **1752 / 0** (master's 1742 + the 10 new ones), gates **9/9**,
`KF3_RC=0`, bare-metal suite **30/30**, `FG_RC=0`, thin suite **30/30** (`traces/v3_cdp/README.md`
§Merge bar, `merge_check/mergecheck_2830988f.tgz`). ⚠ Neither the gates nor the thin suite run
libcuda, so they exercise the unchanged memory path, not `map_sked`; §5.1–5.2 are the SKED evidence.

### 5.4 The full app matrix at the bar's revision

`apps_matrix.sh host|guest m21cdp all` with kf3 `2830988f` (the bar's own binary), 18:49–19:34 UTC,
same box, same harness and predicates as R3 (`traces/v3_cdp/app_matrix_2830988f/`):

| run | result | R3 (`4c48ca0c`) |
|---|---|---|
| host (bare metal) | **71/71** | 71/71 |
| guest, no PM, ONE boot | **67/71 = 61/65 apps + 6/6 stream probes** | 66/71 = 60/65 + 6/6 |
| guest, each non-PASS alone | the same 4 fail alone | the same 5 |

- The four failures are R3's UVM demand-paging four, with R3's signatures (`UnifiedMemoryStreams`,
  `UnifiedMemoryPerf`, `conjugateGradientUM`, `attach_verify`). `cdpSimpleQuicksort` passes in the
  batched boot too. Output digests equal the host's for `torch_correct`, `hf_generate`, `llama_cpp_gen`.
- ★ The batched boot placed **109** SKED pages, none held or refused (`sked=109/0held`,
  `sked_census_b1.txt`): every CUDA context in the matrix went through `map_sked`, so this run is a
  regression test of the new path across the whole matrix, not only of CDP.

## 6. What is and is not established

**Established (measured, 2026-09-30, GA106, 580.159.04 both sides):** the cause (§3's A/B isolates the
kind); the fix makes all four CDP launch shapes run their child with correct output and
`cdpSimpleQuicksort` pass at three sizes, with the control plane in lockstep with bare metal; every
CUDA context maps a SKED page and the whole app matrix passes through the new path with no new failure
(61/65, §5.4); the four refusals are not CDP's.

**Not established:**
- **Hopper and Blackwell.** UVM's SYS_NONCOH SKED leaf takes the new path only in GPU-free tests; no
  VER3 GPU ran it. UVM's own comment says a VIDEO-aperture SKED PTE (what kf places) is valid on
  discrete Hopper; unmeasured here.
- **Hosts below 580.65.06** have no per-map PTE kind: the NVOS46 carry refuses the SKED map by name, so
  there a device-side launch faults on the twin instead of hanging. Unmeasured; the compute-object map
  would be the route there (§4).
- **The address is ignored** — RM's and UVM's word, and consistent with every run here; no run placed
  a SKED page at a non-zero store offset.
- **A killed hung CUDA process poisons the guest** (open, separate): after a pre-fix CDP hang was
  SIGKILLed, every later process in that boot failed `cudaSetDeviceFlags` with 999
  (`traces/recovery_20260928/53004208-cdp4.log.txt`, the `0667b784` and `56032c46` probe boots), and at
  `3f67ed95` the killed process's teardown logged `kmemsysDoCacheOp_GM107: - timeout error waiting for
  reg 0x70010` (`traces/v3_cdp/guest_3f67ed95/probe_4_auto_t.guest_dmesg.log`). No bare-metal
  counterpart was measured (bare metal never hung). The fix removes this trigger, not the underlying
  behaviour.
- **Instrument trap found on the way:** on a long-lived box the host dmesg ring is full, so
  `boot_capture.sh`'s line-count watermark yields `HOST_DMESG_LINES=0` for every boot — *unmeasured*,
  not "no host event" (all four recovered CDP boots carry it). `scripts/cdp/` records the host kernel
  log by time instead; `boot_capture.sh` itself is unchanged.
- The two classified STATUS divergences (`PERF_BOOST`, BINAPI) remain, as before.

## 7. Re-running

On a provisioned box (the app-matrix guest image `guest_apps.qcow2`, the bundle under `/workspace/apps`):

```bash
bash scripts/cdp/cdp_host.sh <tag> "0:auto:1 4:auto:1 1:auto 2:auto 3:auto 4:spin 4:block qs"
bash scripts/cdp/cdp_guest.sh /workspace/bench/kf3-bins/<rev>/qemu-system-x86_64 <tag> "4:auto:1 1:auto 2:auto 3:auto 0:auto:1 qs qs:10000"
python3 scripts/cdp/nvdiff/nvdiff.py diff <host>.jsonl <guest>.jsonl
```

A run spec is `mode:sched[:1]` (`:1` records the ioctl trace) or `qs[:N]`. Put a run that may hang
last: a killed hung process poisons the rest of the boot (§6).
