# w316 — WHAT THE C DID PER LAUNCH, AND WHY IT WAS FAST

**STATUS: LIVE — written 2026-08-14 (w316). SOURCE-ONLY attribution study, no boot, no build.**
Read against `w311_throughput_ratio_and_the_llm_question.md` (the ~100 ms measurement this
exists to explain) and `w315` (measuring the same floor directly on hardware — the two rungs
should be able to agree or disagree).

Sources: the C artifact at `/workspace/nvidia-gpu-passthrough` (`src/qemu/nvkvm_gpu_emul.c`,
9987 lines, HEAD `c86b69d`), its design docs, and the C-era agent memory
`/root/.claude/projects/-workspace-nvidia-gpu-passthrough/memory/mode2_execfwd_layer2.md`
(the first-person perf log of the 2026-06-15 investigation — **this is where the C's own
per-doorbell numbers live**, and nothing in `docs/` reproduces them).

---

## 0. ⊘⊘ TWO THINGS IN THE BRIEF ARE WRONG, AND ONE OF THEM IS IN `CLAUDE.md`

### 0.1 ⊘ "The C reached 60 tok/s" — it did not. **60 was a GOAL, and its provenance is Mode-1.**

`20→60 tok/s` is the **title of a plan**, not a result:
`docs/design/mode2_userbuf_vidmem_passthrough.md:530` — *"PERF investigation — generation
22→60 tok/s (2026-06-15, measure-first)"* — and the memory log states the goal's origin
explicitly: *"GOAL: close the ~3× gap to **Mode-1's ~60 tok/s**"* (`mode2_execfwd_layer2.md:1702`).
**Mode-1 is a different architecture** (guest forwards userspace ioctls; no emulated GPU, no
doorbell trap at all), and its number is 63 t/s (`mode2_bar0_trap_reduction.md:7`).

What Mode-2 actually measured, with logs, on the nested vast.ai box:

| when | Mode-2 LLM gen | source |
|---|---|---|
| pre-fix | **0.1 t/s** | `mode2_execfwd_layer2.md:1666` |
| sweep-trigger fix | **20.1 t/s** | `:1690` |
| steady, whole campaign | **19.5 – 26.6 t/s**, ±40 % run-to-run | `:1836-1838` |
| best single observation | **38.3 t/s** once; ~36 when the box was quiet | `:1888`, `:2039` |

The plan **never reached 60 on the instrumented box.** It was closed out at
`mode2_bar0_trap_reduction.md:104-108` with *"gen is bound by per-token guest+host work + the
±40 % nested-virt variance."*

★ **There IS a near-parity Mode-2 LLM number, and the brief should use that one instead:**
`docs/MILESTONES.md:12-13` — **49.9 tok/s ≈ host-native 47.5 tok/s** on **bare-metal** box .32
(RTX 3050, non-nested), with the explicit attribution *"the vast.ai 20→50 t/s gap was entirely
nested-virt vmexit tax, not Mode-2 design."*
⊘ **But grade its evidence honestly**, and this tree already did:
`kernel_gr_channels_and_the_mme_exposure.md:396-400` records that the 49.9 figure's evidence is
**documentary only** — commit `6942048` is docs-only (verified: 1 file, docs, 2026-06-16), and
**no log or trace of that run is committed**, unlike the matmul. Treat 49.9 as *plausible and
uncorroborated*; treat 20–38 as *measured*.

⇒ **For this study it does not matter much.** Even the pessimistic, fully-instrumented
20–26 t/s number is ~500× our 0.04 t/s, and the C's *per-doorbell* cost was measured directly.
That measurement, not the token rate, is the oracle.

### 0.2 ⊘⊘ `CLAUDE.md` HAS `m2hostsem`'s POLARITY BACKWARDS — and the brief inherits it

`CLAUDE.md` (w275 correction block) says:

> **`m2hostsem=0`** — the host-semaphore forge was **OFF**. The C's `SET_REPORT_SEMAPHORE`
> CPU-forge exists (`:6544-6573`) and **wrote nothing** to the completion page in the green runs.

