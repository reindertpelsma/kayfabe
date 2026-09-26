# V3_BUILD — how v3 is being built (branch `v3`)

**STATUS: LIVE, 2026-09-24 (w826).** Governs the build order and what is copied. The design is
`THE_ARCHITECTURE_v3.md` + `THE_V3_PLAN.md`; this file only records the execution decisions.

## Rules (owner, 2026-09-24)

- v3 is written **as designed**, in fresh `kf-*` crates. The old `kayfabe-*` crates are **frozen**:
  reference + grader (rm-ladder, fastguest harness, bare-metal suite, cup3/cup8/LLM hooks).
- Code may be **copied from the old tree whenever it fits v3**. The test is the code's shape, not its
  origin. Never copied: CPU reads of guest page tables, CPU-executed engine work, address
  tables/VA mirrors/joins, isolates/IPC plane, publication epochs/dirty gates/sweeps, per-page BAR
  fill traps, **and snapshots of the guest's tables** (the walk kernel's delta snapshot and
  `walkmirror` included — v3 §4.2 w825: *the snapshot is a shadow; the ledger replaces it*).
  > ⊘ **AMENDED 2026-09-25 (owner design + COMMIT-ON-ACK ruling; branch `v3-diff`).** The rule
  > above forbade the old walk kernel's snapshot for a reason that still stands: it was the
  > PREVIOUS WALK — guest table content, installed whether or not the host acted on it (a §18.2
  > shadow; `walkmirror` was its CPU copy), and the ack was one generation for the whole report.
  > What the walk kernel now keeps in vidmem is different in kind: per VA-space object, the
  > placements the host **confirmed it made**, committed run by run from the host's verdict — the
  > "ledger of our own map handles" §4.2 (w825) itself sanctions, moved beside the walker. The walk
  > still reads the guest's live tables every time; nothing is served from the record; a stale
  > entry costs one extra diff line, never a wrong translation. ⇒ The walk emits a **DIFF**
  > (`kf_cuda::diffmodel` is the spec; `kf-gate9` holds the GPU to it) and the CPU ledger +
  > `plan_reconcile` are gone (`V3_P5_PORT_MAP.md` Q8). ⊘ Still never copied: `walkmirror`, a
  > whole-generation ack, anything that installs the walk itself as "previous state".
- The target is the v3 architecture; **LLM parity is what v3 must deliver**, never a shortcut.
  Temporary breakage is accepted.

## Reference sources, per open question (checked w826)

Pinned in `third_party/` (ogkm 610 + 580, linux for nouveau/nova, gvisor for nvproxy); nvkvm-pv at
`/workspace/nvkvm-pv`. Rank: ogkm > nova > nouveau (THE_CONSTRAINTS §50); nvproxy answers the
host-userspace question only.

- **Per-die tables** → HOST-QUERY, unprivileged (ogkm `g_subdevice_nvoc.c` flags, `0x8` =
  NON_PRIVILEGED): interrupt data via `MC_GET_STATIC_INTR_TABLE 0x2080170e`,
  `MC_GET_ENGINE_NOTIFICATION_INTR_VECTORS 0x2080170d`, `MC_GET_INTR_CATEGORY_SUBTREE_MAP 0x2080170f`
  (the INTERNAL `0x20800a5c` and `FIFO_GET_DEVICE_INFO_TABLE 0x20801112` are kernel-only); engines
  `GET_ENGINES_V2`, `GET_HW_ENGINE_ID`; GR info, CE PCE mask, CE caps, FB info V2, arch info, name,
  GPU info V2 — all NON_PRIVILEGED. Gaps: nova/nouveau device-info decoding as family rules. ★ The
  old tree's captured GA106 rows are the TEST ORACLE: on a GA106 host, derived == captured.
- **Host event → guest MSI** → nvkvm-pv already materialises a host eventfd for
  `NV01_EVENT_OS_EVENT` (`src/qemu/nvkvm_handle.c:134-142`; the event fd is a `/dev/nvidia*` fd,
  `nvkvm_isolate_handlers.c:2615-2618`); the eventfd → KVM irqfd → MSI-X hop is standard KVM/QEMU.
