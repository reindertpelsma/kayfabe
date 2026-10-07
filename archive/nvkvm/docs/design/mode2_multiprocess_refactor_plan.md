# Mode-2 multi-process refactor — implementation plan

**Status:** design / ready-to-implement (2026-07-19; **rev 2 same day** — the index decision is
corrected to **PDB-primary / CR3-minimal-or-none** after the PDB-vs-CR3 security review with the
owner; the earlier "two-key, CR3 = the security key" framing was wrong, see §1). Branch
`consolidation`.
**Deliverable of:** the "two+ concurrent guest CUDA processes work on Mode-2" task.
**Baseline:** `v3` (commit `862c7c2`) — single-process byte-identical, one-process-reliably-concurrent.
**Companions (read these; this plan applies them, it does not re-derive them):**
`docs/design/mode2_multiprocess_isolate.md` (the #14 synthesis — the wall; its *two-key security*
conclusion is **superseded** by §1 here), `docs/design/mode2_address_table.md` (§13 = "VAS identity is PDB, never the client
handle"; §3/§6 the one forward-populated table, MISS=FAULT), `docs/design/mode2_forwarding_model.md`
(emulate-kernel / passthrough-userspace, unprivileged-host-ops only),
`docs/design/mode2_isolate_consolidation_plan.md` (the per-client→per-isolate spine),
`docs/design/mode2_dataplane_architecture.md`, `docs/design/mode2_memory_model.md`; memories
`mode2_14_concurrent_apps`, `mode2_isolation_cr3_key`, `access_model_split`,
`isolate_architecture`, `multiproc_collision_blocker`, `mode2_gpu_emul_refactor_debt`.

All line numbers below are `src/qemu/nvkvm_gpu_emul.c` at HEAD unless prefixed with another file.
Where a citation may have drifted it is marked "ASSUMPTION — verify."

---

## 0. TL;DR

The emulator holds ~30 keyed tables + ~9 scalar singletons + one host isolate inside a single
per-device struct `NvkvmGpuEmul` (`:127`). Today they are keyed by the **guest RM client handle**
(which two processes reuse with *identical values*) or are outright singletons. The refactor
re-keys them on a **process identity whose primary — and expected only — key is the PDB**:

- **PDB** (page-directory-base = "the GPU's CR3", `mode2_address_table.md` §13) — the **primary
  key for BOTH the data plane AND the isolate grouping**. Client-independent, already distinct per
  process (`0x3401000` vs `0x3405000`), always present GPU-side (no `cpu_synchronize_state`), and
  it is the *destination FB address* of every CE page-table write, so which process a PT-write
  belongs to is knowable **without any CPU signal**. This is the key for: mappings, backing,
  page-table capture, VAS selection, sema resolution, **and** grouping a process's VASes/clients
  into its per-process container + host isolate.
- **vCPU CR3** is demoted to a **minimal, optional disambiguator** — defense-in-depth at exactly
  one *reliable* spot (the **userspace doorbell** trap, where the vCPU is running libcuda so CR3
  really is the process's page-table root), and used **only if** the doorbell token/CHID proves
  ambiguous — experiment **E0**, §1.4. CR3 is **not a security requirement**: the isolate's
  security comes from it being *unprivileged* (+ QEMU's host/cross-VM boundary), not from the key
  choice, and intra-guest isolation is the guest kernel's job (`access_model_split`, §1.2). At
  kernel-context RPCs CR3 is unreliable under PTI anyway (§1.3) — the earlier "stamp CR3 at the
  alloc/VAS-create RPCs" step is **dropped**.

The map is: **PDB carries the address plane AND the exec/grouping plane; CR3 is at most a
doorbell tiebreak.** A per-process container `NvkvmProc { pdb-set; isolate; per-VAS sub-tables;
optional cr3 }` is looked up by **doorbell token → CHID → channel → owning PDB** on the hot
doorbell trap and by PDB (via the v3 dup-edge chain) on the data plane; the GSP falcon, RPC ring,
BAR page-dirs, interrupt tree, and kernel/scrubber channels stay **device-global** (the "system
isolate").

**Phase count: 7** (P0…P6), each individually shippable and single-process-byte-identical, ending
in N× concurrent `cup8` all `rc=0`. **Riskiest item: P4** — making each process's CE page-table
writes *execute and be captured under its own PDB* (the round-5/6 wall: the loser's `PD0[1]` leaf
is never written into our FB shadow because the push carrying it starves). Everything else is
mechanical re-keying that v3 partly did.

---

## 1. The index decision

**Corrected 2026-07-19 (PDB-vs-CR3 security review with the owner): PDB / GR-VAS is the PRIMARY
key for both the data plane and the isolate grouping; CR3 is minimal-or-none — a defense-in-depth
disambiguator used at MOST at the userspace doorbell, and only if experiment E0 (§1.4) proves it
necessary.** The earlier framing ("two keys; CR3 carries the security plane because PDB is
guest-controlled ⇒ spoofable") — and `mode2_multiprocess_isolate.md`'s two-key conclusion — is
superseded: the spoofing argument was wrong (§1.2).

### 1.1 Why PDB is the primary key — data plane

The owner's instinct — index per **"GR VA-space / segment"** — is *exactly* the PDB key, and it is
correct for the **data plane**. `mode2_address_table.md` §13 nails the precise form: a channel does
not name its VAS directly (`kernel_channel.c:1030`: `hVASpace` comes from the ctxshare/TSG, and for
`hVASpace==0` GSP-managed channels it is the device-default VAS); the authoritative identity is the
**PDB** = the page-directory-base physical address stored in the instance block at `RAMIN+0x200` =
"the GPU's CR3", which is **client-independent**. Two processes get distinct PDBs
(`0x3401000`/`0x3405000`), so per-PDB keying makes "same VA in two processes" a *disambiguation*,
not a collision (§3/§13, line ~66/255/287).

What remains of the round-5/6 wall (`mode2_14_concurrent_apps` rounds 4–6,
`mode2_multiprocess_isolate.md` §"remaining wall") under pure PDB-keying:

- The two processes' `hVASpace=0` GR channels reach us with **empty instance blocks** (GSP-managed),
  so the emulator cannot read the channel's PDB from a handle. It falls back to a *content-pick*:
  scan `chan_vas[]`, take the first VAS whose walk of the pending GPFIFO entry reads non-zero
  (`nvkvm_chan_execute` two-pass probe, `:5265`). With identical guest VAs, "first non-zero" resolves
  process B's channel under process A's PDB → FAULT → B never completes.
- v3's dup-edge chain (GR channel → `chan_client` → `m2_dup` → UVM VAS → PDB) recovers the owning
  **client** and thereby the PDB *for channel-registration and backing*, and it works: it eliminates
  the cross-process pushbuffer FAULT (round 2). The page-table *publication* — the CE writing the
  loser's `PD0[1]` leaf into FB — is attributed by its **destination FB address' owning PDB**
  (§4.1-4a), also with no CPU signal.

Round 5 believed the only signal distinguishing two identical-VA submissions at exec time was the
**vCPU CR3** of the submitter. **Corrected:** the doorbell already carries a GPU-side identity —
the work-submit **token encodes the guest vChid/CHID** (`:3386-3396`), and CHIDs (unlike the
reused RM handles) are fresh per channel-create. If E0 (§1.4) confirms per-process distinctness,
the doorbell → CHID → channel → owning-PDB chain resolves the submitter **with no CR3 at all**.

### 1.2 Why PDB is ALSO the isolate-grouping key — the security correction

The old plan treated CR3 as *necessary* for the security boundary ("PDB is guest-controlled ⇒
spoofable ⇒ the isolate must key on CR3"). That reasoning is wrong, on four counts:

1. **An isolate's security comes from it being UNPRIVILEGED, not from its key.** Whatever an
   isolate is keyed on — PDB, CR3, or client handle — it can only issue *unprivileged* host GPU
   ops (`mode2_forwarding_model.md`; QEMU owns the host/cross-VM boundary). The host boundary is
   therefore preserved under PDB-keying exactly as under CR3-keying.
2. **The threat model** (`access_model_split`, `mode2_isolation_cr3_key` — the owner's model, and
   it is the standard virtualization one): the **guest kernel is the authority for intra-guest
   (process-to-process) isolation** — it enforces ALL intra-VM access rights; QEMU + the
   unprivileged isolates enforce ONLY the host/cross-VM boundary. By level: a compromised guest
   *userspace* process → owns its own process and at most (chained) its own isolate; a compromised
   single *isolate* → that process's userspace/contexts, no host reach beyond unprivileged ops; a
   compromised guest *kernel* → all guest userspace (which it already owned), but strictly NO
   escape to host/QEMU/cross-VM.
3. **PDB is set up by the guest KERNEL RM, not by userspace.** Every PDB the emulator sees was
   created and bound by guest kernel RM (SET_PAGE_DIRECTORY / channel-alloc RPCs, instance-block
   writes); the guest kernel blocks userspace from forging those. So a compromised guest
   *userspace* process CANNOT fabricate/spoof another process's PDB — the old plan's spoofing
   claim fails at the userspace level.
4. **PDB-keying survives a compromised guest KERNEL, too.** A compromised kernel *can* forge PDBs
   and reshuffle which isolate its traffic routes to — but every isolate is unprivileged
   (point 1), so it gains nothing host-side; and it already owns all guest processes (point 2), so
   there is no intra-guest escalation either. The "no host escape" invariant holds unchanged.

Two further observations settle the *grouping* granularity:

5. **Sharing a GR VAS ⟹ sharing GPU memory ⟹ mutual trust already implied.** Two guest processes
   sharing a VA space do not isolate from each other on the GPU no matter what we do; mapping them
   to one isolate loses nothing. And forcing *untrusted* processes to share a VAS requires
   guest-kernel cooperation — a kernel which, if compromised, already owns both (point 4). So
   VAS-keyed isolates create **no new cross-process leak**: "one isolate shared by all processes
   on the same GR VAS" is sound.
6. **A process may hold several GR VA spaces**, so strictly per-VAS isolates would OVER-isolate —
   never *less* secure, but they would split one process's VASes across isolates and complicate
   that process's shared host state (guest-RAM share, handle namespace, `m2_cmap`).
   Per-**process** grouping (one isolate per PDB-*set*) is therefore preferred — but note this is
   a **correctness/simplicity** argument, NOT a security one. If per-process grouping ever proves
   hard, falling back to per-VAS isolates is safe.

**Consequence:** the isolate key = the process's PDB-set (§3), associated via the doorbell/CHID
demux + the v3 dup-edge chain. CR3 is not needed for security anywhere.

### 1.3 Precise resolution rules

**Data-plane op (mapping / backing / sema resolve / PT capture) → PDB.** Resolve the owning PDB via
the v3 chain, in order (mirrors `mode2_address_table.md` §13 resolution order):

1. channel `hVASpace` if non-null → its VAS → PDB;
2. else channel's ctxshare/TSG VAS → PDB (via `(client,tsg)` → `m2_cvas`/`chan_vas`);
3. else device-default VAS PDB;
4. **client attribution for `hVASpace=0`:** GR channel → `chan_client` → `m2_dup` dup-edge →
   owning user compute client → its UVM VAS → PDB (`nvkvm_chan_own_pdb_rs` `:4720`,
   `nvkvm_m2_pdb_gr_owner` `:4700`).

A miss is a **fault**, never a blind content-pick (§6). (The blind pass-1 in `chan_execute` `:5271`
becomes dead once every VAS is PDB-attributed; delete it — see P5.)

**Exec op (doorbell ring / submission MMIO) → doorbell token (CHID), not CR3.** The guest writes
its work-submit token at the doorbell trap (`nvkvm_bar0_write`, `off==NVKVM_VF_DOORBELL`, `:3386`);
on Ampere the token encodes the guest vChid+runlist (NVC36F `GET_WORK_SUBMIT_TOKEN`, `:3388`).
Today the emulator dedup-logs the token and **ignores** it, walking every channel (`:240`,
`:3389-3391`). The demux becomes: token → CHID → `chans[]` entry → owning PDB/proc (rules above).
No `cpu_synchronize_state`, no CPU-state read at all.

**Kernel-context RPCs (SET_PAGE_DIRECTORY / RESERVED_PDES / channel-alloc, populate sites
`:2310/:2363/:2477`) → the handle/dup-edge graph, not CR3.** The old plan stamped CR3 here — its
shakiest step, now **dropped**: these RPCs execute in guest **kernel** context, where under PTI the
live CR3 is the *kernel* CR3 and does not cleanly identify the userspace process. Attribution at
these sites already works via the client/dup-edge graph → PDB (v3; the round-2 cross-process FAULT
was eliminated this way). Forwarded alloc/control RPC attribution likewise: `hClient` → dup-graph
→ proc (`mode2_isolate_consolidation_plan` note), no CR3.

### 1.4 The residual CR3 surface + experiment E0 (the round-5/6 open question, resolved)

| Situation | Key | Rationale |
|---|---|---|
| Record a VA→phys binding at bind time | **PDB** | §4/§13; the CE-write destination is already per-PDB |
| Resolve a channel's pushbuffer/sema at exec | **PDB** (via dup-chain) | already works post-v3 (round-2 FAULT eliminated) |
| Capture a CE PT-write into the FB shadow | **PDB** (from the write's destination FB address) | §4.1-4a; destination alone attributes it |
| Choose which process's channel a doorbell rings | **doorbell token → CHID → channel → PDB** | E0; GPU-side identity, no CPU signal |
| Schedule the PT-writer channel (starvation, §4.1-4c) | **per-(PDB, channel)** ring/TSG scheduling | channel identity suffices once the doorbell demuxes by CHID |
| Pick the host isolate | **the process's PDB-set** | §1.2 — grouping choice, not a security key |
| Doorbell demux **iff CHID proves ambiguous** | **CR3 at the userspace doorbell** (only) | the one reliable CR3 spot: the vCPU is running libcuda → CR3 = the process's page-table root |

**Experiment E0 — THE deciding experiment (run first; folds into P1): is the doorbell token/CHID
distinct per process's channel?** Round 1 saw the two processes reuse identical guest RM *handles*
(`0x5c000019`), which killed handle-keying. But CHIDs are not handles: each channel-create yields a
fresh channel (the `chans[]` registry grows per-create, `:2484`; the guest token encodes vChid,
`:3388`), so the doorbell → CHID → channel → owning-PDB chain (via the §1.3 rules) is **likely
unambiguous with no CR3 at all**. E0 = log `(token, resolved channel, owning PDB)` per doorbell
across 2× `cup8` (the `:3392-3396` dedup-log already captures the token) and check for collisions.

- **If distinct (expected):** the ENTIRE refactor keys on PDB/VAS; CR3 is dropped even for
  correctness — `nvkvm_cpukey.c` is never built.
- **If ambiguous:** add the CR3 read at the **userspace doorbell only** — the single place it
  earns its keep — as the tiebreak.

This resolves the round-5/6 open question, superseding the plan's previous CR3-heavy answer
(which reached for CR3 at the doorbell + the RPC populate sites + the PT-write hook).

**★ E0 RESULT — RUN 2026-07-19 on the rebuilt bench (host=580, HEAD 862c7c2-equiv): DISTINCT — CR3
DROPPED ENTIRELY.** Two concurrent `cup8` (`NVKVM_M2CEFWD=1 NVKVM_M2TRACE=1`), `M5.11 DOORBELL` log:
105 doorbell writes, **35 distinct tokens = 35 distinct `[11:0]` vChids — one per channel** (the token
IS the vChid: small sequential ints `4, 12, 13, …`, `token[11:0]==chid`). Since each channel-create
yields a fresh vChid and the two processes' channels are distinct channels, the doorbell token uniquely
identifies the channel → (via §1.3) the owning PDB → the process, **with no CPU signal**. The current
code even names the bug it exposes — the doorbell handler comment (`:3387`): *"Today we ignore it and
ring the host GR token unconditionally — wrong for multi-channel."* So P1's doorbell work is **"demux
the distinct token we already receive,"** not "add CR3." **Consequence: `nvkvm_cpukey.c` is never built;
`cpu_synchronize_state`/`env.cr[3]` appear nowhere; the refactor keys entirely on PDB + vChid.** The CR3
caveats below are now moot (kept only as the rationale for why CR3 was never made load-bearing).

**CR3 reliability caveats (they apply only to the E0-ambiguous fallback, but they are the real
reason CR3 must not be load-bearing):**

- CR3 is reliable ONLY at the **userspace doorbell** (vCPU executing libcuda → CR3 is the process
  root). At kernel-context RPCs it is the *kernel* CR3 under PTI — never read it there (§1.3).
- CR3 values are **reused across process exit** → the `NvkvmProc` must be reaped on exit before a
  new process recycles the value (P0 reap, `ctx_free_drop`).
- Mask PCID/PTI bits: `env.cr[3] & ~0xfff` (round-5 `mode2_14_cr3.patch`).
- `cpu_synchronize_state(current_cpu)` is not free (round 5: an unbudgeted per-doorbell sync timed
  out CTX2-create) → budget it / cache per-vCPU-at-exit if the fallback is ever engaged.

---

## 2. Complete inventory of single-process-global state → new key

One instance per device: `struct NvkvmGpuEmul` (`:127`). No file-scope Mode-2 data tables exist (the
only statics are perf counters + DIAG log-budget + a `g_nvkvm_dma_s` back-pointer, all non-state).
So "make it per-process" = re-key the tables below. **"v3?"** marks what commit `862c7c2` already
converted from process-blind → client-keyed.

### 2.A Channel / VAS / PDB resolution (hot exec path)

| # | Global (file:line) | Tracks | Scope today | NEW key | Lifecycle | v3? |
|---|---|---|---|---|---|---|
| 1 | `chans[64]` / `chan_n` (`:288`, append `:2484`) | per-channel GPFIFO/USERD/token/tsg/payload | `(client,gpFifoVA)`-keyed, cap 64 | **PDB** (+ per-channel CHID/token for the doorbell demux, §1.4) | drop in `ctx_free_drop` `:1750` | key+cap (v3) |
| 2 | `chan_*` scalars (`:220-234,290,399`) | "currently executing channel" scratch | singleton scratch, loaded per iter (`:3510`) | **per-`NvkvmProc` scratch OR pass explicitly** (see §3.3) | per-iter | — |
| 3 | `chan_vas[16]` / `_n` (`:371`, append `:2310/:2363`) | snooped VAS roots `{hvas,client,pdb,root_sys,uvm}` | `client`-keyed, **cap 16 ⚠ under-sized** | **PDB**; grow cap | drop `:1915` | partial (v3) |
| 4 | `m2_cli_vas[64]` / `_n` (`:383`, add `:1711`) | **sticky** per-client VAS roots for sema resolve | `client`-keyed, never freed | **PDB** | **make reapable** at proc-exit | — |
| 5 | `va_map[1024]` / `_n` (`:302`, append `:2083`) | PROMOTE_CTX VA→phys side-table | `client`-keyed, **never reaped** | **PDB** | **add reap** at proc-exit | — |
| 6 | `m2_dup[64]` / `_n` (`:397`, append `:2401`) | DUP_OBJECT ownership edges (process attribution graph) | `(dst,src)` client/obj | **keep client-graph** (the proc-attribution edge, §1.3) | drop `:1872` | **NEW (v3)** |
| 7 | `m2_gr_clients[8]` / `_n` (`:573`, append `:6545`) | all user GR compute clients (≈1/proc) | client list | **subsumed into `NvkvmProc` set** | drop `:1887` | **NEW (v3)** |
| 8 | `m2_user_clients[8]` / `_n` (`:589`, append `:2421`) | early-arm dup-src user clients | client list | **subsumed into `NvkvmProc` set** | drop `:1895` | **NEW (v3)** |
| 9 | `m2_gr_client` scalar (`:567`, set `:6549`) | legacy FIRST GR client | singleton (proc-0 only) | **delete** (replaced by per-proc set) | never cleared | — |
| 10 | `chan_client` scalar (`:290`, set `:3515`) | executing channel's client | singleton scratch | **per-`NvkvmProc` / explicit arg** | per-iter | — |

### 2.B Isolate / host-session (control path)

| # | Global | Tracks | Scope today | NEW key | Lifecycle | v3? |
|---|---|---|---|---|---|---|
| 11 | `m2_iso` (`:424`) | the **one** host stub/isolate for the device | singleton | **per-`NvkvmProc` isolate** (+ system isolate) | never reaped | — |
| 12 | `m2_iso_id`, `m2_ctl_h`, `m2_gpu_h`, `m2_gpu_fd` (`:426-429`, `session_id=1` hardcoded `:6098`) | isolate id + fixed handles | singleton, session hardcoded 1 | **per-proc: session_id = process ordinal**; per-proc ctl/gpu fds | set once | — |
| 13 | `m2_cmap[128]` / `_n` (`:436`, append `:6140`) | guest hClient → synthetic host client | client-remap, never reaped | **per-`NvkvmProc`** | **add reap** | — |
| 14 | `m2_maph_next` (`:441`) | per-mapping fresh fd handle allocator | singleton counter | **per-`NvkvmProc`** | monotonic | — |
| 15 | `m2_databuf_next` (`:490`) | unique host handle allocator | singleton counter | **per-`NvkvmProc`** | monotonic | — |

Infra note (`nvkvm_isolate.h`): the isolate table already supports `NVKVM_ISOLATE_MAX=4096`
isolates + a `session_id` param. Mode-2 only ever creates **one** (`nvkvm_isolate_create(...,1,...)`).
The machinery for per-process isolates **already exists** — Mode-2 just never used it. This is the
`mode2_isolate_consolidation_plan.md` "one host isolate per guest RM client" spine; here we make the
grouping **per guest process — its PDB-set, associated via the doorbell/CHID demux + dup-edge
chain (§1.2 point 6, §1.4)** — rather than per hClient (finer-grained over-isolation, per hClient
or per VAS, is also safe — §1.2 points 5-6 — but per-process coalesces a process's clients,
simplifies its shared host state, and matches Mode-1).

### 2.C Device / VAS / TSG forwarding bookkeeping (control path)

| # | Global | Tracks | Scope today | NEW key | Lifecycle | v3? |
|---|---|---|---|---|---|---|
| 16 | `m2_devvas[32]` / `_n` (`:445`, append `:6237`) | `{client,dev,vas}` forwarded VASpaces | client-keyed | **per-`NvkvmProc`** (PDB via VAS) | drop `:1808` | — |
| 17 | `m2_tsgeng[64]` / `_n` (`:449`, append `:6350`) | TSG→engineType | tsg-keyed, never reaped | **per-`NvkvmProc`** (`(proc,tsg)`) | **add reap** | cap (v3) |
| 18 | `m2_subdev[64]` / `_n` (`:455`, append `:6229`) | `{client,subdev}` | client-keyed, never reaped | **per-`NvkvmProc`** | **add reap** | — |
| 19 | `m2_grmap[8]` / `_n` (`:599`, append `:7282`) | `{client,hvirt,hvas,hdev}` GR virtmem mapper | client-keyed, **cap 8 ⚠** | **per-`NvkvmProc`** | never reaped → **reap** | — |
| 20 | `m2_cvas[16]` / `_n` + `m2_cur_cvas` (`:617`, append `:7169`) | per-`(client,tsg)` fresh host VAS | `(client,tsg)`-keyed, cap 16 | **PDB / `(proc,tsg)`** | drop `:1827` | — |
| 21 | `m2_tsg_sched[16]` / `_n` (`:632`, mark `:4677`) | which `(client,tsg)` GR TSGs were scheduled | `(client,tsg)`-keyed | **`(PDB,tsg)`** | drop `:1903` | **NEW (v3)** |
| 22 | `m2_user_ce_clients[16]` / `_n` (`:607`, append `:7272`) | libcuda CE-copy clients | client list, never reaped | **per-`NvkvmProc`** | **add reap** | — |
| 23 | `m2_gr_channel`, `m2_gr_tsg` scalars (`:620`) | host GR channel/TSG handles | singleton (proc-0) | **per-`NvkvmProc`** | set once | — |
| 24 | `m2_gr_reply[64]` + meta (`:640`) | captured host `0xc7c0` alloc reply for RPC forge | singleton (last-alloc) | **per-`NvkvmProc`** (keyed by requesting proc) | overwritten | — |

### 2.D Data-plane memory backing (hot fb + control)

| # | Global | Tracks | Scope today | NEW key | Lifecycle | v3? |
|---|---|---|---|---|---|---|
| 25 | `m2_fbback[128]` / `_n` (`:468`, append `:7359/:8714`) | FB-range→host_qva double-mmaps | `fb_base`-keyed (no client) | **PDB** (per-VAS backing) | drop `:1789` | cap (v3) |
| 26 | `m2_chanbuf[96]` / `_n` (`:487`, append `:8721`) | `{client,chan,h_userd,qva}` host USERD | `(client,chan)`-keyed | **per-`NvkvmProc`** | drop `:1784` | cap (v3) |
| 27 | `m2_mapped_va[65536]` / `_n` (`:674`, append `:7503`) | `{client,va,gpa,hmem,reback}` backed-VA dedup/staleness | `(client,va)`-keyed | **`(PDB,va)`** | compute flush `:1852`; else **reap** | — |
| 28 | `m2_objs[1024]` / `_n` (`:700`, append `:7708`) | gpu_memory_object backings `{client,hMemory,…}` | has client field, indexed by GPGA, never reaped | **per-`NvkvmProc`** (client field → proc) | **add reap** | — |
| 29 | `m2_gpga[2048]` / `_n` + `m2_gpga_sorted[2048]` (`:712`) | GPGA page-range→(obj,off) + binary index | **GPGA-addr-keyed, not client-keyed** | **stays GPGA-global** (single guest-GPU-phys space) — but each GPGA range must belong to exactly one PDB's backing (§2.E note) | never reaped → **reap** | — |
| 30 | `m2_gr_pt_set[8192]` + meta (`:546`, record `:4359`) | hash-set of vidmem GR PT pages (dirty track) | device-global | **per-PDB** (P4) | reset each sweep | — |
| 31 | `m2_cpt[4096]` / `_n` + `m2_cpt_dirty[256]` (`:560`, record `:7944`) | compute-VAS PT page ownership `{page,pdb,vabase,level,dirty}` | **pdb-keyed already** | **PDB** (formalize) | reset each sweep | — |
| 32 | `bar1_wpg[64]` + meta (`:318`) | BAR1-written FB pages MRU (ring resolution) | device-global scratch | **per-`NvkvmProc`/per-channel pin** (P4 ring-pin; round-5) | ring-overwritten | — |

### 2.E System / kernel / HW-global — **STAYS device-global (the "system isolate")**

Do **not** per-process these. They are the single faked GSP + HW, or kernel/scrubber-scoped, and
per-`mode2_forwarding_model.md` are emulated (not forwarded) or forwarded through the system isolate.

- **GSP falcon + RPC ring:** `mbox0/1`, `sec_mbox0`, `fwsec_ran`, `gsp_suspended/reloaded`, `q_*`,
  `stat_seqnum`, `cmd_readptr` (`:151-195`). One faked GSP per VM; the RPC ring is one shared queue
  (its cursors are inherently global). Process attribution of records populated *from* RPCs
  (VAS/chan alloc) comes from the handle/dup-edge graph → PDB (§1.3), **never from CR3** — the RPC
  path runs in guest kernel context where CR3 is unreliable.
- **VRAM + BAR page-dirs:** `fb_pages` (GHashTable), `bar0_window`, `bar1_pdb`, `bar2_pdb`,
  `bar2_inst_block`, `bar2_virtual` (`:203-214`). HW/GPU-global.
- **Interrupt tree:** `intr_leaf[]`, `intr_leaf_en[]`, `intr_top/_en`, `gsp_swgen0_pending`
  (`:340-355`). MSI-X, HW-global. (Per-process completion *delivery* is P6, but the tree stays one.)
- **OS events:** `osevents[64]` (`:353`) — `(hclient,hevent)`-keyed, already handle-scoped; kernel
  scrubber/CeUtils/UVM channels are correctly SYSTEM-scoped and the finishPayload forge (`:3829`)
  already **excludes** every user GR/CE client (keep it forging kernel channels only).
- **Backdoor kernel sema pages:** `m2_bd_pages[128]`, `dbg_gpa_lo/hi` (`:407-417`). Kernel/UVM.
- **Guest-RAM share:** `m2_guest_ram_*`, `m2_ht`, `m2_stub_ram_base` (`=0x7e0000000000`, `:6051`),
  `m2_ram_shared` (`:681-692`). One 126 TiB MAP_FIXED of *all* guest RAM. **Caveat:** this is shared
  into the *single* stub today; per-process isolates each need the guest-RAM share (or the relevant
  slices) mapped in — see P2 risk. The share itself (whole-guest-RAM) is device-global; *which
  isolate it is exposed to* becomes per-process.

### 2.F Gating + retry helpers (v3, become unconditional / re-keyed)

- `nvkvm_m2_multiproc()` (`:4657`): returns `gr_clients_n>1 || user_clients_n>1`. Gates the
  process-separation divergences so single-proc stays byte-identical. **Post-refactor:** the
  divergences become the *only* path (keyed correctly from process start), so the gate's job shrinks
  to "is there >1 `NvkvmProc`" and eventually can be dropped where the per-proc key already
  disambiguates (P5).
- `m2_poll_kick` + `m2_last_db_token` (`:595/:565`): per-poll completion retry — a singleton a 2nd
  process's poll overwrites. **Becomes per-`NvkvmProc`** (P6).

### 2.G GPA-window / arena allocator (`virtio_nvgpu.*`, Mode-1 path — `multiproc_collision_blocker`)

Not inside `NvkvmGpuEmul`. On `struct VirtIONvgpu`: `sparse_gpa_base/size/vmm_va/cur` (bump pointer),
a `#80/H-1` free-list (`virtio_nvgpu.h:177`), `NVKVM_MMAP_WIN_SIZE=16GB`,
`NVKVM_SPARSE_GPA_SIZE=128GB`; helpers `nvkvm_sparse_gpa_alloc/free` (`:486-488`). **Per-VM, one
free-list shared by all processes** — a bump/recycle arena. `multiproc_collision_blocker` (Mode-1)
already RESOLVED the concurrent-CUDA hang here (per-vq spinlock 73b206d + async dispatch 199040e +
single-window heap), so this arena is *not* the Mode-2 #14 blocker. But the plan must (P2/risk R3)
confirm the arena does not exhaust with N per-process isolates each carving mappings, and adopt the
`multiproc_collision_blocker` "per-fd arena + MAP_FIXED + slot-recycle" follow-ups if it does.

**Counts:** ~30 keyed tables (rows 1–32 minus group E) + ~9 scalar singletons (row-2 `chan_*`;
`m2_gr_client`; `m2_gr_channel`/`m2_gr_tsg`; `m2_doorbell_ready`/`m2_usermode_qva`/`m2_gr_token`
`:644-646`; `m2_poll_kick`/`m2_last_db_token`) + **1 isolate/session singleton**. v3 already
re-keyed rows 1,3(partial),6,7,8,21,26 and bumped caps on 1,17,25,26. **Under-capacity even after
v3:** `chan_vas[16]`, `m2_grmap[8]`, `m2_cvas[16]` (fine at 2 procs, not many).

---

## 3. The data structure

### 3.1 The per-process container

```c
/* one per live guest CUDA process; PRIMARY identity = its PDB-set (the GR/UVM VASes the guest
 * kernel RM created for it), grouped via the doorbell/CHID demux + v3 dup-edge chain (§1.2-1.4).
 * cr3 exists ONLY for the E0-ambiguous doorbell-tiebreak fallback; 0 = unused. */
typedef struct NvkvmProc {
    uint32_t isolate_id;          /* its own unprivileged host isolate (session_id = ordinal) */
    uint32_t ctl_h, gpu_h; int gpu_fd;   /* per-proc ctl/gpu handles + registration state     */
    uint64_t cr3;                 /* OPTIONAL doorbell tiebreak (env.cr[3] & ~0xfff), §1.4;
                                   * never load-bearing for security (§1.2); 0 = unused       */

    /* address plane: PDBs owned by this process (a process may hold several VASes) */
    struct NvkvmVas {
        uint64_t pdb;             /* the VAS key (RAMIN+0x200); "the GPU's CR3"                */
        /* per-VAS forward-populated VA->binding table (address_table.md §3):
         *   sorted VA-range -> { gpga_base, aperture, size }  + one RW-lock              */
        struct VaBinding *bindings; int n_bindings;
        /* per-VAS captured PT pages (rows 30/31) + backing (row 25) live keyed here      */
    } vas[NVKVM_PROC_MAX_VAS];
    int n_vas;

    /* control-plane bookkeeping formerly client-keyed (rows 13-24,26,28): */
    /* m2_cmap slice, m2_devvas slice, m2_grmap slice, m2_chanbuf slice, m2_objs slice,
     * m2_tsgeng slice, m2_subdev slice, m2_user_ce_clients slice, m2_cvas slice,
     * m2_tsg_sched slice, m2_gr_reply, m2_gr_channel/tsg, m2_maph_next, m2_databuf_next */

    /* exec/completion (rows 2 scratch, row 32 pin, row G-poll): */
    uint64_t last_db_token; bool poll_kick;
    /* per-channel resolved ring-page pin (round-5): chans[i].gpfifo_phys/bar1off */

    bool live;                    /* reaped on process-exit (last VAS/client freed)          */
} NvkvmProc;
```

Inside `NvkvmGpuEmul`, replace the ~30 flat tables with:

```c
NvkvmProc  m2_proc[NVKVM_MAX_PROCS];   /* small: bench = 2-4; cap ~16 */
int        m2_proc_n;
NvkvmProc *m2_sys;                     /* the SYSTEM isolate: GSP/scrubber/kernel traffic  */
```

The **system pseudo-process** (`m2_sys`) owns all group-E kernel/GSP/scrubber state and the system
isolate; kernel/GSP/scrubber traffic routes here by **traffic class** (kernel RM clients, scrubber/
CeUtils channels — the same line the v3 finishPayload exclusion `:3837` already draws), not by CR3
(the `mode2_isolation_cr3_key` "kernel threads → system isolate" outcome, reached without reading
CR3).

### 3.2 Lookup on the hot path (must be cheap)

- **By doorbell token / CHID (doorbell / submission trap):** decode the vChid from the written
  token (`:3388`, `:3394` logs the candidate fields), match it to the `chans[]` entry, follow the
  channel's PDB attribution (§1.3) to `(NvkvmProc*, NvkvmVas*)`. A linear scan of `chans[]`
  (n≤64, L1-resident) or a direct CHID index; cache the last-hit — bursts of doorbells come from
  one channel. **No `cpu_synchronize_state` on the hot path at all** — the old plan's per-doorbell
  CR3-read cost (round 5: an unbudgeted sync timed out CTX2-create) disappears in the expected E0
  outcome. Only in the E0-ambiguous fallback does `nvkvm_current_guest_cr3()` enter, with the §1.4
  mitigations (budget the sync / cache CR3 per-vCPU-at-last-exit / read once per burst; ASSUMPTION
  — verify a cached-at-exit CR3 is valid at the doorbell trap). ASSUMPTION — verify the token's
  vChid field decode against the GA100 HAL (E0 does this).
- **By PDB (data plane):** the channel already resolves to a client via v3's dup-chain; extend that
  to yield `(NvkvmProc*, NvkvmVas*)` by PDB. `chan_vas[]`/`m2_cpt[]` are already PDB/client-tagged, so
  this is a field lookup, not new walking. Per-VAS binding lookup is the existing GPGA binary-search
  index (`m2_gpga_sorted`) sharded per VAS.

### 3.3 The `chan_*` scratch (row 2) — the one structural wart

Today `nvkvm_chan_execute` reads a **global** `chan_*` scratch loaded from `chans[i]` each iteration
(`:3510-3516`). With concurrent processes this scratch is a shared funnel. Two options:

- **(A) minimal:** keep it a scratch but treat it as strictly loop-local — never read it outside the
  `chans[]` iteration that set it, and pass `NvkvmProc*`/`NvkvmVas*` explicitly into
  `chan_translate`/`own_pdb`. (C-retrofit; lowest churn.)
- **(B) clean:** delete the scratch; make `chan_execute(NvkvmGpuEmul*, NvkvmProc*, ChanEntry*)` take
  the channel + proc explicitly. (Preferred end-state; matches the Rust shape §7.)

Choose (A) for P1–P3 (byte-identical), migrate to (B) in P5.

### 3.4 Separating system vs process state

Rule: **a page guest userspace can write to → per-process; a kernel-only page → system**
(`mode2_forwarding_model.md` "delineation principle"). Concretely: GR/compute channels, their VASes,
mappings, USERD/GPFIFO/pushbuffers/sema → `NvkvmProc`. The CE-scrubber's kernel USERD, GSP RPC ring,
BAR page-dirs, interrupt tree, finishPayload forge (kernel channels only) → `m2_sys`. The v3
finishPayload exclusion (`:3837`) already draws this line for completions; the refactor generalizes
it to *all* state.

---

## 4. The page-table-publication piece (the #14 crux — P4, load-bearing)

This is the wall (`mode2_multiprocess_isolate.md` §"remaining wall"; `mode2_14_concurrent_apps`
rounds 5–6): the loser's own PDB is walkable PD3→PD2→PD1 but **`PD0[1] @0x340a010 = 0`** — one leaf
PDE is never present in our FB shadow, so the loser's working-set VA `0x200200000` FAULTs under its
own tree. Two independent findings pin *why*, and both must be addressed:

**Finding 1 (round 6, decisive): there is no bind-time RPC to forward-populate the leaf from.** On
the GSP-emulated compute path, `DMA_FILL_PTE_MEM` (0x801802)=0 occurrences; channel-alloc/PROMOTE_CTX
carry the GPFIFO **VA** + handles but never the **phys**; both §5 invalidate transports
(`INVALIDATE_TLB` RPC fn=200, `MEM_OP`/`MMU_TLB_INVALIDATE`)=0. The compute working set's leaf PTEs
are published **exclusively through the CE page-table-write data plane** (kernel-RM CeUtils
identity-map CE copies to the PD pages — the same mechanism #13 handled via
`b83d0b4`/`ce_fb_write_hook`). **→ The binding must come from the exec-time CE PT-write.**

**Finding 2 (round 5): the loser's PT-write push starves — two paths.**
(a) Its PT-writer channel's ring page is evicted by the `bar1_wpg` MRU-of-last-64-pages heuristic
(row 32) under doubled 2-proc BAR1 traffic (`RING-DARK`) → the push carrying the loser's leaf never
executes. (b) A **second** starvation path: the leaf push is dropped even *without* ring eviction
(seen on role-swapped boots).

### 4.1 The approach

**Step 4a — per-PDB PT-write capture (attribution).** Key the `ce_fb_write_hook` capture
(`m2_gr_pt_set` row 30 / `m2_cpt` row 31) by the **destination FB address' owning PDB**. Since the
CE write's destination *is* a physical page inside a specific process's page-directory range, the PDB
is derivable from the destination alone — **no CR3 needed for capture, and (per §1.2) none needed
for security either**: the attribution signal is entirely GPU-side, exactly the address-table
directive (`mode2_address_table_of_truth`). Each process's PD leaves then populate its own tree's
shadow. A capture whose destination maps to no known PDB is a **fault-log**, never guessed (§6).

**Step 4b — per-process ring-pin (schedule, path-a).** Adopt round-5's ring-pin (proven to reach 2×
`cup8` both-pass *once*): pin each channel's resolved ring page
(`chans[i].gpfifo_phys/bar1off`, forward-populated at first `bar1_wpg` success, reset at
channel-alloc) so doubled BAR1 traffic can't evict the PT-writer's ring. **#12-safety (the blocker
that stopped round 5 landing it):** *invalidate the pin at channel-free / ring-drain* so a stale pin
isn't consumed across libcuda driver-unload (round 5's ungated pin regressed `cupctx2_min`). Since
the pin now lives on the per-`NvkvmProc` channel and is reaped with the process, and is invalidated
at channel-free, the #12 single-process teardown path never sees a foreign stale pin — so the pin can
be **on by default** (not `multiproc()`-gated), removing the round-5 "gated ⇒ one-pass / ungated ⇒
#12-regress" tension.

**Step 4c — close the 2nd starvation path (schedule, path-b).** Round 5 saw the loser's `PD0[1]` push
dropped with the ring healthy. Hypotheses (`mode2_multiprocess_isolate.md` §open): the push is
dropped at (i) doorbell scheduling (the loser's PT-writer TSG sits off-runlist), or (ii) CE-copy
resolution (the push resolves under the wrong PDB and is skipped). Per-process channel scheduling —
each process's PT-writer/compute channels schedulable independently, **keyed on channel/PDB**
(the submitting channel via the doorbell CHID demux §1.4, or the destination-PDB of the write) —
makes no process's pushes starvable by another's. Concretely: extend `m2_tsg_sched` (row 21) + the
doorbell re-sweep (`exec_doorbell` `:8337`) to iterate **per `NvkvmProc`** and schedule each
process's PT-writer TSG. Only if E0 showed the CHID ambiguous does the CR3-at-the-userspace-
doorbell tiebreak (§1.4) enter to attribute a submission to a process's ring — never CR3 at the
PT-write hook itself (that path can run in kernel context, where CR3 is unreliable, §1.3).

**Step 4d — resolve, don't guess.** With 4a+4b+4c, each process's leaf PTEs land in its own PDB's
shadow, and exec is a **pure per-PDB table lookup** (`address_table.md` §3/§6); the blind content-pick
(`chan_execute` pass-1, `:5271`) and per-doorbell re-sweep are deleted (P5). MISS=FAULT.

### 4.2 Open risks (P4)

- **R-P4-1:** capture-by-destination might not attribute *every* PT-write (e.g. a shared identity-map
  scratch page). Mitigation: attribute via the submitting channel (doorbell CHID demux, §1.4);
  the CR3-at-the-userspace-doorbell tiebreak only as last resort if E0 showed ambiguity. No CR3 at
  the PT-write hook (kernel-context CR3 is unreliable, §1.3).
- **R-P4-2:** the 2nd starvation path's root (4c-i vs 4c-ii) is *unconfirmed* — round 5 only observed
  it. P4 must begin with a per-channel exec trace (`mode2_multiprocess_isolate.md` §open q1) to
  decide. This is the single most uncertain step in the whole plan.
- **R-P4-3:** `cpu_synchronize_state` cost — only if the E0 fallback is engaged (doorbell-only,
  §1.4). Mitigation: destination-PDB attribution (4a) + the CHID demux keep CR3 entirely off the
  path in the expected case; if engaged, budget the sync / cache CR3 per-vCPU-at-exit.

---

## 5. Phasing (each individually shippable + single-process-byte-identical)

Every phase must be green on the ladder `cup2` / `cupctx2_min` (#12) / `cup8` / `cup8_iter` (#13)
**before** the next, and must preserve the group-E system state device-global. "v3 done" marks work
already in the baseline.

**P0 — capacity + reap hygiene (no behavior change).** Grow the under-capacity arrays for N procs
(`chan_vas` 16→64, `m2_grmap` 8→32, `m2_cvas` 16→64, `m2_gr_clients`/`m2_user_clients` 8→16); add the
missing proc-exit reap to the never-reaped tables (rows 4,5,13,17,18,19,22,28,29) driven off the
existing `ctx_free_drop` (`:1744`) root-client-free path. Pure safety; single-proc identical. *Ships
alone.*

> **★ P0 SHIPPED 2026-07-19 (ladder green) — with one bench-forced amendment.** Reaping the
> RESOLUTION/BACKING tables (rows 4,5 `m2_cli_vas`/`va_map` and 28,29 `m2_objs`/`m2_gpga`) *at* the
> root free is NOT single-process-safe: the dying context's userspace keeps busy-polling
> overlay-backed pages *after* its client-root frees (cupctx2_min hung at CTX2 destroy, post-fn-47;
> A/B with reap disabled passed — deterministic). Fix: those four tables go on a pending list
> (`m2_reap_pend[]`) consumed by `nvkvm_m2_reap_dead()` at the **GSP queue re-handshake**
> (tx-header write = the next context/process boot, after the fn-47 idle-release — provably
> quiesced; bench: 13 clients / 427 entries reaped there, CTX2 rebuilt cleanly). The light
> control-plane tables (rows 13,17,18,19,22 `m2_cmap`/`m2_tsgeng`/`m2_subdev`/`m2_grmap`/
> `m2_user_ce_clients`) reap immediately at root-free as planned. Consequences: (a) `m2_cmap`'s
> host-handle mint is now a monotonic counter (`m2_cmap_next`) so reap can never recycle a live
> synthetic handle; (b) `m2_tsgeng` gained a `client` field (populate-site hClient) purely as the
> reap key; (c) mid-life multi-proc churn (proc exits while another runs → no idle-release → no
> re-handshake) keeps the pre-P0 leak-until-idle behavior for the heavy tables — the real owner of
> that residual is P2's per-proc isolate teardown. Ladder: cup2 rc=0, cupctx2_min rc=0, cup8
> byte-exact rc=0, cup8_iter 5/5 rc=0, all on fresh boots.

**P1 — experiment E0 + `NvkvmProc` registry keyed by PDB-grouping (identity plumbing).** Run
**E0** (§1.4) first: log `(doorbell token, resolved channel, owning PDB)` per doorbell across 2×
`cup8` and decide whether the CHID demux is unambiguous (expected: yes — CHIDs are fresh per
channel-create, unlike the reused RM handles `0x5c000019`). Build the `m2_proc[]` registry keyed by
**PDB-grouping**: create a proc on first sight of a new owning-PDB group via the dup-edge chain
(GR client ↔ UVM VAS ↔ PDB, `:4700/:4720`); route kernel/GSP/scrubber traffic to `m2_sys`
(identified by traffic class — kernel RM clients / scrubber channels — not by CR3). **Only if E0
shows ambiguity:** add `nvkvm_cpukey.c` (`specific_ss`, the round-5 `mode2_14_cr3.patch` TU —
`nvkvm_gpu_emul.c` is target-independent `system_ss` and can't read `X86CPU`; `meson.build`
`specific_ss.add`) with `nvkvm_current_guest_cr3()` (`cpu_synchronize_state(current_cpu);
env.cr[3] & ~0xfff`, the `vapic.c` pattern), read at the **userspace doorbell only**,
budgeted/cached per §1.4. **No CR3 stamping at the `chan_vas`/`chans`/`m2_dup` populate sites**
(dropped — kernel-context RPCs, CR3 unreliable there, §1.3). **No table is re-keyed yet** —
registry + logging only, so behavior is byte-identical. **Simpler than the old P1:** no
`cpu_synchronize_state` on the RPC path at all, and in the expected E0 outcome no new TU and no
CPU-state read anywhere. *Ships alone; verify the per-process PDB-groups + E0's verdict in the log.*

> **★ P1 SHIPPED 2026-07-19 (ladder green + E0 re-confirmed live).** Built the `m2_proc[]`
> registry (`struct nvkvm_proc { clients[]; pdbs[]; live }`, cap `NVKVM_MAX_PROCS=16`) keyed by
> **PDB-grouping via the dup-edge chain** — a proc is anchored on the dup SRC (user compute
> client), joined by the dup DST (its UVM gpu-ops client), and accretes PDBs at the
> `RESERVED_PDES`/`SET_PAGE_DIRECTORY` capture sites (`nvkvm_m2_proc_get/add_client/add_pdb`,
> reaped by `drop_client` at anchor root-free). Recovered the guest **vChid** at channel-alloc
> from the `USERD_INDEX` flags (`chid = flags[20:12]*8 + flags[10:8]`, per open-driver
> `kernel_channel.c:2688` GSP-client ChID plumbing; stored `chans[].vchid`). Extended the M5.11
> DOORBELL log to resolve `token[11:0]=vChid → chans[] → owning proc`. **All keying is
> registry+logging only — nothing consumes it — so single-process is byte-identical (ladder:
> cup2/cupctx2_min/cup8/cup8_iter all rc=0 on fresh boots).** **Live E0 verdict (2× cup8, both
> rc=0 byte-exact): the demux is CLEAN** — 2 procs registered with the plan's exact PDBs
> (PROC[0]=0xc1d00004→PDB 0x3401000, PROC[1]=0xc1d00005→PDB 0x3405000); every one of ~40 distinct
> vChids maps to exactly ONE channel and ONE proc, zero collisions. CR3 confirmed unnecessary;
> `nvkvm_cpukey.c` never built. (Note: 2× cup8 already both-passed here — P1/P2 arming separation
> from process start, ahead of the plan's "P3 = one reliably passes" expectation; not yet
> load-bearing, since no table keys on the registry until P3.)

**P2 — per-process host isolate.** For each `NvkvmProc`, create its own isolate via the existing
`nvkvm_isolate_create(..., session_id=ordinal, ...)` (the infra exists,
`NVKVM_ISOLATE_MAX=4096`); route that process's forwarded control/alloc/map ioctls to *its* isolate
(rows 11–15,16,18,19,26 → per-proc). **Grouping key = the process's PDB-set**, with channels/clients
joined to the proc via the doorbell/CHID demux + dup-edge chain — NOT CR3-at-RPC (per-process
rather than per-VAS is a §1.2-point-6 simplicity choice; per-VAS fallback is safe). Kernel/GSP/
scrubber traffic → `m2_sys`'s isolate. Expose the guest-RAM share into each isolate (risk R3).
This is the `mode2_isolate_consolidation_plan.md` spine re-grouped per-process. Note the isolates
are a *robustness/blast-radius* structure here — the security boundary is their unprivilege + QEMU
(§1.2), which one isolate or N preserve equally. **Single-process:** exactly one `NvkvmProc` +
`m2_sys` → one isolate as today, byte-identical. *Ships alone.*

> **★ P2 BANKED 2026-07-19 — deferred to land WITH P4 (tree stays green at P1; see
> `mode2_14_P2.patch`).** Standalone P2 is a *regression*, not "ships alone": the **execution
> plane is not isolate-split until P4**. `nvkvm_m2_doorbell_setup`/`exec_doorbell` ring **one**
> host doorbell page (`m2_usermode_qva`, from the base isolate's `AMPERE_USERMODE_A`) with the
> first GR channel's token (`m2_gr_channel`/`m2_gr_token`/`m2_doorbell_ready` — singletons, plan
> rows 23/32 = explicit P4 work). If P2 puts proc[1]'s host channels/objects in a *separate*
> isolate, the base doorbell page never rings proc[1]'s host channel → its work never runs →
> this **regresses the P1 result** (2× `cup8` both-pass), which works *only because* both procs'
> control AND execution share one isolate so the single doorbell reaches both. P2's control-split
> and P4's execution-split are **mutually dependent** for a 2nd process to run on its own isolate;
> they must land together. The banked design (a "current isolate context" activated per RPC/
> channel at dispatch entry — byte-identical for one process by construction, avoids threading a
> ctx through ~20 forwarding fns) + the P4 co-requisites are in `mode2_14_P2.patch`. **Security is
> unaffected** (§1.2: an isolate's security = its unprivilege + QEMU, identical for 1 or N
> isolates), so deferral loses no security property. **P3 is independent of the isolate split**
> (it keys data-plane *tables* by PDB, on the shared isolate) and is taken next.

**P3 — PDB-key the data-plane tables.** Move rows 3,4,5,20,25,27,31 onto the per-VAS (PDB) sub-tables
of `NvkvmProc`. Formalize `m2_cpt` (row 31, already pdb-keyed) and `m2_fbback` (row 25) as per-VAS.
Resolve every channel to its PDB via the v3 dup-chain and select backing/mapping/sema by PDB.
**Single-process:** one PDB per VAS → the filter is a no-op → byte-identical (v3 already proved the
client-scoped pick is a no-op for one process). *Ships alone.* **After P3, 2× `cup8` should reach
"one reliably passes" without the transition-window fragility of round-3** (because P1/P2 armed
per-process separation from process *start*, not at the 2nd `0xc7c0`).

> **★ P3 BANKED 2026-07-19 — behavioral target already MET at P1; mechanical re-keying folds into
> P4-4a (tree stays green).** Empirically, **the P3 target outcome is already satisfied by the P1
> committed state:** across 5 fresh-boot 2× `cup8` runs the **winner ALWAYS completes rc=0
> byte-exact** (A or B), and single-process is byte-identical (ladder green) — i.e. "one reliably
> passes, single-proc untouched." The reason: the data plane **already separates by
> distinct-per-proc client** (v3: `m2_mapped_va` is `(client,va)`-keyed, `m2_cli_vas`/`va_map`
> client-keyed, per-client `cvas`/`grmapper` host VASes; P1 registered distinct PDBs per proc,
> e.g. `0x3401000`/`0x3402000`). So converting rows 3,4,5,20,25,27,31 from client-key to PDB-key
> is a **single-process no-op AND a 2-proc no-op for the winner** (distinct clients already
> disambiguate the same guest VA) — pure hot-path churn on the load-bearing #12/#13 backing/sema
> resolvers for **zero behavioral change**, which the "correctness over green-test-hacks" mandate
> says not to do speculatively. **The loser's flaky hang is NOT a P3 data-plane-aliasing bug — it
> is the P4 §4 wall** (its own `PD0[1]` leaf is never published into its PDB's FB shadow because
> its PT-writer push starves; evidence: the loser sits in a busy `MC_SERVICE_INTERRUPTS`/backing
> re-sweep loop with its compute-aperture VAs `st=0x51 ALREADY-MAPPED`, never completing). That is
> exactly what **P4-4a (per-PDB PT capture)** consumes the per-VAS sub-tables *for*; re-keying them
> without 4a's consumer would be dead code. **Decision:** land the rows-3/4/5/20/25/27/31 per-VAS
> re-keying **together with P4-4a**, where it becomes load-bearing, rather than as a standalone
> single-proc no-op. Tree stays green at P1 (which meets the P3 acceptance behavior). See
> `mode2_14_P3.patch` for the row-by-row re-keying spec + the P4-4a coupling.

**P4 — per-process page-table publication (THE crux, §4).** 4a per-PDB PT capture, 4b default-on
#12-safe ring-pin, 4c per-process PT-writer scheduling, 4d delete the content-pick fallback.
**Riskiest phase** (R-P4-2). Verify `cup2`/`cupctx2_min`/`cup8`/`cup8_iter` **and** 2× `cup8` both
`rc=0`. *Ships alone once both-pass.* **Fallback if 4c stalls:** keep the ring-pin (4b) — it already
reached both-pass-once; ship "2× reliably" behind default-on 4a+4b and continue 4c as a follow-up
(strictly better than the v3 one-pass baseline, no regression).

**P5 — delete the heuristic cascade + the multiproc gate (consolidation).** With P3/P4 making exec a
pure per-PDB lookup, delete `chan_execute`'s blind pass-1 (`:5271`), the per-doorbell re-sweep, the
`chan_*` global scratch (migrate to §3.3(B)), and shrink/remove `nvkvm_m2_multiproc()` (`:4657`) where
the per-proc key already disambiguates. This is the `address_table.md` §12 / `refactor_debt` "one
exec path / one resolver" cleanup. Verify the full ladder. *Ships alone.*

**P6 — per-process completion delivery + N× scale.** Make `m2_poll_kick`/`m2_last_db_token` (row G)
per-`NvkvmProc` so one process's `MC_SERVICE_INTERRUPTS` poll doesn't overwrite another's pending
kick; deliver each process's completion independently (`mode2_multiprocess_isolate.md` §3). Verify 3×
and 4× concurrent `cup8` all `rc=0`, then concurrent + multi-iter (`cup8_iter`). *Final phase.*

**Riskiest step overall: P4-4c** (the unconfirmed 2nd starvation path). Its fallback (ship 4a+4b for
robust 2× and defer 4c) keeps every earlier phase's gains.

---

## 6. Constraints check, testing, risks

### 6.1 Invariant preservation (per phase)

- **Unprivileged host ops only** (`mode2_forwarding_model.md` §"0x1b lesson"): P2 creates *more*
  isolates but each is the same unprivileged stub session as today (`session_id` differs, privilege
  does not). No phase adds a privileged/GSP-internal replay; PROMOTE_CTX etc. stay ack-only. A
  per-process isolate issuing a Case-2 control would still get `0x1b` — the split is unchanged.
- **Emulate-kernel / passthrough-userspace** (`mode2_forwarding_model.md` §"two classes",
  `access_model_split`): the emulate-vs-forward decision is made *before* keying and is unchanged by
  it. Kernel/GSP/scrubber → `m2_sys` (emulated / forged as today); userspace GR/compute → per-proc
  isolate (forwarded as today). §3.4 draws the line exactly where the forwarding model does.
- **Cross-VM/host boundary is QEMU's job; intra-VM is the guest kernel's** (`access_model_split`):
  P2's per-process isolate is *host-side* sandboxing (cross-trust-domain) and does **not** add
  intra-VM access checks in QEMU (the reverted H-1 lesson). **The keying is not the boundary** —
  the corrected model (§1.2): each isolate is unprivileged, so PDB-keying preserves the host
  boundary identically to any other key (points 1/4); PDB is set up by guest *kernel* RM, so guest
  userspace cannot spoof another process's PDB (point 3); a compromised guest kernel can reshuffle
  isolate routing but gains no host reach and no intra-guest escalation it did not already have
  (point 4); and same-VAS processes already share GPU memory, so a shared isolate leaks nothing
  new (point 5). The isolate key selects *which sandbox*, never *access rights* — and the residual
  CR3 caveats (reuse-on-exit reap, PTI/PCID masking, doorbell-only reliability, §1.4) apply only
  to the optional doorbell tiebreak, not to any security property.
- **One forward-populated table, MISS=FAULT** (`address_table.md`): P3/P4/P5 move resolution *toward*
  the table-of-truth (delete the cascade), never away. No phase adds a reverse-resolve.

### 6.2 Test ladder

1. **Regression floor (every phase):** `cup2` (byte-exact), `cupctx2_min` (#12 CTX1+CTX2), `cup8`
   (byte-exact), `cup8_iter` all-5 (#13). Any red = stop.
2. **2 proc:** `scripts/mode2_diag/cup8_concurrent_run_guest.sh` — both `rc=0` byte-exact (P4 gate).
3. **3+ proc:** extend the runner to 3× then 4× `cup8` (P6 gate) — the old
   `multiproc_collision_blocker` 4-concurrent target.
4. **Concurrent + multi-iter:** 2× `cup8_iter` (combines #13 + #14) (P6).
5. **Isolation smoke:** confirm one process's fault/exit does not wedge another (P2/P6) — the Mode-1
   reaper path (`teardown_hardening_done`).

*Bench is down for this planning task; the ladder is the acceptance spec for when it returns. Each
phase is written to be verifiable independently so work can resume from any phase.*

### 6.3 Top risks + mitigations

- **R1 — single-process regression.** The #1 danger (round 3's always-on refusal regressed #12).
  *Mitigation:* every phase is byte-identical for one process by construction (one PDB-group → one
  `NvkvmProc`; one PDB per VAS → filters are no-ops); the regression floor (6.2.1) gates every phase;
  P4's ring-pin is #12-safe by pin-invalidate-at-channel-free (§4.1), removing round 5's tension.
- **R2 — E0 comes out ambiguous.** If the doorbell token/CHID cannot uniquely attribute a ring
  (§1.4), the CR3-at-doorbell fallback engages and its costs return (`cpu_synchronize_state` per
  doorbell timed out CTX2-create in round 5). *Mitigation:* scope CR3 strictly to the userspace
  doorbell; budget the sync / cache CR3 per-vCPU-at-last-exit / read once per submission burst;
  prefer destination-PDB attribution everywhere else (P4-4a). Expected case: CR3 is never read.
- **R3 — GPA-arena / isolate resource exhaustion.** N per-process isolates each carving mappings +
  the guest-RAM share (§2.G, `multiproc_collision_blocker`). *Mitigation:* adopt the resolved Mode-1
  fixes (per-fd arena, MAP_FIXED, slot-recycle, single-window heap); reject overlapping arenas at
  install; reap arena on process-exit. Confirm `KVM_CAP_NR_MEMSLOTS` headroom for N isolates.
- **R4 — P4-4c 2nd starvation path unconfirmed.** *Mitigation:* start P4 with a per-channel exec
  trace; fallback ships 4a+4b (robust 2×) and defers 4c.
- **R5 — never-reaped tables leak across process churn.** *Mitigation:* P0 adds proc-exit reap to all
  of them, driven off `ctx_free_drop`.

---

## 7. Appendix — "if this were Rust instead"

The single-process assumptions are woven through channel registration, VAS selection, backing, and
scheduling (`mode2_multiprocess_isolate.md` §"why this is the Rust rewrite's job"). Mapping this plan
onto the clean Rust core (`rewrite_horizon_target`):

**Maps cleanly to Rust structures (the plan's shape *is* the Rust shape):**

- `NvkvmProc` → `struct Proc { vases: HashMap<PdbRoot, Vas>, isolate: Isolate,
  cr3: Option<Cr3> /* E0-fallback doorbell tiebreak only */ }`; the device holds
  `procs: Vec<Proc>` + `system: Proc`, indexed by `PdbRoot → proc` on the data plane and by
  `chid → channel → proc` at the doorbell. **PDB-primary keying maps 1:1 onto the Rust core**: all
  keys are GPU-side inputs (PDBs, CHIDs, FB addresses), so the core stays target-independent —
  exactly the `mode2_language_rust` boundary (no `X86CPU`/vCPU dependency in the logic core). The
  §3.2 "linear scan + last-hit cache" hand-optimization disappears — a `HashMap` (or `SmallVec`
  for N≤4) is idiomatic and the borrow checker enforces the "load scratch per iteration, never
  read across" discipline §3.3 that C leaves to convention.
- The per-VAS table → `HashMap<PdbRoot, IntervalMap<VaRange, Binding>>` behind one `RwLock` per VAS
  (`address_table.md` §12) — two populate entry points (RPC, PT-capture), one lookup, no heuristics.
  The row-2 `chan_*` global scratch simply does not exist (§3.3(B) is the default).
- Reap → `Drop`. The ~9 "never reaped" tables (R5) become owned fields of `Proc`; dropping the `Proc`
  frees them, eliminating the whole manual-reap phase (P0's reap work + row-4/5/13/… drops vanish).
- The emulate-vs-forward split → an enum at the type level (`Kernel` traffic → `system`, `User` →
  `proc`), making §3.4's delineation a compile-time guarantee rather than a runtime exclusion list
  (the v3 `finishPayload` exclusion `:3837` becomes unrepresentable-if-wrong).

**C-retrofit scars (present only because we finish in C):**

- The `multiproc()` gate (`:4657`) and its six gated divergences exist *only* to keep single-process
  byte-identical while the tables are half-re-keyed. In Rust every path is per-`Proc` from the ground
  up, so the gate never exists — P5's "delete the gate" is free.
- The `nvkvm_cpukey.c` `specific_ss` TU split (target-independent `system_ss` can't see `X86CPU`)
  is a QEMU-build-system scar — and post-correction it is built **only** in the E0-ambiguous
  fallback. In the Rust core the optional CR3 tiebreak is just an `Option<Cr3>` field fed from the
  vCPU snapshot at the shell boundary; in the expected E0 outcome it does not exist at all.
- The flat fixed-capacity arrays (`chan_vas[16]`, `m2_grmap[8]`, `m2_mapped_va[65536]`, …) and their
  P0 cap-bumps are pure C artifacts; Rust collections grow.
- The global `chan_*` scratch funnel (§3.3) is a C-only hazard; there is no equivalent to retrofit.

**Verdict for the finish-in-C-vs-port decision:** P0–P3 are mechanical re-keying that C tolerates and
that de-risks the design regardless of language. **P4 is the genuinely hard part and is
language-independent** (it is a GPU-scheduling/attribution problem, not a data-structure problem) — so
P4 is worth *proving* in C (the WIP patches already got 2×-both-pass once) before committing to the
port. P5's cleanup (delete gate + scratch + cascade) is exactly the work the Rust rewrite gets for
free, so if the port is on the near horizon, stop after a robust P4 and let the rewrite absorb P5/P6.