**The flag gates HOST OWNERSHIP, not the forge.** Definition site,
`nvkvm_gpu_emul.c:9930`: `DEFINE_PROP_BOOL("m2hostsem", …, false)` — */\* M5.35: **host owns
completion sema** \*/*. And the predicate, `nvkvm_chan_sem_wr32`, `:5555`:

```c
bool hostonly = s->m2exec && s->m2hostsem && nvkvm_m2_is_user_ce(s, s->chan_client);
```

with the write itself at `:5726`, guarded `if (p != NVKVM_GMMU_FAULT && !hostonly)`:

```c
nvkvm_phys_wr32(s, p, sy, payload); wrote = true;
```

⇒ **`m2hostsem = 0` ⇒ `hostonly = false` ⇒ the CPU forge is ON, not off.** The C's own comment
says so in as many words at `:5749`: *"With **software completion active (!m2hostsem, the
default)** the LAGGING bridged host channel must NOT have write access…"*

★★★ **And the second conjunct is stronger than the first.** `hostonly` also requires
`nvkvm_m2_is_user_ce(chan_client)`. That list is populated at exactly one site, `:7855-7865`,
under an explicit guard — `if (!nvkvm_m2_is_gr_client(s, client) && …)`, commented *"never
record a GR compute client … as a user-CE client"*. So **a GR client can never be a user-CE
client, and `hostonly` is `false` on the compute plane unconditionally, at every flag
setting.** The C's
`SET_REPORT_SEMAPHORE` (`0x1b00`/`0x1b04`/`0x1b08`/`0x1b0c` at `:6544-6570`) therefore wrote the compute
completion semaphore in software **on every green run, including `cap3`.**

⚠ This is the exact class `same_flag_opposite_polarity.md` already banked, and it is load-bearing
here: **it is the difference between "the C's completion came from hardware" and "the C's
completion was satisfied by a CPU store inside the doorbell trap."** The C measured the latter
independently — see §2.

⊘ **Scope this correction, do not over-swing it.** It does **not** refute w275's finding that
the host GR engine executed the guest's ctx-init pushbuffer and wrote `0x2_0440fff0`. Both can
be true: the software writer is an **additional, earlier** writer, not an exclusive one, and
w275's evidence (the engine ran) is about *execution*, not about *who won the race to the
semaphore*. What it does refute is the inference *"therefore the C's guest waited on hardware
completion latency."* It did not have to. ⚠ **Unresolved from source: which writer landed first
in `cap3`.** A trace could answer it; I did not run one.

### 0.3 ★ The brief's central premise — "whatever the C did per launch was cheap" — **HOLDS**, and is now a number

Not inferred from tok/s. The C instrumented its own doorbell path (`nvkvm_t_doorbell_ns`,
`:989`) and reported per-window:

**`db ≈ 0.94 ms per doorbell`** (`mode2_execfwd_layer2.md:1856`), later **~890 µs** after the
binary-search index (`:1893`), with `db_wall` ≈ 327–363 ms per 400-doorbell window and
`db_CPU` ≈ 58–83 ms of that (`:1915-1918`) — i.e. **~188 µs of actual CPU work per doorbell**,
the rest off-CPU descheduling under nested virt.

w311 fits **~100 ms fixed per submit**, and notes *"283 GrCompute doorbells over ~90 launches
≈ 3 per launch ⇒ **~33 ms per doorbell** if it is all there"* (`w311…md`, §6).

> ### ★★★★★ **THE HEADLINE: ~0.94 ms per doorbell (C) vs up to ~33 ms per doorbell (us). ~35×. And the C's number INCLUDES a full GPFIFO walk.**

---

## 1. ONE LAUNCH THROUGH THE C — the functions, in order

Everything below happens **synchronously inside a single guest MMIO write**, on the vCPU
thread, and returns before the guest's next instruction retires.

**Entry:** `nvkvm_bar0_write_inner`, `if (off == NVKVM_VF_DOORBELL)` — `nvkvm_gpu_emul.c:3835`.
The guest writes its work-submit token; that write is the whole trigger.

1. **`nvkvm_m2_exec_doorbell(s)`** — `:8927`, timed at `:3952-3956`.
   1. **New-work scan** (`:8968-8978`): for each channel, one `nvkvm_fb_read(userd + 0x8C)` to
      read GP_PUT, compared against a per-channel `sweep_put` latch.
   2. **Page-table sweep** (`:8994-9019`) — **GATED OFF in steady state.** `want` requires
      `m2_gr_vas_dirty` (a guest write to a tracked GR page-table page), a new VAS, a truncated
      walk, or a 1-in-256 insurance net. Measured: **re-sweeps 11487 → 41, walks 91960 → 392,
      and ZERO sweeps during generation** (`:1689`, `:1737`).
   3. **Host token fetch** (`:9033-9045`) — one-shot per channel, `token_valid` latched.
   4. **GPFIFO walk** (`:9071-9085`): read each new GP entry (`e0`/`e1`), extract the pushbuffer
      VA, `nvkvm_m2_va_seen()` dedup, and `nvkvm_m2_back_and_map()` **only for a
      genuinely-new pushbuffer**. Measured: `newpushbufs = 0` on every steady-state doorbell
      and `backmap = 0 ms / 0 calls` (`:1895`, `:1935`).
   5. **★ THE RING** (`:9160`): **one 4-byte CPU store into an mmap'd host page.**
      ```c
      stl_le_p((uint8_t *)s->m2_usermode_qva + 0x90, tok);
      ```
      `m2_usermode_qva` is QEMU's `mmap` of the host `AMPERE_USERMODE_A` doorbell page
      (`:689-691`). **That store IS the host submit.** No ioctl, no RPC, no process boundary.
2. **Per-channel `nvkvm_chan_execute(s)`** — `:5791`, called at `:4105-4108`. Walks the GPFIFO,
   decodes the FERMI method stream, executes CE `LAUNCH_DMA` as a **page-batched CPU
   `memcpy`/`memset`**, and on `SET_REPORT_SEMAPHORE` / `SEM_RELEASE` calls…
3. **`nvkvm_chan_sem_wr32(s, sem_addr, payload, &redir)`** — `:5546`. Resolves the semaphore VA
   under the channel's content-validated VAS and **writes the payload with a CPU store**
   (`:5726`), plus the BAR1 mirror page libcuda actually polls (`:5763-5768`).
   ⇒ **The completion the guest is about to spin on is already there when the trap returns.**
4. **`nvkvm_gsp_deliver_events(s)`** — `:1849`, called at `:4364-4368` if any channel advanced:
   POST_EVENT for each registered os-event, then `nvkvm_gsp_raise_swgen0()`. Gated to one
   outstanding batch (`:1857-1862`). Measured cost: **~0 ms** (`:1853`).
5. Return from the MMIO write.

**Steady-state host RM ioctls on this path: ZERO.** Both by construction (every `control1` /
`back_and_map` site is one-shot-latched) and by measurement — `backmap = 0 ms / 0 c`
(`:1895`).

**Fault handling on this path: NONE.** `grep -cE 'fault_buffer|FAULT_BUFFER|replayable|mmu_fault'`
over the whole 9987-line file returns **1**, and that one hit (`:9726`) is an unrelated comment
string. The C has **no fault-buffer emulation at all**, consistent with `CLAUDE.md`'s finding
that it went green *"without servicing or forwarding a single GPU fault."*

⚠ **`m2opaque` was OFF in the ladder boots.** `:9932` default `false`; `bench_boot.sh:56` sets
only `NVKVM_M2CEFWD=1`. So the ~0.94 ms figure is for a doorbell that **did** do the GPFIFO
walk. Turning the walk off measurably **did not help** (`:1962` — *"MECHANISM IS CORRECT. BUT
PERF DID NOT MOVE"*).

---

## 2. WHAT THE C PAID PER LAUNCH, IN ITS OWN TERMS — quoted, not constructed

The C carries a full time-share instrument: `nvkvm_timeshare_dump` (`:1057`) and the per-window
`nvkvm_timeshare_window_dump` / **`NVKVM-TWIN`** (`:1104`), which prints `INSIDE_qemu%`,
`db=…/…us_per`, `db[cpu=…]`, `sweep`, `resolve`, `fbrd`, `overlay_iters`, `vaseen`, `backmap`,
`event`, and `OUTSIDE(guest+spin)%`. It fires every 400 doorbells (`:4371-4374`).

Verbatim from its own log (`mode2_execfwd_layer2.md`), generation phase, ~23–26 t/s:

| quantity | value | line |
|---|---|---|
| **per-doorbell wall** | **≈ 0.94 ms**, later ~890 µs | `:1856`, `:1893` |
| per-doorbell CPU | **≈ 188 µs** | `:1918` |
| doorbells per token | **~25** | `:1856` |
| `INSIDE_qemu` (all QEMU work) | 55–65 % of gen wall | `:1851` |
| `OUTSIDE` (guest launch + libcuda spin) | 35–45 % | `:1852` |
| **`event` (completion delivery)** | **≈ 0 ms** | `:1853` |
| `sweep` during generation | **0** | `:1737` |
| `backmap` (host RM ioctls) | **0 ms / 0 calls** | `:1895` |
| `resolve` (GMMU walk) | ~5 ms / window (≈ 12 µs/db) | `:1894` |
| model-load bandwidth (CPU copy) | 469 MB in 28.6 s of load-dominated elapsed ⇒ **≥16 MiB/s** | `:1743`, `mode2_userbuf…:519` |

★★★ **The single most useful line the C ever wrote about completion**, `:1853-1855`:

> `event (nvkvm_gsp_deliver_events os-event wake) = ~0ms` ⇒ **H2 (completion-sema visibility /
> os-event delivery latency) is REFUTED.** Completion delivery is essentially free; libcuda's
> spin is NOT waiting on a late sema — **the sema is written in software during the doorbell
> trap.**

⊘ **Note what the C ALSO ruled out by experiment**, so we do not re-derive it: clocksource
(`m575`, null 4-boot A/B, `:1835-1838`), logging (`m577`/`m578`, null, `:1864-1866`), the
GPA-window CPU trap path (`win_wr` 6 ms / 17 524 calls over the **whole run**, `:1738`), the O(n)
overlay scan (real, ~9 %, `:1893`), and host-CE forwarding (`m570`: the bulk copies are
PHYSICAL-mode and untranslatable — `:1762-1766`).

---

## 3. ★★★ THE DELIVERABLE — structural differences on the per-launch path, RANKED

★★★★★ **The Rust column is not inferred — it is MEASURED, from our own committed artifact.**
`traces/w311_bench/run_w311bench_qemu.log.zst` (on master, 25 214 lines) is the device's own
report for the w311 boot, and it prints one `DOORBELL-XLATE`, one `RING-PROJ`, one `PT-DECODE`
and one `VAS-BIND-CENSUS` **per doorbell**, with their work counts. **877 doorbells** — 594
`engine=Ce`, **283 `engine=GrCompute`** (matching w311 §6's "≈3 per launch" over ~90 launches).

⊘ **The log's `t=+…ms` stamps are TICK-QUANTISED** (`GR-CURSOR … tick=333`, `SEMA-PAGE …
why=tick`) — this is w311 §6's 251 ms observer clock. **I did not compute any per-doorbell
latency from them**, and neither should anyone else. Everything below is a **work count**, not
a time.

### 3.0 First, the shape of the whole comparison

| step (brief's list) | the C | us |
|---|---|---|
| trap under BQL | **same** — QEMU MMIO write handler, `:3835` | same |
| read the ring | `for (idx = c->gp_get; idx < gp_put; …)` — **the NEW entries only**, `:9071` | **`scan=1024/1024`, ALL 877 doorbells** |
| decode | **more than us** — full FERMI method decode *and* CPU-executes CE ops, `:5791` | pushbuffer method decode |
| translate | `nvkvm_chan_translate` GMMU walk; measured ~12 µs/db (`:1894`) | per-VAS resolve |
| **check publication** | **ABSENT** — dirty-gated sweep, **0 sweeps in generation** (`:1737`) | **`PT-DECODE` every doorbell** |
| **issue a host verb** | **ONE 4-byte store**, `:9160`. **Zero host RM ioctls** (`backmap=0`, `:1895`) | `DOORBELL-VERB → ring_doorbell` |
| **detect completion** | **ABSENT** — the C *wrote* it, `:5726` | observed |

⇒ **The C skips exactly two of the seven steps** — publication and completion detection — and
degenerates a third (the host verb) to a single store. Everything else it did *more* of.

### 3.1 ⚠ WHICH ARMS WERE ON IN THE MEASURED BOOT

Every heavy pass in our shell is env-gated and **defaults OFF**
(`shim.rs:12796 VasPublishArm::Off`, `:12608 OperandJoinArm::Off`, `:12871 FbJoinArm::Off`,
`:12808 GuestRingArm::Off`). Read out of the w311 log itself, the boot that produced the
~100 ms carried: **`VAS_PUBLISH=drain`** (7 777 `arm=drain`), **`OPERAND_JOIN=assert`** (878),
**`FB_JOIN=shared`** (67), **`GUEST_RING=ring`** (66), **`host_isolates=true`** (real isolate
children, not the loopback stub at `loopback.rs:380`).
⇒ **The floor is a floor OF THAT CONFIGURATION.** ⊘ It is not established that any of these
rows survives with the arms off, and nothing here says which arms are load-bearing for
correctness. w315 is the rung that can separate them.

### 3.2 The ranked rows

| # | difference | the C did | we do | plausibly ~100 ms? | conf. |
|---|---|---|---|---|---|
| **1** | ★★★★★ **A per-doorbell page-table decode that binds nothing.** Log, steady state: `PT-DECODE drained=3234 latched=52 requeued=3182 rounds=1 → bound=17 … refusals=1602`, and **`bound=0` in 864 of 877 doorbells (98.5 %)**. Source: `shim.rs:4707-4715` runs **four** passes per doorbell — `witness_executor_fb_pages` (`:8271`), `decode_cpu_pt_writes` (`:8304`, up to `PT_DECODE_ROUNDS = 8`, `:4498`), `sweep_cpu_pt_tables` (`:8467`, a whole-VAS walk from the installed root per live proc), `vas_census` (`:8624`) | **Dirty-gated.** `nvkvm_gpu_emul.c:8994`: `want` requires `m2_gr_vas_dirty` (a guest write to a *tracked* PT page), a new VAS, a truncated walk, or a 1-in-256 net. Measured: sweeps **11 487 → 41**, walks **91 960 → 392**, **ZERO during generation** (`:1689`, `:1737`) | ~3 200 items drained and **3 182 requeued** per doorbell; the requeue means the same items are re-drained on the *next* doorbell too | ★★★★★ **YES — the strongest row by a distance.** The C measured this EXACT shape and it cost it 200×: *"of 91960 GR-VAS walks, **91932 (99.97 %) backed NOTHING** — pure waste"*, and the gate alone took it **0.1 → 20.1 tok/s** (`:1679-1690`). **We are at 0.04 tok/s. The C's pre-gate number was 0.1.** | **high** — measured on both sides, and the C's own before/after quantifies the lever |
| **2** | ★★★★★ **The two biggest O(n) passes are DIAGNOSTIC PROJECTIONS built on the vCPU.** `RING-PROJ … (projection: ce_channel_facts) … scan=1024/1024 declared (COMPLETE: every declared entry was read)` — **877 of 877 doorbells**, sourced at `shim.rs:4643-4662` (`ce_channel_facts`, `device.rs:2013`, plus `addressing_probe_facts` = a live page-table descent per doorbell, `shim.rs:8656`). And `vas_census` (`shim.rs:8624-8652`) iterates **the whole address table of every VAS of every live proc** (`device.rs:3179`, `:3263`) purely to build the `PT-DECODE` string; only the *printed* output is capped (`PT_SWEEP_RANGE_CAP = 48`, `:13035`), **the scan is not**. Log: `VAS-BIND-CENSUS … rows=18326`. Each line is then an unconditional `format!` + `eprintln!` — a `write(2)` — on the vCPU with the BQL held | The executor itself is bounded: `MAX_ENTRIES_PER_DOORBELL = 8` (`ceutils.rs:74`). The 1024 is the *projection*, not the work | **the C read `[gp_get, gp_put)` — typically ONE entry** (`nvkvm_gpu_emul.c:9071`), had **no census**, no publication ledger, no refusal vocabulary, and gated its own per-doorbell DIAG log behind `m2trace` (360 k → 24 k lines/run) | ★★★★ **YES, plausibly most of the remainder.** 1 024 uncached ring reads at the C's own measured **1 200–3 500 ns** (`:1920`) ≈ **1.2–3.6 ms**; the 18 326-row census is the same order again *per doorbell*; and the C's logging A/B was **"INCONCLUSIVE not null"** (`:1970`) even at ~94 short lines/doorbell — ours are multi-kilobyte lines with table enumerations inside | **high** that the work is unconditional and O(table); **medium** on the cost, which is extrapolated from the C's box, not measured on ours |
| **3** | ★★★★ **The host submit crosses a PROCESS BOUNDARY, synchronously, under three held locks.** `SharedDoorbell::ring` (`shim.rs:4553`) → `SharedDevice::doorbell` (`device.rs:2359`) → `ProxyRmBackend::call` (`isolate.rs:360-376`): `write_frame` then a **blocking `read_frame`** on a `UnixStream` to the isolate child, which does the store (`rm.rs:1602-1605`). Held across it: the **BQL** (`nvkvm.c:481-488`), the `RegPlane::doorbell` `RwLock` read guard (`plane.rs:3445-3448`), and the `ce.vmm` `Mutex` (`shim.rs:4849`). The pool gate can **park the vCPU** outright (`device.rs:1865`, `:523-544`) | ★ **one 4-byte store, in-process**: `stl_le_p(s->m2_usermode_qva + 0x90, tok)` (`nvkvm_gpu_emul.c:9160`) into QEMU's own mmap of the host `AMPERE_USERMODE_A` page. **Zero host RM ioctls in steady state** — by construction and by measurement (`backmap = 0 ms / 0 calls`, `:1895`) | one blocking IPC round trip per doorbell (0 ioctls for the *submit verb* itself, `isolate/lib.rs:2961`) | **UNRESOLVED, and it is the row w315 should time first.** A socketpair round trip is normally tens of µs — but this one is taken under the BQL and can park on the pool gate, so its *tail* is not bounded by its median. ⊘ Note the isolate boundary is `isolate_exists_for_VA_IDENTITY_not_security.md`'s design, not accidental | **low** — the structure is certain, the cost is not measured anywhere |
| **4** | ★★★ **Publication / pinning per fresh row costs 3 IPC round trips.** `pin_guest_ram` → `map_guest_ram` (mmap) + `describe_guest_ram` (**one `NV_ESC_RM_ALLOC` OS_DESCRIPTOR**) + `map_gpu_va` (**two `NV_ESC_RM_MAP_MEMORY_DMA`**) — `isolate/lib.rs:2789-2830`, `rm.rs:5198/5237/4267-4284`. Our own log times it: `per_row=225 us/row`, `degrade[first_q=267us last_q=206us]`, individual rows `fresh 837us / 462us / 408us / 352us` | nothing analogous | ~225–500 µs per fresh 4 KiB row | ⊘ **NO, measured out of the per-launch path.** `DRAIN_MS` over all 877 doorbells: **median 0, p90 0**, Σ 7 191 ms — of which **5 864 ms is TWO first-doorbell drains**. w311 §5 said it; the raw log confirms it row by row. ★ It is a **startup** cost | **high** |
| **5** | ★★ **Completion is OBSERVED on a 250 ms tick, not written.** `OBSERVER_TICK_MS = 250` (`shim.rs:3621`), `reactor.run_with(PollTimeout::Millis(OBSERVER_TICK_MS), 1)` (`:3655`), `OBSERVE_DEADLINE = 20 s` (`completion_watch.rs:72`). ★★★ **This names the constant behind w311 §6's 251 ms cadence** — so w311's identification of it as *the observer's clock* is now confirmed from source, not inferred | **wrote it.** `nvkvm_phys_wr32(…, payload)` at `nvkvm_gpu_emul.c:5726`, at pushbuffer-parse time, inside the trap, on a plane where `hostonly` is provably always false (§0.2). Guest's spin was over before the trap returned. Measured `event ≈ 0 ms`, **H2 explicitly REFUTED** (`:1853`) | 250 ms sweep, eventfd-armed for newly declared watches (`shim.rs:10285-10287`) | ⊘ **NO — and w311 already killed the tempting version.** The guest's N=512 latencies form a continuous **102.9–138.1 ms** band, not multiples of 251. ⚠ But the eventfd arm means a *newly declared* watch is looked at promptly, so the 250 ms is not obviously the guest's wait either. **Do not re-run this inference; it has been made and refuted once** | **medium** — the constant is certain, its irrelevance rests on w311's distribution argument |
| **6** | ★★ **The host-CE copy polls with a 1 ms sleep, inside an IPC round trip the vCPU is blocked on.** `std::thread::sleep(Duration::from_millis(1))` in `await_semaphore` (`rm.rs:6685`, *"Polling, not waiting on an interrupt"*), `CE_COPY_TIMEOUT = 2 s` (`rm.rs:955`), reached synchronously from `ce_copy` (`rm.rs:4832-4845`) | CPU `memcpy` inline; nothing to wait for | 1 ms poll granularity per host CE copy | ⊘ **NOT in the launch window.** Copies are outside the timed region on both w311 arms. It bears on the **9.25 MiB/s copy plane**, not the ~100 ms launch floor | **high** on the mechanism; it simply is not this question |
| **7** | ★ **`GP_PUT` is never read.** By design: *"this port does not know where this channel's USERD lives"* (`ceutils.rs:89-97`); progress comes from an adapter-side `GpCursor` (`:110`) and a zero-entry scan (`fwd/lib.rs:5200 gpfifo_live_entries`) | read it, one `nvkvm_fb_read(userd + 0x8C)` per channel per doorbell (`:8973`, `:9060`) | cursor + scan | ⊘ **NO — and note the direction: we do LESS here, not more.** Listed because it is a genuine structural divergence and because the zero-entry scan is what makes the 1024 read (row 2) look necessary | **high** |
| **8** | ⊘ **The data plane.** | CPU `memcpy`, page-batched — **≥16 MiB/s** | **9.25 MiB/s** (w311 §3) | **NO.** Same order of magnitude, both ~3 orders below native, and outside the timed launch window on both arms. **The C did NOT beat us here** | **high** |
| **9** | ⊘ **No timing instrumentation on our doorbell path at all.** No histogram, no span, no per-doorbell duration counter — only **counters** (`plane.rs:3442-3473`), **budgets reported when exceeded** (`ENGINE_FWD_DRAIN_BUDGET` 1 s `:2973`, `VAS_DRAIN_WALL_BUDGET` 3 s `:13015`, `VAS_PUBLISH_WALL_BUDGET` 2 s `:13033`), and one per-op timer around `pin_guest_ram` (`shim.rs:7951-7958`) | ★ **had exactly this**: `NVKVM-TWIN` (`:1104`) printing `db=…/…us_per`, `db[cpu=…]`, `sweep`, `resolve`, `fbrd`, `overlay_iters`, `vaseen`, `backmap`, `event`, `INSIDE_qemu%` / `OUTSIDE%`, every 400 doorbells (`:4371`) | — | **NOT A COST — A CAUSE OF NOT KNOWING.** ★★★ The C could answer *"where does a doorbell go?"* in one boot. We currently cannot, which is exactly why w311 §6 had to *fit* the cost instead of reading it | **high** |

---

## 4. ★ WHAT THE C'S CHEAPNESS COST IT — and this is a legitimate answer

The brief asked whether the C's speed depended on shortcuts we have deliberately refused. **In
part, yes — and naming which parts is more useful than the ranking above.**

Three C shortcuts, each with its price:

1. **The completion was a CPU store, not an observation.** §0.2 / §1.3. The C wrote the
   payload the *pushbuffer declared*, at parse time, before any engine could have run. That is
   **zero completion latency by construction** — and it is exactly the property `CLAUDE.md`
   names when it says the C is *"no oracle at all for … hardware completion."* Any honest
   completion plane must **observe**, and observation has a latency floor a forge does not.
   ⊘ **But note the size of the prize being defended**: the C's measured completion-delivery
   cost was ~0 ms, and w311's §5 shows publication is not in our per-launch path either. So
   *"we are slower because we are honest about completion"* explains **at most the difference
   between 0 ms and a real detection latency** — it does not automatically explain 100 ms. It
   only explains 100 ms if our detection mechanism is a **poll with a ~100 ms period**. It is
   a poll with a **250 ms** period (`OBSERVER_TICK_MS = 250`, `shim.rs:3621`) — and w311 §6
   **refuted** that as the mechanism from the guest's own latency distribution.

2. **No fault plane at all.** §1. The C mirrored the guest's page tables wholesale and
   committed the mirror before anything ran, so faults were *impossible as a class* rather
   than *handled*. It therefore paid nothing for fault-safety on the hot path. Our publication
   machinery is the honest replacement — but w311 §5 measured it **out of the per-launch
   path** (877 drains / 7 191 ms across ~90 launches, 5 864 ms of it in two first-doorbell
   drains). ⇒ **This shortcut is NOT a defence of the 100 ms.**

3. **The data plane was CPU memcpy.** The C moved 145 MB per LLM run through
   `nvkvm_chan_execute`'s software CE (`:1706`). ★ **And it was not fast**: 469 MB of weights
   inside 28.6 s of load-dominated elapsed ⇒ **≥16 MiB/s** (a lower bound; load includes more
   than the copy), against our measured **9.25 MiB/s** (w311 §3). **Same order of magnitude**,
   and both are ~3 orders below native. ⇒ The copy plane is **not** where the C beat us. It
   beat us on the *launch* path, and only there.

⊘ **And a fourth thing, which is NOT a shortcut and should not be defended as one.** The two
largest O(n) passes on our doorbell (row 2) are **diagnostic projections** — `ce_channel_facts`
scanning 1024/1024 ring entries and `vas_census` iterating every row of every VAS of every live
proc, both to build a log line. That is not the price of honesty about faults or completions.
It is instrumentation on the hot path, and the C hit the identical class and **gated it**
(`m2trace`, 360 k → 24 k lines/run, `:1985`). ⚠ Note it did so *without* a measured win — its
logging A/B was **inconclusive, not null** (`:1970`) — so this row is a *candidate*, not a
verdict, and w315 is what settles it.

⇒ ★★★★★ **THE ANSWER TO THE BRIEF'S CENTRAL QUESTION.** The ~100 ms per-submit floor is
**not intrinsic to doorbell passthrough.** The C is a live counterexample: a per-doorbell cost
of ~0.94 ms wall / ~188 µs CPU, *while doing a full GPFIFO walk, a method decode, an address
translation and a CPU memcpy of the payload* — i.e. while doing **strictly more real
per-doorbell work than the brief's list of what we do**, minus exactly three things: **an
unconditional O(table) pass, a cross-process round trip, and a completion it had to wait for.**

★ Ranked by what the evidence actually supports, the honest ordering is:
**(1) the unconditional O(n) work — a decode that requeues 98.5 % of what it drains and two
diagnostic projections that scan whole tables — is by far the best-supported candidate, and the
C's own 200× before/after on exactly this shape is the strongest single piece of evidence in
this document. (2) The isolate round trip under three held locks is structurally certain and
completely unmeasured. (3) Honest completion detection is the WEAKEST of the three** — it is
where the C is no oracle at all, but it is also where w311 already looked and found nothing.

---

## 5. ⊘ UNRESOLVED — stated, not inferred

- **Launches per doorbell.** The C measured ~25 *doorbells*/token; a small Qwen2 issues far more
  than 25 *launches*/token, so libcuda coalesced. w311 counts ~3 *doorbells*/launch on our side.
  **I could not determine the C's launches-per-doorbell from source** — it is a libcuda batching
  property, not a C property. If the ratio is ~10:1 for the C and 1:3 for us, that is a ~30×
  amplification on top of the ~35× per-doorbell gap, and the two together would over-explain the
  measurement. ⚠ **Do not multiply them without measuring the C's ratio.**