- **Per-family GSP boot** → nova-core (`linux/drivers/gpu/nova-core`, Rust, GA10x+ FWSEC/WPR2).
- **Host control allowlist + layouts across driver versions** → nvproxy.
- **Bench grading** → the old project's nvdiff live oracle + bare-metal suite.

## The crate map (from the w826 five-way inventory)

| v3 crate | built from (copy / adapt) | new |
|---|---|---|
| `kf-util` | copy `kayfabe-util` | — |
| `kf-arch` | copy `kayfabe-arch` (GSP seam; GMMU/USERD traits later) | — |
| `kf-abi` | copy `kayfabe-abi` (generated modules, wire/view/versions, table layouts); `oracle.rs` test-only | regenerate from ogkm later |
| `kf-gsp` | copy `kayfabe-gsp` {ring, element, ram, rpc, fault, seq}; **adapt** `boot.rs` (drop HeldReply / holds_for_refresh / deferred lane); `kayfabe-device/abi.rs`; drop `replay.rs` | — |
| `kf-chip` | ⊘ **NOT a copy of `kayfabe-device/ga10x.rs`** (a GA106-only captured-constants file — the per-die shape v3 exists to end). Built on the **axes** (`derive_per_die_maintain_per_family`, owner 2026-09-13): **(a) FAMILY row, hand-maintained, one per generation** (`ga10x`/`ad10x`/`gh100`/`gb20x`: MMU format regime, USERD model, doorbell encoding, GSP boot style SEC2-booter vs FSP, ogkm header dir); **(b) register offsets GENERATED** from ogkm `dev_*.h` per family by the swref parser (`kayfabe-doorbell/swref.rs`) and class sets by `classgen.rs`; **(c) per-die facts READ FROM THE HOST GPU** at realize through unprivileged RM queries, else derived from ogkm, else fabricated so RM's own code accepts it — **each field carries its provenance**. The driver-version axis stays in `kf-abi`/`kf-gsp` (element layout keyed by version). | the generator, the `HostFacts` query set, the family table |
| `kf-trap` | `kayfabe-doorbell` {token, bitmap, wake, ring, trap, trappolicy, shadow, memmap}; the inline passthrough store | thin register adapter (~200 lines) replacing `RegPlane` |
| `kf-rm` | `kayfabe-device` {inittables, staticinfo, guestsysinfo, inert, unserviced, census, sticky}; `kayfabe-rmrpc` {translate, reasm, fault}; doorbell {rmgraph, accessmap, rpc} | object graph keyed `(hClient,hObject)`; `GPU_GET_NAME_STRING` from the host |
| `kf-host` | `RmConnection` from `isolate-host/rm.rs` (raw verbs public; no `HostRmBackend`, birth clients, exec VAS); `map_local_at_with_flags` + `invalidate_tlb`; `birth_channel`, `alloc_engine_object`, schedule; `kayfabe-linux-raw` chardev/ioctl/mapping/window/eventfd | deferred **unmap** flag; OS-event / NONSTALL registration |
| `kf-mem` | existing `kf-mem`; `kayfabe-cuda` + `cuda/walk` kernel (walk + the commit-on-ack DIFF against confirmed placements — ⊘ 2026-09-25, replaces "no delta snapshot" + the CPU ledger/planners); `reconcile_scoped` | our BAR1/BAR2 roots declared in static info; batched map with TLB-defer + one invalidate; scoped walks by named PDB |
| `kf-chan` | doorbell {channel, completion, plane, hostverb}; `kayfabe-rt/translated.rs`; birth rules from `kayfabe-fwd` (adoptability by GPGA arithmetic, not bindings); `kayfabe-completion` policy | **Translated execution** (windows, own ring, GP copy, split at MEM_OP); **host event → irqfd → MSI** |
| `kf-core` | doorbell `vmm.rs` seam | — |
| `kf-qemu` + `qemu/hw/misc/kf3/` | QOM glue + `KayfabeHostOps` from `nvkvm.c` / `shim_unsafe.rs`; `vmm-qemu` window verbs | device `kf3-gpu`, `kf3_` symbols, `KF_DEVICE` in the 3 harness sites |

⊘ **CORRECTED w826 (`V3_P2_PORT_MAP.md` §4.1): `kayfabe-device/sweep.rs` is NOT a publication
sweep** — it is the control TRIAGE table (`SWEEP_TRIAGE`, :83-171) for the guest's engine sweep, and
dropping it re-creates `t134a`'s silent engine amputation. It is COPIED into kf-rm. The name misled
the list below. The full P2/P3 copy order is `V3_P2_PORT_MAP.md`.

**Dropped outright:** `kayfabe-device/plane.rs` (except GSP dispatch ideas), fbwin/gpgaview/ceresolve/
gvaspub/pubqueue/mmuinval/bar2/setpagedir, `kayfabe-core` gpu/project/gpa/reactor/promote-join,
`kayfabe-rt` device/ceutils/completion_watch, `kayfabe-fwd` CE half, isolate crates, barmirror,
deviceview, walkmirror, the whole-generation delta/ack (⊘ 2026-09-25: the per-run commit-on-ack diff is a different thing — see the amended rule above).

## ★★★ Order: the real planes FIRST, graded by a GPU harness — guest boot LAST (owner, w826)

> *"I would not go to guest boot with fake planes. Since you need emulated channels and translated
> to boot in v3, its better to postpone that until this land. Otherwise you are going to race
> towards a inline synchronous fake completor which is precisely what we need to avoid. Thats why
> your test infrastructure is actually important."*

The stock driver cannot pass `RmInitAdapter` without its kernel CeUtils channels completing. Booting
before the channel and completion planes exist forces a synchronous fake completion on the vCPU —
the CPU executor under another name. ⇒ **No guest boot until kf-host + kf-mem + kf-chan are real.**

1. `kf-host` — in-process RM session; one reserved object; identity + guest-RAM windows; VAS; batched
   map/unmap with TLB-defer; channel birth; host events → eventfd.
2. `kf-mem` — the store; OUR bar1/bar2 roots; the walk diffed against our handle ledger.
3. `kf-chan` — passthrough birth; Translated execution; VMM-executed emulated channels (§37);
   completions via host event → eventfd → one wake path, **never inline**.
4. **`kf-harness` (the gate for 1–3): the harness plays the guest, no QEMU.** Guest RAM = a memfd,
   guest VRAM = the store. It writes real page tables into the store, places a kernel-style CE
   pushbuffer (PHYS operands + semaphore release) in guest RAM, rings the token, and asserts: the
   REAL engine copied and released, the completion arrived via the eventfd, `forwarded=` per token,
   the `MEM_OP` split published before resuming, hostile rings refused by name.
5. Only then P2 guest boot (`kf-trap`, `kf-qemu`), P3 `nvidia-smi`, and the 30-arm suite.

P2's non-booting work (`kf-chip` generator, `HostFacts`) proceeds in parallel.

## Order and gates (THE_V3_PLAN §2)

P2 GSP boot → `GSP_INIT_DONE` + first RPC · P3 objects/controls → `nvidia-smi` correct · P4 memory/VA/BAR
→ alias/map/invalidate arms · P5 channels → ring/doorbell/concurrency arms · P6 Translated → kernel scrub
`forwarded>0`, ce-client* · P7 compute → cup3, cup8 · then the LLM lane at ≥0.8× host tok/s.