- **Which semaphore writer won in `cap3`** (§0.2). Both writers were live; source cannot order
  them.
- **Whether `vh` (our bench) is nested.** The C's entire 22 → 49.9 t/s delta was attributed to
  nested-virt vmexit tax (`MILESTONES.md:13`), and `m582-m584` measured ~318 k `mmio_exits`/run
  that **no memslot trick could remove under nesting** (`mode2_bar0_trap_reduction.md:95-101`).
  If our bench is nested we inherit that tax; if it is not, w315's floor is cleaner than the C's
  ever was. **I did not establish this and it should not be assumed either way.**
- **No C per-launch timing for a matmul.** The C's numbers are all llama.cpp generation. There is
  no `cup8` per-launch figure anywhere in the tree, so the C↔w311 comparison is
  *per-doorbell to per-doorbell*, never *per-launch to per-launch on the same workload*.
- **The cost of the isolate round trip** (row 3). Structure is certain — `write_frame` +
  blocking `read_frame` on a `UnixStream` (`isolate.rs:360-376`), under BQL + the doorbell
  `RwLock` + `ce.vmm`, with a pool gate that can park the vCPU (`device.rs:1865`). **Its
  latency is measured nowhere.**
- **Steady-state ioctls per doorbell.** The submit verb itself is **0** ioctls
  (`isolate/lib.rs:2961`); everything else depends on how many table rows are still unpinned at
  that doorbell, which is runtime state and not derivable from source.