★ **Host baseline, recorded w826** (box 52430332, RTX 3060 GA106, 580.159.04, torch 2.6.0+cu124 —
the same pinned wheel the guest gets, transformers 5.17.0, `Qwen/Qwen2-0.5B-Instruct`, 16 tokens
greedy, `scripts/bench/run_llm.py` = the guest's own runner): **`HOST_LLM_TOK_PER_S=10.43`**
(`generate()` wall, COLD — the basis the guest hook reports). ⊘ The line this replaces said "never
recorded"; w720 had measured one on another box (0.20× parity). A denominator belongs to ITS box:
re-record on the box the guest number comes from.

★★★ **LLM lane measured on kf3, w828 (branch `v3-llm`, device built at `53c2bc20` = master `f8cfe8d7`
+ the cache-op fix `8dd2bbdf`).** Fat guest (`boot_capture.sh`, `KF_DEVICE=kf3`, 8 GiB, 4 vCPU) on
`vh3` (itself a KVM guest — nested), the same box's host, and `vc` (non-KVM container, bare metal).
Matrix `scripts/bench/llm_parity.sh`, tables `llm_parity_summary.py`, raw output `traces/llm_parity/`.
Each cell mean of 3 processes.

| tok/s | guest (kf3) | vh3 host | vc bare | guest/vh3 | guest/vc |
|---|---|---|---|---|---|
| short 16-tok cold (recorded basis) | 3.10 | 8.94 | 13.40 | 0.35 | 0.23 |
| 512 steady decode (warm) | 5.89 | 19.33 | 28.28 | 0.30 | 0.21 |
| 2048 steady decode (warm) | 6.02 | 19.50 | 28.26 | 0.31 | 0.21 |

- ⊘ **Long runs do NOT amortize the gap: it is per TOKEN, not per process.** Guest text == host text
  for 16/512/2048 tokens (sha256). Ledger: every token `emulated=0`; every passthrough token
  `forwarded>0`. **~1080 doorbells per decoded token** (eager HF decode = ~1000 kernel launches), and
  doorbells are 99.7% of all trapped exits (16.79M of 16.84M in llm_g1): ≈106 µs of guest time per
  doorbell on this nested box. 0.8× would need ≤~8 µs per doorbell — no trapped exit reaches that
  here. ⇒ parity for eager decode needs an EXIT-FREE doorbell (owner decision), not setup work.
- Per-process fixed cost: guest persistence mode (`nvidia-smi -pm 1`) removes the per-process
  emulated-GSP unload/reboot (11 → 1 `UnloadingGuestDriver`) and ~3-4 s per process.
- ⊘ **WALL: any kernel on a non-default CUDA stream faults** — host `Xid 13 SKEDCHECK05_LOCAL_MEMORY_
  TOTAL_SIZE failed` on the second GR twin (bisect `llm_graph_probe.py`: default stream incl.
  StaticCache passes; `x*2+1` on a side stream fails). Every twin is born in its OWN host TSG
  (`kf-host/src/channel.rs` `birth_channel`), so TSG-scoped GR state the guest programmed through its
  first channel is absent from the second's context (`CtxBind initialized: 0` vs `3591`). Blocks the
  CUDA-graph arm (`run_llm_graph.py`: bare 169.6 tok/s, host 165.6, 6x eager) and any multi-stream app.

★★★ **Same matrix on AMD, w828b (`vh`: EPYC 7452 KVM guest, RTX 3060, 580.159.04; device `e5ff45ff`
= the above + the sysmem-containment fix `f4813205`).** Raw: `traces/llm_parity/vh_amd/`.

| tok/s | guest (kf3) | guest + PM | vh host | vc bare (Xeon) | guest+PM/vh | guest+PM/vc |
|---|---|---|---|---|---|---|
| short 16-tok cold | 4.80 | 5.81 | 17.19 | 13.40 | 0.34 | 0.43 |
| 512 steady decode | 12.62 | 12.58 | 43.98 | 28.28 | 0.29 | 0.45 |
| 2048 steady decode | 13.04 | 12.69 | 43.53 | 28.26 | 0.29 | 0.45 |

- Exits are ~2x cheaper than on vh3's Intel, but the EPYC host is also 2.2x faster, so the ratio is
  unchanged (0.29x vs 0.31x). Per 2048-token process (PM lane): **1084.5 doorbells and 1303 KVM
  exits per token** (5.34M exits, 4.44M ledger doorbells). Excess per token 55.8 ms ⇒ **~51 µs of
  guest time per doorbell** (42.8 µs per exit on average). `perf kvm stat` in the box (L1): npf
  (the doorbell MMIO write) is 70% of exits at 18.5 µs average L1 handling; the rest is L0 nesting.
  0.8x needs ≤ ~5 µs per doorbell.
- ⊘ **Fixed: sysmem walk runs were bounded by the vidmem store span** (`f4813205`): UVM's CE channel
  died on a valid guest page above 8 GiB GPA in any 8 GiB guest. ⊘ **Open: without guest persistence
  mode, UVM's CE channel dies on the 5th CUDA process of a boot** (both boots, `vhA_g`/`vhB_g`):
  `split walk (pdb 0x201000): walk report TRUNCATED (flags=0x85, refuse_mask=0x10 RUN_CAP)` — each
  process re-inits the adapter; with PM (one init) 9 processes ran clean. Looks like placements for
  UVM's re-created VAS accumulating across adapter cycles; not fixed.