- **Whether `vas_census` is the cost or just reports it.** It calls `vas_table_ranges` /
  `vas_published_ranges`, both of which iterate the whole table (`device.rs:3179`, `:3263`), so
  it *is* O(n) — but I did not establish what one row's iteration costs.
- **Which arms w315 will run with** (§3.1). Our floor is a floor of `VAS_PUBLISH=drain` +
  `OPERAND_JOIN=assert` + `FB_JOIN=shared` + `GUEST_RING=ring` + real isolates. A different arm
  set is a different measurement.

---

## 6. ⊘ WHAT THIS RUNG CANNOT DO, AND WHAT WOULD SETTLE IT

This is attribution **by comparison**. It ranks candidates; it measures none of them on our
plane. ★★★ **The C's decisive advantage was not architectural — it was `NVKVM-TWIN`**
(`nvkvm_gpu_emul.c:1104`): a per-window dump that split one doorbell into `sweep` / `resolve` /
`fbrd` / `vaseen` / `backmap` / `event` and printed `us_per` doorbell and `INSIDE_qemu%` vs
`OUTSIDE%`. Every one of the C's perf findings — and every one of its **refutations**
(clocksource, logging, completion latency, host-ioctl, the O(n) overlay scan) — came out of
that one instrument. We have **counters and exceeded-budget warnings, and no per-doorbell
duration at all** (row 9).

⇒ If w315 disagrees with this document, believe w315: it is measuring, and this is reading.

