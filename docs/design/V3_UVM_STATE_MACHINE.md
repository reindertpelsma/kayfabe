# V3_UVM_STATE_MACHINE — the managed-page state machine, read from nvidia-uvm

**STATUS: RESEARCH, 2026-10-02, from nvidia-uvm 580.159.04.** Read from source only; nothing in
this note was run. Written on branch `v3-uvm-state-machine`, cut from master `d88639f9`, the commit
that recorded the 2026-10-02 UVM rulings (`docs/OWNER_RULINGS.md` §E). The question: what states can
a managed (UVM) page be in, what moves it between them, and who does the moving. Kayfabe runs the
stock guest nvidia-uvm against an emulated GPU, so the guest-UVM design (`V3_UVM_DEMAND_PAGING.md`,
the guest fault plane on `v3-uvm-guest`) has to host exactly this machine. The owner's five-state
hypothesis is judged in §7.

**Conventions.** Paths are relative to `kernel-open/nvidia-uvm/` in ogkm-580.159.04
(`research_clones/ogkm-580.159.04` in the nvkvm tree). Paths starting with `src/nvidia/` (RM) or
`kernel-open/nvidia/` (nvidia.ko glue) are in the same tree; paths starting with `crates/` or `docs/`
are in this repository. Line numbers are at that tag. `[src@580]`
marks a statement read in that tree. `[inf]` marks an inference from the reading that is not itself
read. Module-parameter values are the compile-time defaults; an admin can override them at load
time. "x86 PCIe" means the case kayfabe targets: x86_64, 4 KiB pages, a discrete GA10x on PCIe, no
ATS, no C2C, Confidential Computing off.

---

## 0. The answer in one screen

1. **There is no owner field.** The code keeps, for each 4 KiB page of a va_block of at most 2 MiB,
   three things: the set of processors holding a coherent copy (*residency*), each processor's PTE
   permission (NONE / RO / RW / RWA), and overlay bits (read-duplicated, discarded, evicted, thrashing
   pin or throttle). "Authoritative" is implied, not stored. A single resident copy is authoritative
   where it lives. Several copies exist only as a read-duplicated page, and then every mapping is
   read-only (§2). `[src@580]`
2. **One function decides where a faulting page goes.** It is `block_select_processor_residency`
   (`uvm_va_block.c:11612-11764`): twelve ordered rules over the access type, the range policy, the
   thrashing hint and the processor-capability matrices (§3.1). GPU replayable faults, non-replayable
   faults, CPU faults and access-counter notifications all end there. Explicit migrations
   (`UVM_MIGRATE`) and policy calls bypass it (§4.5, §4.6). `[src@580]`
3. **Nothing below nvidia-uvm decides residency.** The GMMU writes fault packets and, if enabled,
   access-counter notifications. The copy engine runs the copies, zeroing memsets and PTE writes that
   UVM pushes. Host methods run replay, cancel and TLB invalidation. GSP-RM programs buffer registers
   and moves non-replayable CE/PBDMA faults into a shadow buffer. No RM or firmware path carries a
   placement decision (§5). `[src@580]`
4. **On x86 PCIe the live machine is smaller than the code.** The CPU never maps vidmem (§2.5).
   Access-counter migration is off by default on every non-ATS system. The counters are not even
   switched on in hardware, and a 4 KiB-page kernel drops every notification unserviced (§4.8).
   Eviction leaves no remote mapping (§4.9). The only *automatic* "map remotely instead of migrating"
   mechanism is the thrashing PIN (§4.7). Every other remote mapping is user policy (accessed-by,
   preferred location). `[src@580]`
5. **Verdicts (§7).**
   - H1: confirmed.
   - H2: confirmed as the default, with five policy and heuristic exceptions.
   - H3: partly. A write *collapses* the duplicate set onto the writer, and it skips the copy only
     when the writer already holds one.
   - H4: confirmed, and the state is entered in more ways than hypothesised.
   - H5: confirmed for x86 PCIe. The state is missing there because of the platform, not by design:
     coherent C2C systems have it.
   - The GSP switch between DMA and trap modes: refuted. `[src@580]`
6. **For kayfabe (§8).** Every transition triggered by a GR replayable fault keeps the host GR engine
   held for the guest's whole service. NVIDIA's own comment says replayable faults stall other
   channels (`uvm_gpu_non_replayable_faults.c:78-81`), and THROTTLE deliberately lengthens the stall.
   Prefetch, advise, CPU faults, non-replayable faults, eviction and discard never hold GR. The
   primitives that must be faithful:
   - the fault packet and its replay and cancel semantics;
   - the RO and ATOMIC_DISABLE PTE bits;
   - the TLB invalidate as the commit point;
   - the Ampere CHRAM clear-faulted register write.

---

## 1. Granularity

| unit | size | what it scopes | source |
|---|---|---|---|
| va_block | at most 2 MiB; never crosses a 2 MiB boundary | one lock, one work tracker, one GPU PDE; every operation runs under one block lock | `uvm_va_block_types.h:35-45`; `uvm_va_block.h:60-73` |
| page | `PAGE_SIZE` (4 KiB on x86_64) | every residency, mapping, read-dup, discard, evicted and thrashing bit | `uvm_va_block_types.h:50`; `uvm_va_block.h:216-225` |
| GPU PTE | 2 MiB, big (64 KiB on GA10x) or 4 KiB, chosen per block from the page state | the hardware mapping only; the state stays per page | `uvm_va_block.h:166-214` |
| GPU backing chunk | naturally aligned, the largest that fits in the block | allocation, and eviction by 2 MiB root chunk | `uvm_va_block.h:114-120`; `uvm_mmu.c:2439` |
| CPU backing chunk | 2 MiB, 64 KiB or `PAGE_SIZE` | allocation | `uvm_pmm_sysmem.h:39` |
| fault packet | one page-aligned address | fault service | `uvm_gpu_replayable_faults.c:919` |
| fault batch | up to 256 packets | one service pass | `uvm_gpu_replayable_faults.c:73-78` |
| prefetch tree | the block, aligned to the big-page size | density prefetch | `uvm_perf_prefetch.c:258-285` |
| access-counter region | 2 MiB, 32 sub-regions | notifications, where enabled | `uvm_gpu_access_counters.c:41`, `:56` |

---

## 2. The state space, as the code models it

### 2.1 Residency: which processors hold a coherent copy

- **GPU g.** One bit per page in `gpus[g]->resident` (`uvm_va_block.h:101-109`). The backing is in
  `gpus[g]->chunks[]` (`:114-120`). A chunk can exist without residency: it is zeroed when it is
  populated and kept for reuse (`uvm_va_block.c:2899-2994`).
- **CPU.** Per NUMA node, `node_state[nid]->resident` and `->allocated` (`uvm_va_block.h:232-262`).
  These are OR-ed into `cpu.resident` and `cpu.allocated` (`:356-378`). The header says: *"A page may
  be resident on multiple processors (but not multiple CPU NUMA nodes) when in read-duplicate mode"*
  (`:240-242`). A cleared resident bit over an allocated chunk is *"a cached chunk which can be
  reused"* (`:244-250`).
- **Block summaries.** The `resident` processor mask (`:302-318`). `ever_fully_resident`
  (`:326-329`), which only steers zeroing.
- **Unpopulated** means no processor is resident. A new managed range has no va_blocks at all; they
  are created on first use (`uvm_va_range.c:186-207`, `:1475`). The default policy is preferred
  location INVALID, read-dup UNSET, accessed-by empty (`uvm_va_policy.c:32-36`).

### 2.2 Mapping: what each processor's PTE allows

- **The permission lattice** is `UVM_PROT_NONE < READ_ONLY < READ_WRITE < READ_WRITE_ATOMIC`
  (`uvm_hal_types.h:220-229`). It is stored per page and per processor as PTE-bit masks:
  - CPU: {READ, WRITE} (`uvm_va_block.h:79-86`, `:381-407`). For the CPU, write implies atomic
    (`uvm_va_block.c:5368-5383`).
  - GPU: {READ, WRITE, ATOMIC} (`uvm_va_block.h:88-99`, `:216-226`).
- **The mapping's target is not stored.** *"We don't track the specific aperture of each mapped page.
  Instead, we assume that each virtual mapping from a given processor always targets the closest
  processor on which that page is resident"* (`uvm_va_block.c:2609-2612`).
- **GPU state is exact; CPU state is a bound.**
  - GPU `pte_bits` *"are always accurate"* (`uvm_va_block.h:216-220`).
  - For managed memory the CPU bits are the *maximum*: the kernel can downgrade the PTE (for example
    on `MADV_DONTNEED`) without telling UVM (`:395-400`).
  - For HMM they are the *minimum* (`:402-406`).
- **On the wire (GA10x).** Ampere uses the Turing PTE encoder (`uvm_ampere_mmu.c:57-80`,
  `uvm_turing_mmu.c:35-80`):
  - RO sets `READ_ONLY`;
  - RW sets `ATOMIC_DISABLE`;
  - RWA sets neither;
  - NONE is no valid PTE.

  A remote sysmem mapping is `VOL` (uncached) unless the GPU is fully coherent
  (`uvm_va_block.c:127-155`).
- **Summaries.** The `mapped` processor mask (`uvm_va_block.h:331-340`). `maybe_mapped_pages`
  (`:459-471`) is a fast-path hint that a page is mapped nowhere. `[src@580]`

### 2.3 Overlay bits that change transitions

| overlay | stored in | what it changes | source |
|---|---|---|---|
| read-duplicated | `read_duplicated_pages` | the page may have two or more copies; all mappings are RO, each to the mapper's local copy | `uvm_va_block.h:456-457`; `uvm_va_block.c:11393-11408` |
| discarded | `discarded_pages` | contents are undefined: not copied on migration, re-zeroed on population, read faults map RO | `uvm_va_block.h:319-324`; `uvm_va_block.c:4680-4685`, `:11100-11107` |
| evicted | `gpus[g]->evicted`, `evicted_gpus` | records that g held the page before eviction; read only by the deferred eviction mappings | `uvm_va_block.h:111-112`, `:342-352`; `uvm_va_block.c:5028-5047`, `:13227-13324` |
| thrashing | per-page `page_thrashing_info_t` in the perf module | PIN (fixed residency plus remote mappings) or THROTTLE (deferred service) | `uvm_perf_thrashing.c:47-120` |
| prefetch history | `prefetch_info` | last destination, and the fault count toward `min_faults` | `uvm_va_block.h:491-499`; `uvm_perf_prefetch.c:340-343`, `:410` |
| CPU fault race window | `cpu.fault_authorized` | a CPU fault on a page the CPU is already authorized for is ignored for 300 µs if it comes from another thread (a mapping race); the same thread faulting again means the kernel downgraded the PTE, and forces a remap | `uvm_va_block.h:414-441`; `uvm_va_block.c:55`, `:12745-12788` |
| `force_4k_ptes` | per GPU | sticky 4 KiB PTEs after a fatal fault; Pascal only, because Volta+ returns early | `uvm_va_block.h:159-164`; `uvm_va_block.c:13569-13581` |
| zombie range | range state | the vma is gone (multi-process sharing mode): logical prot NONE, GPU faults fatal | `uvm_va_block.c:8184-8186`, `:12503-12504`; `uvm_va_range.c:622-630` |
| EGM | `gpus[g]->egm_pages` | sysmem reached through C2C (coherent platforms only) | `uvm_va_block.h:228-229` |

### 2.4 Policy, per range

| field | values | set by | source |
|---|---|---|---|
| `read_duplication` | UNSET, ENABLED, DISABLED | `UVM_ENABLE/DISABLE_READ_DUPLICATION` (cudaMemAdviseSetReadMostly and its unset) | `uvm_va_policy.h:35-41`, `:59-60` |
| `preferred_location`, `preferred_nid` | a processor or INVALID; a NUMA node for the CPU | `UVM_SET/UNSET_PREFERRED_LOCATION` | `uvm_va_policy.h:62-76` |
| `accessed_by` | processor mask | `UVM_SET/UNSET_ACCESSED_BY` | `uvm_va_policy.h:78-80` |
| range-group migratability | allowed or prevented | `UVM_SET_RANGE_GROUP`, `UVM_PREVENT/ALLOW_MIGRATION_RANGE_GROUPS` | `uvm_range_group.c:300-528` |
| system-wide atomics (per VA space) | per processor; on by default for the CPU and every fault-capable GPU | `UVM_ENABLE/DISABLE_SYSTEM_WIDE_ATOMICS` | `uvm_va_space.c:217-219`, `:855-862`; `uvm_policy.c:963-1057` |

Read duplication is honoured only while no non-fault-capable GPU VA space is registered
(`uvm_va_space.c:710-726`, `uvm_va_policy.c:38-42`).

### 2.5 What may map what: the capability matrices

The matrices are `can_access`, `accessible_from`, `can_copy_from`, `has_fast_link` and
`has_native_atomics` (`uvm_va_space.h:251-288`). They are filled at GPU registration
(`uvm_va_space.c:855-901`).

| matrix entry | x86 PCIe | coherent C2C, GPU memory NUMA-onlined | Confidential Computing |
|---|---|---|---|
| GPU on its own vidmem: access and native atomics | yes (`:870`, `:891-892`) | yes | yes |
| GPU on sysmem: access | yes (`:894-897`) | yes | **no** (`:833-842`) |
| GPU on sysmem: native atomics | no | yes (`:877-881`) | no |
| CPU on vidmem: access and native atomics | **no** | yes (`:883-887`) | no |

"Coherent" means RM reported a system-memory window (`uvm_gpu.h:1801-1804`, set at
`uvm_gpu.c:96-108`). Peer GPUs get access per link, with native atomics over NVLink.

Two consequences recur below. On x86 PCIe, only its own GPU (and its peers) can map a GPU-resident
page. Every GPU can map a CPU-resident page, but without native atomics. `[src@580]`

### 2.6 Invariants the code keeps

1. **Two or more copies imply read-duplicated, which implies read-only.** With more than one resident
   copy, a processor maps RO if it holds a copy and nothing otherwise. *"enforce that a
   read-duplication mapping always points to local memory"* (`uvm_va_block.c:11393-11408`).
2. **With one copy, everyone maps that copy** (the closest-resident rule, `:2609-2612`), if the
   matrix allows it (`:11420-11422`; asserted for the CPU at `:8266`).
3. **Non-native atomics are exclusive.**
   - If a processor without native atomics to the residency holds RWA, every other processor is
     limited to RO (`:11442-11448`).
   - RWA is granted only when there is no other writer, the requester has native atomics, or
     system-wide atomics are disabled for the requester (`:11459-11474`).
   - This is how system-scope atomics stay correct over PCIe: by permission ping-pong, not by the
     interconnect.
4. **A move starts with unmapping.**
   - Before a page changes residency, every other processor's mapping to it is removed, including
     read-duplicates (`:4917-4946`).
   - Afterwards, residency elsewhere is cleared (`:5085-5088`).
   - Only accessed-by and thrashing processors are mapped again (`:12393`, `:11211-11360`).
5. **Unpopulated implies unmapped.** With no resident copy the highest permission is NONE
   (`:11389-11391`). UVM never maps a zero page.
6. **State is valid only when the block tracker retires.** Copies, PTE writes and TLB invalidates are
   asynchronous pushes ordered by the block's tracker. *"The residency and mapping state is only valid
   once this tracker is done"* (`uvm_va_block.h:473-480`). `[src@580]`

### 2.7 The named states (x86 PCIe, CPU plus one GPU G)

The names are this note's, not NVIDIA's; they are derived from §2.1-§2.6.

| state | resident on | CPU PTE | G PTE | entered by (§4) |
|---|---|---|---|---|
| **U** unpopulated | nothing | NONE | NONE | range creation |
| **C** CPU-resident | CPU | NONE / RO / RW | NONE | CPU first touch; a CPU fault on G; eviction; GPU unregister; a collapse onto the CPU |
| **C+G** CPU-resident, GPU remote (DMA) | CPU | NONE / RO / RW | RO / RW / RWA, uncached | a GPU fault under preferred-location CPU, accessed-by G or a PIN to the CPU; SetAccessedBy(G); `UVM_MIGRATE` to the CPU with G in accessed-by |
| **G** GPU-resident | G | always NONE | RO / RW / RWA | GPU first touch; a GPU fault on C (the default); `UVM_MIGRATE` to G |
| **D** read-duplicated | CPU and G | RO / NONE | RO / NONE | a read fault or `UVM_MIGRATE` under ReadMostly |

Overlays ride on these states:

- **pinned**: thrashing, C+G or G, for `uvm_perf_thrashing_pin` (150 ms);
- **throttled**: one processor's faults deferred in 500 µs windows;
- **evicted**: C with G's evicted bit set;
- **discarded**: any state, contents undefined;
- **zombie** and **killed**: the range is gone.

On x86 PCIe, two things the hypothesis allows for are absent: **G with a CPU mapping**, and **C+G
entered automatically by eviction or access counters**.

**The core transition matrix** (default policy unless a rule is named; "no fault" means the existing
PTE already allows the access):

| from | GPU read fault | GPU write fault | GPU atomic fault | CPU read fault | CPU write fault |
|---|---|---|---|---|---|
| **U** | G, mapped RWA (rule 7); C+G mapped RW if the preferred location is the CPU | G, RWA; C+G (RW) if preferred is the CPU | G, RWA; C+G (RWA) if preferred is the CPU | C, mapped RW (rule 7) | C |
| **C** | G (rule 12); D under ReadMostly (rule 2); C+G under a PIN, accessed-by G or preferred CPU (rules 6, 8, 10) | G; C+G (rules 6, 8, 10) | G; or C+G with G RWA and the CPU revoked to RO (rules 6, 8, 10) | stays C (map or upgrade) | stays C |
| **C+G** | no fault | no fault (G holds RW or RWA) | stays C+G, G to RWA, CPU to RO; G (rule 12) if no policy or pin holds it | stays C+G | stays C+G; G's atomic is revoked (RWA to RW) |
| **G** | no fault | no fault unless the page is RO under ReadMostly | no fault | C (rule 12), or C+G if G is in accessed-by; D under ReadMostly (rule 2) | C, or C+G if G is in accessed-by |
| **D** | no fault (local RO copy) | G: the CPU copy is invalidated and no data moves (rule 3) | as write | no fault | C: G's copy is invalidated (rule 3) |

The same machine as an edge list, adding the non-fault triggers of §4:

```
U    --GPU fault (rule 7)-----------------------------------> G     (C+G if preferred = CPU)
U    --CPU fault (rule 7)-----------------------------------> C
C    --GPU fault (rule 12), UVM_MIGRATE(G)------------------> G
C    --GPU fault under preferred CPU / accessed-by G / PIN--> C+G   (G mapped remotely, no copy)
C    --SetAccessedBy(G)-------------------------------------> C+G   (mapped at once, no fault)
C    --GPU read fault or UVM_MIGRATE(G) under ReadMostly----> D
G    --CPU fault (rule 12; always on x86 PCIe)--------------> C     (C+G if G is accessed-by)
G    --CPU read fault under ReadMostly (rule 2)-------------> D
G    --eviction, GPU unregister, UVM_MIGRATE(CPU)-----------> C
C+G  --pin expiry (150 ms) unmaps G-------------------------> C
C+G  --GPU atomic / CPU write-------------------------------> C+G   (permission ping-pong, §3.2)
D    --write or atomic by X (rule 3)------------------------> X alone, C or G (no copy: X held one)
D    --UnsetReadMostly-------------------------------------> one holder: the preferred location, else the first (CPU)
D    --eviction of G's copy--------------------------------> C
any  --munmap, process exit---------------------------------> killed (no data migrated)
```

`[src@580]` for the cells, via the rules in §3.1 and the permission rules in §3.2. `[inf]` for the
two-processor reduction itself.

---

## 3. The decision functions

### 3.1 Where the page goes: `block_select_processor_residency`

The function (`uvm_va_block.c:11612-11764`) is called per page by every fault path and by the
access-counter path, through `uvm_va_block_select_residency` (`:11814-11857`). It returns at the
first rule that applies:

| # | condition | result | lines |
|---|---|---|---|
| 1 | `uvm_fault_force_sysmem` (honoured only with `uvm_enable_builtin_tests`, `:157-161`), or an HMM page that cannot migrate or must use sysmem | CPU | `:11632-11637` |
| 2 | read duplication allowed (`can_read_duplicate`, `:11554-11568`) **and** the access is read or prefetch. Access counters use PREFETCH, so they read-duplicate too | the faulting processor, as a **copy** | `:11639-11649` |
| 3 | read duplication allowed but the access is write or atomic | the faulting processor (collapse) | `:11651-11657` |
| 4 | the faulting processor is the preferred location | the faulting processor | `:11661-11669` |
| 5 | an HMM CPU fault | CPU | `:11674-11675` |
| 6 | the thrashing hint is PIN | `pin.residency` | `:11677-11681` |
| 7 | resident nowhere (first touch) | the preferred location if the faulter can access it, else the faulter | `:11683-11698` |
| 8 | the faulter is in accessed-by, can access the current copy, and this is not an access-counter service | stay (remote mapping) | `:11700-11716` |
| 9 | `uvm_perf_map_remote_on_native_atomics_fault` (default **0**), a GPU atomic, residency not the CPU, native atomics to it | stay | `:11579-11607`, `:11720-11721` |
| 10 | a preferred location is set, the faulter can access it, and the page is not on the faulter | the preferred location | `:11723-11729` |
| 11 | a CPU fault from an mm other than the block's (ptrace, NIC), and the CPU can access the copy | stay | `:11731-11757` |
| 12 | otherwise | the faulting processor (migrate) | `:11763` |

Then: if the chosen processor has no memory, the result is the CPU (`:11845-11848`). The CPU NUMA
node is picked separately (`block_select_node_residency`, `:11766-11812`).

Read rule 10 with §2.5 in mind. On x86 PCIe a GPU preferred location is not accessible from the CPU,
so **a CPU fault on a page whose preferred location is the GPU still migrates the page to the CPU**
(rule 12). `[src@580]`

### 3.2 What permission the faulter gets, and what everyone else keeps

- **The faulter.** `compute_new_permission` (`uvm_va_block.c:11054-11111`) starts from the access
  type: atomic strong maps to RWA; atomic weak and write map to RW; read and prefetch map to RO
  (`uvm_hal_types.h:563-579`). Then the *query_promote* policy (`uvm_va_block.c:11066-11068`) applies:
  - A read is promoted to RW when no read-duplication is possible and no other faultable processor
    holds an atomic mapping without native atomics (`:11074-11093`).
  - RW is promoted to RWA when the faulter has native atomics to the new residency (`:11095-11098`).
  - Discarded pages stay RO on reads (`:11100-11107`).

  So on x86 PCIe a GPU read fault on a GPU-resident page maps **RWA**. A GPU fault served remotely
  from sysmem maps **RW** (ATOMIC_DISABLE set), unless the access was itself an atomic.
- **Everyone else, when residency did not change.** `block_service_finish_revoke_prot`
  (`uvm_va_block.c:12081-12174`) revokes the other processors' permissions:
  - a CPU fault strips their atomics (`:12139`);
  - a GPU write strips their atomics;
  - a GPU atomic strips their writes (`:12142`);
  - processors with native atomics to the copy are exempt (`:12127-12131`).

  This is how the C+G atomic ping-pong in §2.7 arises. When residency *did* change, the move already
  unmapped everyone (invariant 4).
- **Who else is mapped after a move.** `uvm_va_block_add_mappings_after_migration`
  (`uvm_va_block.c:11211-11360`) maps the accessed-by processors and any thrashing processors:
  - only those that can access the new copy (`:11281-11284`);
  - never the preferred location as a remote mapper (`:11289-11294`);
  - never for read-duplicated pages (`:11244-11262`).

  `do_block_add_mappings_after_migration` (`uvm_va_block.c:11113-11209`) caps a remote atomic mapping at RW
  (`:11172`).
- **The ceiling for any mapping** is `uvm_va_block_page_compute_highest_permission`
  (`uvm_va_block.c:11368-11476`). It is used by policy calls and migrations, and it encodes invariants 1-3 and 5.
  `[src@580]`

### 3.3 The service sequence one fault runs

`uvm_va_block_service_locked` (`uvm_va_block.c:12429-12481`) does the following:

1. It computes the density prefetch hint (`:11880-11933`; §4.4).
2. For each new residency it runs `uvm_va_block_service_copy` (`:11935-12079`):
   - read-duplicate copies through `uvm_va_block_make_resident_read_duplicate` (`:5185-5362`);
   - moves through `uvm_va_block_make_resident_copy` (`:4878-4984`), which unmaps, populates,
     zeroes and copies;
   - if the destination is the CPU, it waits for the GPU work so it can check ECC (`:12019-12029`).
3. It then runs `uvm_va_block_service_finish` (`:12332-12427`):
   - commit residency (`:12347`);
   - compute permissions (`:12368`);
   - revoke (`:12382`);
   - map the faulter (`:12387`); for a managed CPU fault this first unmaps the CPU PTE and maps it
     again (`:12191-12210`);
   - map the accessed-by processors (`:12393`);
   - clear the discard bit for pages mapped writable (`:12403-12410`).

Every GPU-side step is a push on one of UVM's own CE channels (§5). `[src@580]`

---

## 4. Transitions

Each row is trigger → decision (and its inputs) → actions → resulting state.

### 4.1 GPU replayable fault (read, write, atomic, prefetch)

**Pipeline.** The interrupt reaches UVM's top half through nvidia.ko's ISR
(`kernel-open/nvidia/nv.c:2858`; `uvm_gpu_isr.c:218-280`). The top half then:

- checks for pending work (`uvm_gpu_replayable_faults.c:318-340`);
- disables the interrupt and queues **one** bottom half per GPU on a kthread (`uvm_gpu_isr.c:86-119`,
  `:614-616`).

`uvm_parent_gpu_service_replayable_faults` (`uvm_gpu_replayable_faults.c:2906-3028`) then works
through these steps:

1. **Fetch** up to 256 packets. Each packet is spun on until its VALID bit is set, and GET is written
   once at the end (`:844-987`).
2. **Coalesce** packets with the same instance pointer and page. The most intrusive access type wins:
   atomic > write > read > prefetch (`:753-842`).
3. **Translate** each instance pointer and VEID to a VA space (`uvm_gpu_replayable_faults.c:1028-1121`;
   `uvm_gpu.c:3534-3594`). Then sort the packets (`uvm_gpu_replayable_faults.c:1134-1178`).
4. **Dispatch per VA space** (`uvm_gpu_replayable_faults.c:2232-2377`), under the mmap and VA-space *read* locks (`:2286-2289`):
   - a managed range or an HMM block goes to `service_fault_batch_block`;
   - an ATS-capable GPU with no range goes to `service_fault_batch_ats`;
   - anything else is fatal (`:1946-2040`).
5. **Per block** (`service_fault_batch_block_locked`, `:1375-1597`), for each page:
   - check logical permissions (`:1493`);
   - skip the page if the GPU is already authorized (`:1518-1522`);
   - read the thrashing hint (`:1524`): THROTTLE means the page is not serviced (`:1528-1539`), PIN
     goes into the pin mask;
   - select the residency (`:1549`);
   - then service the whole block (`:1585-1586`, §3.3).
6. **Replay** under the default `BATCH_FLUSH` policy (`:80-84`). Flush the buffer, push a `START`
   replay and **wait for it** (`:2986-3006`). The replay push acquires the batch tracker, so it runs
   after that batch's copies, PTE writes and TLB invalidates (`:503-520`).
7. **Bound the pass.** It stops after 20 batches, or after 5 batches with throttled faults
   (`:103-113`). At least one replay is always issued (`:3017-3022`).

**Outcomes by access type** are the matrix in §2.7. Prefetch faults are handled like reads
(`uvm_hal_types.h:563-579`). A prefetch fault that fails its permission check is **never cancelled**;
it disappears on replay (`uvm_gpu_replayable_faults.c:1316-1325`). UVM writes the hardware
fault-on-prefetch filter itself:

- `MMU_PAGE_FAULT_CTRL.PRF_FILTER` through an RM-provided BAR0 pointer (`uvm_pascal_mmu.c:377-403`);
- enabled at init (`uvm_gpu_replayable_faults.c:216`);
- disabled while more than two thirds of a batch is invalid prefetches, and re-enabled after 1000 ms
  (`:65-70`, `:2877-2904`).

**Fatal faults** come from four sources:

- a hardware fault type at or above `FATAL` (`:922`; list in `uvm_hal_types.h:330-345`);
- no range, no HMM block and no ATS (`uvm_gpu_replayable_faults.c:2023`);
- a zombie range (`uvm_va_block.c:12503`);
- a non-migratable range-group page the GPU cannot map (`:12517-12535`).

Fatal faults are cancelled precisely. The sequence is:

1. flush the buffer, push a START replay and re-fetch;
2. service the non-fatal faults;
3. send one `CANCEL_VA_GLOBAL` per 4 KiB page, with an access-type mode
   (`uvm_gpu_replayable_faults.c:438-501`, `:2042-2230`).

The mode is WRITE_AND_ATOMIC when reads are allowed, so a page's pending reads still complete
(`uvm_gpu_replayable_faults.c:1330-1350`). Any *global* error (OOM after eviction, ECC) cancels **every** fault in the batch,
across VA spaces (`uvm_gpu_replayable_faults.c:2583-2655`, `:2960-2969`). The resulting RC is RM's; UVM's only own signal is a
tools event (`uvm_tools.c:1661-1687`).

The VMA's own protection (`mprotect`) is checked on GPU faults only for HMM blocks or under builtin
tests (`uvm_va_block.c:12511-12514`). For managed ranges, a GPU fault does not see `mprotect`.
`[src@580]`

### 4.2 Non-replayable fault (CE or PBDMA)

| trigger | decision | actions | result |
|---|---|---|---|
| A CE or PBDMA access misses a translation. *"Fault and switch"*: the whole TSG is preempted while other TSGs keep running (`uvm_gpu_non_replayable_faults.c:64-77`). GSP-RM copies the packet into the shadow buffer and sends `MMU_FAULT_QUEUED` (§5) | one fault at a time (`:793-801`); the same `uvm_va_block_select_residency` with the thrashing hint forced to NONE (`:399-400`, `:415-439`); a single-page region (`:437`) | the same service sequence as §3.3, then **clear faulted** (`:332-342`). On Ampere with neither SR-IOV nor CC this writes the channel's CHRAM `RESET_ENG/PBDMA_FAULTED`, then `NV_RUNLIST_INTERNAL_DOORBELL` (`uvm_ampere_host.c:108-147`). The C076 `CLEAR_FAULTED` SW method is used only under SR-IOV or CC (`uvm_gpu_non_replayable_faults.c:239-260`) | as for a replayable fault. A fatal fault goes to `nvUvmInterfaceReportNonReplayableFault` and RM tears the channel down (`:483-500`) |

`[src@580]`

### 4.3 CPU fault (read, write)

| trigger | decision | actions | result |
|---|---|---|---|
| `vm_ops->fault` or `->page_mkwrite` on the managed vma (`uvm.c:558-575`) → `uvm_va_space_cpu_fault` (`uvm_va_space.c:2531-2739`). A CPU write is treated as ATOMIC_STRONG, *"we cannot differentiate CPU writes from atomics"* (`uvm_va_block.c:12917-12921`, `:11592-11596`) | logical permissions plus range-group migratability (`:12834-12840`). The 300 µs race window may ignore the fault (`:12847`). THROTTLE returns a wake-up time (`:12852-12856`), then `usleep_range` with the VA-space lock dropped, in a loop (`uvm_va_space.c:2596-2625`). Otherwise rules 1-12 for one page (`uvm_va_block.c:12869-12898`) | the service sequence of §3.3. A move off the GPU is a CE copy on UVM's GPU-to-CPU channel, and the faulting thread waits for it (`:12019-12029`). The CPU PTE is installed with `vm_insert_page` under the block lock (`:12925-12937`) | C (C+G if G is in accessed-by), or D under ReadMostly. `SIGBUS` on a fatal status, `VM_FAULT_OOM` on no memory (`uvm_va_space.c:2729-2737`). A disabled (forked or moved) vma always gets `SIGBUS` (`uvm.c:328-342`) |

`[src@580]`

### 4.4 Fault-driven density prefetch

| trigger | decision | actions | result |
|---|---|---|---|
| Any fault service, replayable, non-replayable, CPU or access-counter, with **exactly one** destination processor (`uvm_va_block.c:11889`, `:11931`) | `uvm_perf_prefetch_get_hint_va_block`: a bitmap tree over the block whose leaves are the pages resident on the destination plus the faulted pages; the largest subtree above **51 %** occupancy wins (`uvm_perf_prefetch.c:42-48`, `:110-120`). Big pages holding a fault are filled whole (`:148-220`). First touch at the preferred location prefetches the whole region (`:361-366`). Thrashing pages are excluded (`:406-408`). Needs at least `min_faults` = 1 (`:50-56`, `:477-478`) | the extra pages join the new-residency mask with access type PREFETCH (`uvm_va_block.c:11914`). They migrate, and they are mapped on the faulter with the §3.2 permission | the neighbours move with the faulted page. **Not used by `UVM_MIGRATE`** (`uvm_migrate.c:663`). ATS uses its own variant (`uvm_perf_prefetch.c:425-445`) |

`[src@580]`

### 4.5 Explicit migration: `UVM_MIGRATE` (cudaMemPrefetchAsync), `UVM_MIGRATE_RANGE_GROUP`

| trigger | decision | actions | result |
|---|---|---|---|
| `UVM_MIGRATE {base, length, destination, flags, semaphore, cpuNumaNode}` (`uvm_ioctl.h:772-785`; `uvm_api_migrate`, `uvm_migrate.c:878-1101`) | the destination is given; rules 1-12 are not used. ReadMostly read-duplicates to the destination; otherwise it moves (`uvm_va_block_migrate_locked`, `uvm_migrate.c:196-284`). A non-migratable range group is skipped (`:582-590`) | multi-block: pass 1 MAKE_RESIDENT, pass 2 MAKE_RESIDENT_AND_MAP (`:669-724`). Mapping is on unless a test flag says otherwise (`:690`). Pages mapped nowhere get RWA on the destination (`:74-102`). Accessed-by processors are mapped (`:130-141`); already-mapped pages get the highest permission that needs no revocation (`uvm_va_block.c:8773-8817`). An async semaphore is released by a pushed CE/host write or eagerly from the CPU (`uvm_migrate.c:807-845`). No prefetch heuristics and no thrashing input (`:663`) | C, G or D at the destination. The discard bit clears (`uvm_va_block.c:5063-5071`). **No fault.** `UVM_MIGRATE_RANGE_GROUP` runs the same for each range of a group (`uvm_migrate.c:1103-1210`) |

`[src@580]`

### 4.6 cudaMemAdvise and its unset

| trigger | immediate action on existing pages | later effect | source |
|---|---|---|---|
| **SetPreferredLocation(P)** | no data moves. Pages P maps remotely but that are not resident on P are unmapped from P, unless the range is read-duplicated. UVM-Lite GPUs are re-mapped. Non-migratable range groups are migrated now | rules 4, 7 and 10 send faults to P when the faulter can access P; the PIN target prefers P (`uvm_perf_thrashing.c:1503-1514`) | `uvm_policy.c:175-236`, `:322-454`; `uvm_va_range.c:1654-1812` |
| **UnsetPreferredLocation** | the same call with INVALID; no data moves | rule 12 decides | `uvm_policy.c:456-500` |
| **SetAccessedBy(P)** | **maps P now** on every resident page (minus discarded and read-dup pages) at the highest permission: a remote mapping with no residency change (enters C+G) | after every move, P is mapped again (§3.2); rule 8 keeps the page in place on P's faults | `uvm_policy.c:514-563`, `:580-683`; `uvm_va_block.c:11478-11550` |
| **UnsetAccessedBy(P)** | clears the bit only. **Existing remote mappings stay**, except for UVM-Lite GPUs | the next move does not re-map P | `uvm_va_range.c:1878-1907` |
| **SetReadMostly** | per resident holder: unmap everyone else and revoke the holder to RO (`block_prep_read_duplicate_mapping`). **No copies are made now** | rule 2: read faults copy, writes collapse | `uvm_policy.c:699-728`, `:863-947`; `uvm_va_block.c:5141-5183` |
| **UnsetReadMostly** | **collapses now**. The preferred location wins if it holds a copy; otherwise the first holder in mask order (CPU first). No copy is made. Accessed-by is re-mapped | rule 12 decides | `uvm_policy.c:748-828` |

ATS ranges ignore these calls (`uvm_policy.c:400-405`, `:624-627`, `:882-885`). HMM read
duplication is a stub (`uvm_hmm.h:207-220`). `[src@580]`

### 4.7 Thrashing: PIN, THROTTLE, unpin

| trigger | decision | actions | result |
|---|---|---|---|
| A page sees **3** migrations or revocations (`_threshold`), each within **500 µs** (`_lapse_usec`) of the last. Counted causes: replayable fault, access counter, prefetch; MOVE only; not ReadMostly ranges (`uvm_perf_thrashing.c:1235-1387`, defaults `:242-305`) | `uvm_perf_thrashing_get_hint` (`:1628-1793`). **THROTTLE** for a processor still inside its 500 µs window (`_nap`), and for anyone but the exempt processor while unpinned. **PIN** after **10** throttle periods (`_pin_threshold`), or at once when every thrashing processor can access the preferred location. Target order: the preferred location; a fast-link + native-atomic common location; otherwise the requester (`:1466-1608`) | THROTTLE: GPU faults on the page are left unserviced and re-raised by each replay (`uvm_gpu_replayable_faults.c:1528-1539`); a CPU fault sleeps (§4.3). PIN: rule 6 fixes the residency; the faulter and the other thrashing processors get remote mappings with cause `Thrashing` (`uvm_va_block.c:12216-12243`, `:12284-12310`) | pinned state for **150 ms** (`_pin` = 300 lapses). On x86 PCIe the CPU cannot map vidmem, so CPU↔GPU ping-pong settles at a **CPU-resident page, GPU-remote**: C+G pinned (`[inf]` from `uvm_perf_thrashing.c:1559-1597` and §2.5) |
| pin expiry | a delayed work item (`uvm_perf_thrashing.c:931-958`, `:1836-1919`) | unmaps the pinned pages from every mapper that is neither accessed-by nor holds the copy (`:1102-1173`), then resets the page | C+G becomes C. The next GPU access faults and migrates (rule 12) |
| any non-heuristic migration (API migrate, advise, range group), a COPY, or a ReadMostly policy | — | resets the page's thrashing state, which also unpins (`:1290-1297`, `:1065-1066`) | — |

Non-replayable faults never consult thrashing (`uvm_gpu_non_replayable_faults.c:399-400`). Eviction
neither counts nor unpins (`uvm_perf_thrashing.c:1263-1266`). If any simulated GPU is registered,
`lapse` becomes 400 ms and `pin` 10 lapses (`:573-591`; `isSimulated` comes from RM's
`GET_SIMULATION_INFO`, which CPU-RM answers NONE on silicon,
`src/nvidia/src/kernel/gpu/subdevice/subdevice_ctrl_gpu_kernel.c:1326-1347`). `[src@580]`

### 4.8 Access-counter notification

| trigger | decision | actions | result |
|---|---|---|---|
| The GPU's MIMC counter for a 2 MiB region reaches **256** remote accesses (`uvm_gpu_access_counters.c:41-44`). Only MIMC and only virtual (GVA) notifications exist in 580 (`src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:9481-9483`; `uvm_turing_access_counter_buffer.c:115-117`) | migration is enabled only if `is_migration_enabled()`. *"Migrations are disabled by default on all non-ATS systems"* (`uvm_gpu_access_counters.c:142-157`). With migration off, the counters are **not enabled** at GPU registration (`:476-485`; `uvm_va_space.c:783-789`). Even when forced on, a 4 KiB-page kernel returns before servicing any notification (`uvm_gpu_access_counters.c:1644-1651`, Bug 4299018). Where it runs: rules 1-12 with processor = the notifying GPU and access = PREFETCH (`:1172-1181`). Accessed-by does not hold the page (rule 8 excludes counters); THROTTLE skips; PIN redirects (`:1142-1157`) | expand to the closest copy that is not on this GPU (`:1287-1354`), then the service sequence; then a targeted or all clear through `MEM_OP ACCESS_COUNTER_CLR` (`uvm_turing_host.c:365-384`). HMM blocks are skipped (`uvm_gpu_access_counters.c:1600-1609`) | C+G becomes G, or D under ReadMostly. **On x86 PCIe: never.** On ATS systems (where counters default on) it is the main migration driver (§6) |

`[src@580]`

### 4.9 Eviction under vidmem oversubscription

| trigger | decision | actions | result |
|---|---|---|---|
| A GPU chunk allocation fails inside any service or migration. UVM drops the block lock, retries with EVICT, and restarts the operation (`uvm_va_block.c:1978-2027`). The other trigger: RM-side PMA clients (cudaMalloc) through the eviction callbacks (`uvm_pmm_gpu.c:115-125`). Eviction exists only on GPUs with replayable-fault support (`uvm_gpu.h:1816-1820`) | `pick_root_chunk_to_evict` (`uvm_pmm_gpu.c:1460-1506`) takes, in order: free chunks, chunks with no residency, discarded chunks, then the head of the used list. The list is LRU **by allocation and residency, not by access**: *"TODO: Bug 1765193: Move the chunks to the tail of the used list whenever they get mapped"* (`:1493-1494`). Pinned and in-flight chunks are excluded (`:1389-1408`) | per owning block, `uvm_va_block_evict_chunks` (`uvm_va_block.c:13331-13507`) calls `make_resident(CPU, CAUSE_EVICTION)`. That unmaps every mapper, breaks read-duplication, copies with CE and sets `evicted` (`:5028-5047`). Re-mapping is deferred to a work item: accessed-by processors, plus the evicting GPU **only if** access-counter migrations are on (`:13227-13324`, `:163-167`) | C with G's evicted bit. On x86 PCIe G is unmapped, so its next access faults and migrates back. If eviction cannot free memory, the batch is cancelled: **there is no fallback to sysmem** (`uvm_pmm_gpu.c:1522-1534`; `uvm_gpu_replayable_faults.c:2960-2969`) |

`[src@580]`

### 4.10 Discard (`UVM_DISCARD`, ioctl 80)

| trigger | actions | result |
|---|---|---|
| `uvm_api_discard` (`uvm_policy.c:1059-1130`), refused while any non-fault-capable GPU is registered (`:1083-1087`); ATS and HMM are no-ops | `va_block_discard_locked` (`uvm_va_block.c:12551-12719`) sets `discarded_pages`, collapses read-duplicates, and with `UNMAP` unmaps everyone and **revokes residency**. A fully discarded 2 MiB block hands its root chunk to the PMM's discarded list | discarded overlay. Later: not copied on migration (`:4680-4685`); re-zeroed on population (`:2834-2835`, `:2919-2931`); read faults map RO (`:11100-11107`). The bit clears on a writable mapping (`:12403-12410`) or an API migration (`:5063-5071`) |

The semantics are documented in `uvm.h:2680-2770`. Whether libcuda issues this ioctl cannot be
determined from this tree. `[src@580]`

### 4.11 Range groups (migration prevented)

| trigger | actions | result |
|---|---|---|
| `UVM_PREVENT_MIGRATION_RANGE_GROUPS` (`uvm_range_group.c:470-528`). It needs a preferred location that is not a fault-capable GPU (`:427-439`); on fault-capable x86 that means the CPU | `uvm_range_group_va_range_migrate_block_locked` (`:175-252`) unmaps the CPU, makes the range resident at the preferred location, and maps UVM-Lite and accessed-by GPUs | while prevented, *"CPU accesses are always fatal"* (`uvm_va_block.c:12517-12535`), and the CPU is never mapped (`:11319-11343`). Allow (`uvm_range_group.c:494-510`) flips the flag only, and the CPU then faults normally. That CUDA uses this for stream-attached memory on systems without concurrent managed access is `[inf]` |

`[src@580]`

### 4.12 Lifecycle: create, munmap, free, unregister, fork, mremap, mprotect, exit

| trigger | actions | result |
|---|---|---|
| `mmap` of `/dev/nvidia-uvm` (`uvm.c:790-835`: `MAP_SHARED`, RW, offset == VA, `VM_MIXEDMAP\|VM_DONTEXPAND\|VM_DONTCOPY`) | `uvm_va_range_create_mmap` (`uvm_va_range.c:209-242`) with the default policy; no blocks | U |
| first touch | population zeroes the page: CPU with `__GFP_ZERO` when not every page is resident (`uvm_va_block.c:1936-1942`; `uvm_pmm_sysmem.c:464-478`); GPU with a CE `memset` on the `GPU_INTERNAL` channel unless the PMA chunk is already zero (`uvm_va_block.c:2787-2895`). Residency is set without a copy (`:4553-4591`) | C or G |
| `munmap`, partial or whole | `uvm_vm_close_managed` (`uvm.c:510-551`) leads to `uvm_va_block_kill` (`uvm_va_block.c:9666-9774`): unmap all processors (PTE writes and TLB invalidates), free GPU chunks, wait for the tracker, free CPU chunks. **No data is migrated.** A split goes through `uvm_vm_open_managed` and `uvm_va_range_split` (`uvm.c:460-489`) | killed |
| `UVM_FREE` | not for managed ranges: *"VA ranges created by mmap ... go through munmap"* (`uvm_va_range.c:661-735`) | — |
| `UVM_UNREGISTER_GPU` | resets a preferred location of this GPU and drops it from accessed-by (`uvm_va_range.c:1136-1150`); **migrates GPU-resident pages to the CPU** with mapping (`uvm_va_block.c:9549-9584`) | C |
| fork or mremap-move | *"On fork or move we want to simply disable the new vma"* (`uvm.c:444-448`). `uvm_disable_vma` unmaps by file offset, which also zaps the parent's CPU PTEs (`:344-369`) | the child's vma always gets `SIGBUS`. The parent re-faults its CPU mappings |
| `mprotect` | no hook beyond the vma split. It acts through `compute_logical_prot` (`uvm_va_block.c:8170-8202`): checked when mapping the CPU (`:8282-8285`), and **not** on managed GPU faults (§4.1) | — |
| `MADV_DONTNEED` and other kernel PTE zaps | UVM is not told (`uvm_va_block.h:395-400`) | the next CPU access re-faults; the race window of §2.3 tells a real downgrade from a multi-thread race |
| process exit | channels stop first. In multi-process sharing mode the ranges become **zombies** (`uvm.c:518-541`; `uvm_va_range.c:622-630`) | killed or zombie |
| VA space destroy | ranges are destroyed **before** GPUs are unregistered (`uvm_va_space.c:463-517`) | GPU-resident data is dropped, not migrated |

`[src@580]`

---

## 5. Who does what

| actor | does | does not |
|---|---|---|
| **GMMU** (hardware) | translates. On a missing or insufficient PTE it writes a 32-byte packet: to the **replayable** buffer for GR/SM clients (*"fault and stall"*: the access waits for replay or cancel and other channels stall, `uvm_gpu_non_replayable_faults.c:78-81`), or to the **non-replayable** buffer for CE and PBDMA (*"fault and switch"*: only the TSG is preempted, `:64-77`). Counts remote accesses (MIMC) when enabled. Applies the prefetch-fault filter | decides placement; migrates |
| **Copy engine** | runs every UVM copy (CPU↔GPU, staged GPU↔GPU), the zero-on-populate memsets, and **the PTE writes**: `uvm_pte_batch` uses `memset_8` for up to 4 PTEs and inline copies beyond that (`uvm_pte_batch.h:31-43`). Page-table pushes use `GPU_INTERNAL` (`uvm_mmu.c:46-67`), and these channel types are CE-backed (`uvm_channel.h:84-96`). CPU writes to page tables happen only at coherent-Hopper bootstrap (`uvm_mmu.c:268-275`) | — |
| **Host methods** on UVM's MEMOPS channel | `MEM_OP` TLB invalidate (`uvm_tlb_batch.c:70-86`); replay `START` / `START_ACK_ALL` against a dummy PDB (`uvm_volta_host.c:234-264`); `CANCEL_VA_GLOBAL` (`:47-114`); access-counter clear (`uvm_turing_host.c:365-384`). The C076 `FAULT_CANCEL` and SR-IOV/CC `CLEAR_FAULTED` SW methods (`uvm_pascal_host.c:332-364`; `uvm_ampere_host.c:190-210`) are executed by RM; the executor is not in this tree | — |
| **GSP-RM** (firmware; only its plumbing is visible) | programs the replayable buffer from a PTE array CPU-RM sends (`src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1228-1261`). Owns the non-replayable hardware buffer, copies "designated" CE/PBDMA faults into UVM's shadow buffer and raises `MMU_FAULT_QUEUED` (`src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:1040-1056`). The monolithic rule it mirrors: UVM gets a non-replayable fault only with a client buffer, `REPLAYABLE_FAULT_EN` set, and a CE or PBDMA source (`src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:716-744`). RCs a channel on `REPORT_NON_REPLAYABLE_FAULT` (route-to-physical) | reads replayable faults when CC is off: *"A Replayable fault is never RM servicable"* (`src/nvidia/src/kernel/gpu/mmu/arch/volta/kern_gmmu_gv100.c:1050-1058`). Decides residency |
| **CPU-RM** (nvidia.ko) | allocates the replayable buffer in sysmem, single-instance (`src/nvidia/src/kernel/gpu/mmu/kern_gmmu.c:1192-1291`, `:1213-1216`); allocates the shadow buffer (`:1757-1818`) and the access-counter buffer. Maps GET/PUT, interrupt and `MMU_PAGE_FAULT_CTRL` registers for UVM (`src/nvidia/src/kernel/gpu/mmu/arch/turing/kern_gmmu_tu102.c:188-231`). Hands the replayable interrupt to UVM (`MC_CHANGE_REPLAYABLE_FAULT_OWNERSHIP`, `src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:2381-2393`). Programs access counters by BAR0 writes, not GSP (`src/nvidia/src/kernel/gpu/uvm/access_cntr_buffer_ctrl.c:84-115`). Supplies channel facts, including the CHRAM register (`uvm_user_channel.c:160-176`) | services replayable faults |
| **nvidia-uvm** | everything in §3-§4: fetch, coalesce, attribute, decide, allocate, zero, copy, map, revoke, invalidate, replay, cancel, clear faulted, thrashing, prefetch, eviction | — |
| **Linux mm** | the vma lifecycle through `vm_ops` (§4.12); CPU PTEs through `vm_insert_page` and `unmap_mapping_range` (`uvm_va_block.c:8248-8330`, `:6014-6060`); may zap managed CPU PTEs silently. For HMM it owns the CPU pages, `migrate_vma` and the mmu notifiers (§6) | — |
| **libcuda** (inferred from the ioctls only) | creates managed ranges (`mmap`), sets policy (`UVM_SET_PREFERRED_LOCATION`, `UVM_SET_ACCESSED_BY`, `UVM_ENABLE_READ_DUPLICATION`), prefetches (`UVM_MIGRATE` with a semaphore), registers GPUs, VA spaces and channels. It never sees a fault; it learns of a fatal one through RM's RC | — |

**Searched for, and absent:** any RM or GSP path that decides residency or switches between
remote-mapped and migrate modes. Searched `src/nvidia/{src,inc,arch}` and `kernel-open/nvidia` for
`thrash`, `read.dup`, `preferred.loc`, `accessed.by`, `uvm_perf`, `migrat`, `prefetch` and
access-counter consumers. The GSP→CPU event list (`src/nvidia/src/kernel/gpu/gsp/kernel_gsp.c:1486-1611`) carries only
`MMU_FAULT_QUEUED` and `RC_TRIGGERED` for UVM. RM consumes no access-counter notification beyond
`notifyEvents` (`src/nvidia/src/kernel/gpu/uvm/arch/turing/uvm_tu102.c:426-470`). Firmware internals
are not visible. No plumbing exists through which they could tell UVM where a page goes.
`[src@580]`

---

## 6. Platform branches

| platform | what changes in the machine | source |
|---|---|---|
| **x86 PCIe** (primary) | The GPU reaches sysmem uncached, with no native atomics in either direction; the CPU never reaches vidmem (§2.5). Access counters are off (§4.8), so evicted pages are not remote-mapped (§4.9). Thrashing PIN settles at C+G. HMM is on by default on x86_64 if the kernel supports it (`uvm_hmm.c:28-35`) | §2.5, §4.7-§4.9 |
| **Confidential Computing** | GPUs cannot access sysmem, so there is no C+G and everything migrates (`uvm_va_space.c:833-842`). GSP-RM copies encrypted replayable packets into a shadow buffer (`src/nvidia/src/kernel/rmapi/nv_gpu_ops.c:9211-9214`). Clear faulted and the prefetch toggle go through RM (`uvm_gpu_non_replayable_faults.c:250-254`; `uvm_pascal_mmu.c:377-403`) | — |
| **Coherent C2C** (Grace Hopper/Blackwell) | GPU native atomics on CPU memory. With GPU memory NUMA-onlined, the CPU can access and map vidmem through its `struct page`s (`uvm_va_space.c:877-887`; `uvm_va_block.c:8205-8224`), so **G with a CPU mapping exists**: via accessed-by CPU (rule 8), the remote-mm rule 11, or a PIN. Sysmem may be GPU-cached; EGM aperture | `uvm_va_block.c:127-155` |
| **ATS** (Grace, P9; `pci_ats_supported`) | For pageable memory the GPU walks the CPU page tables, with no GPU PTEs (`uvm_mmu.h:121-127`). A fault goes to `uvm_ats_service_faults`, which populates or migrates through `uvm_migrate_pageable`; already-mapped pages are served remotely (`uvm_ats_faults.c:46-118`, `:577-683`). **Access counters default on** (`uvm_gpu_access_counters.c:150-154`) and drive the migration | — |
| **HMM** (system-allocated memory, x86 included) | CPU residency is ordinary anonymous Linux pages; GPU residency is `MEMORY_DEVICE_PRIVATE` pages (`uvm_pmm_gpu.c:3240-3272`). A CPU touch of a GPU-resident page goes through `migrate_to_ram` and always returns the page to the CPU (`uvm_va_block.c:11674-11675`). No read duplication. File-backed, shared, hugetlb and `VM_SPECIAL` vmas must stay in sysmem: rule 1, remote-mapped (`uvm_hmm.c:3955-3977`). GPU atomics use `make_device_exclusive`, and they are fatal on shared or hugetlb vmas (`:2593-2665`). Invalidation comes from mmu notifiers (`:428-530`). Disabled per VA space by `DISABLE_HMM` or multi-process sharing (`:141-166`) | — |
| **Integrated GPU** | every processor is always mapped, because memory is only sysmem (`uvm_va_block.c:11276-11279`) | — |

---

## 7. The owner's five hypotheses: verdicts

| # | hypothesis | verdict | the code |
|---|---|---|---|
| H1 | GPU-resident and authoritative; the CPU traps, and a CPU access migrates | **confirmed** for x86 PCIe | A G page is never CPU-mapped (§2.5). A CPU fault resolves by rule 12 to a migration to the CPU, or by rule 2 to a read-duplicate copy under ReadMostly. A GPU preferred location does not stop it: rule 10 needs the CPU to reach the GPU, and on PCIe it cannot. The fault is synchronous in the faulting thread (§4.3) `[src@580]` |
| H2 | CPU-resident and authoritative; the GPU traps, and a GPU fault migrates | **confirmed as the default** (rule 12) | Five exceptions keep the page on the CPU and map the GPU remotely: preferred location CPU (rule 10), accessed-by G (rule 8), a thrashing PIN to the CPU (rule 6), an HMM sysmem-only vma (rule 1), and a range group with migration prevented. ReadMostly copies instead of moving (rule 2) `[src@580]` |
| H3 | Both hold read-only copies; a write trap sets the authoritative owner without migrating | **partly** | Read-only copies, yes: under ReadMostly every mapping is RO and local (invariant 1). A write *collapses* the set, not just retags it: the writer becomes the only holder, and every other copy is unmapped and loses residency (`uvm_va_block.c:4938-4946`, `:4510-4551`). No data moves **only if the writer already held a copy**; a writer without one gets the page copied (rule 3). The state exists only under an explicit ReadMostly policy, and only while no non-fault-capable GPU is registered (§2.4) `[src@580]` |
| H4 | CPU-resident; the GPU maps sysmem remotely (DMA), a compromise for interleaved access | **confirmed, and wider** | The state is C+G. It is entered by SetAccessedBy (mapped immediately), preferred-location CPU, the thrashing PIN (the automatic version of exactly this compromise: §4.7), HMM sysmem-only vmas, and range groups. Remote sysmem mappings are uncached (`VOL`). GPU atomics in this state ping-pong *permissions*: a GPU atomic revokes the CPU to RO, and a CPU write strips the GPU's atomic (§3.2). On x86 neither eviction nor access counters enter it (§4.8, §4.9) `[src@580]` |
| H5 | Deliberately absent: GPU-resident with the CPU reading vidmem over BAR/MMIO | **confirmed for x86 PCIe**; a platform property, not a UVM design choice | The CPU is in `accessible_from[GPU]` only on a coherent, NUMA-onlined GPU (`uvm_va_space.c:877-887`). The CPU map path asserts access (`uvm_va_block.c:8266`) and maps GPU pages only through NUMA `struct page`s (`:8205-8224`). No BAR1 path exists for managed memory. On C2C platforms the state does exist, over coherent C2C rather than BAR/MMIO (§6) `[src@580]` |
| — | a GSP-side optimization switches between DMA and trap modes | **refuted** | No RM or firmware path carries placement (§5). The switches that exist are all in nvidia-uvm on the CPU: the thrashing PIN (on by default, 150 ms pins); access-counter migration (default off on non-ATS, impossible on 4 KiB kernels); `map_remote_on_native_atomics_fault` (default 0); remote-on-eviction (gated on access-counter migration) `[src@580]` |

**Access-counter migration on non-ATS systems, specifically:**

- `uvm_perf_access_counter_migration_enable` defaults to `-1` (`uvm_gpu_access_counters.c:69`).
  `is_migration_enabled()` then returns `g_uvm_global.ats.supported` when ATS support is compiled in,
  and `false` otherwise (`:142-157`).
- Counters are enabled at registration only when "required" (`:476-485`), so on x86 the hardware
  counters are never switched on.
- Forcing the parameter to 1 still does nothing on x86_64. The service routine returns before
  migrating when `PAGE_SIZE == 4K` (`:1644-1651`), with the comment *"TODO: Bug 4299018 : Add support
  for virtual access counter migrations on 4K page sizes"*.

`[src@580]`

---

## 8. What this means for kayfabe

### 8.1 Transitions that hold the host GR engine while the guest services

Every transition triggered by a **GR replayable fault** holds GR: first touch (U to G or C+G), C to
G, C to D, D to G, the C+G atomic upgrade, and an RO-to-RW upgrade under ReadMostly. NVIDIA states
the stall as design: replayable faults *"block preemption of the channel until software (UVM)
services the fault … prevent the execution of other channels"* (`uvm_gpu_non_replayable_faults.c:78-81`).
That is the mechanism behind the ~4 s all-tenant stall the owner ruled unacceptable
(`docs/OWNER_RULINGS.md` §E). The stall comes from run `uvmg4` at `d178a737` (2026-09-30, RTX 3060,
branch `v3-uvm-guest`, `traces/v3_uvm_guest/README.md`), where the host's ctxsw watchdog bounded it.

Inside that hold, besides the guest-to-host round trip, stock UVM does:

- population, with the CE zeroing memset (§4.12), and the CE copy (§3.3);
- **synchronous eviction** of a 2 MiB root chunk when vidmem is full (§4.9);
- the prefetch-expanded neighbours (§4.4) and the `BATCH_FLUSH` replay wait (§4.1);
- for a fatal fault: a flush, a re-fetch and one precise cancel per 4 KiB page (§4.1).

**THROTTLE lengthens the hold on purpose.** A throttled page is not serviced; each replay re-raises
it, in 500 µs windows, for up to 5 throttled batches per bottom half (§4.7). `[inf]`, untested: if a
guest fault's round trip approaches the 500 µs lapse, CPU↔GPU ping-pong may stop registering as
thrashing. The PIN, the one automatic move to C+G, would then not engage, and every interleaved
access would become a full migration under a GR hold.

### 8.2 Transitions that never raise a GPU fault

`UVM_MIGRATE` (§4.5); every advise call (§4.6); discard, range groups, eviction, munmap and GPU
unregister (§4.9-§4.12); every **CPU** fault (§4.3), which is a guest-CPU trap with copies on guest
UVM's CE channels; **non-replayable** CE and PBDMA faults (§4.2), whose "fault and switch" stops only
the faulting TSG; and the steady states C+G and D, which run fault-free over DMA. Access-counter
migration does not exist on x86 (§4.8). None of these hold GR. They depend only on kf3's mapping plane
publishing each guest PTE change, permission bits included, at the guest's TLB invalidate. This
matches the 2026-09-26 bare-metal baseline, where the malloc, prefetch, advise, hostalloc and d2h
shapes took zero replayable faults and pass in kf3 (`V3_UVM_DEMAND_PAGING.md` §1.2).

### 8.3 Hardware primitives kayfabe must provide faithfully

1. **The fault packet and buffer protocol.** VALID; the instance pointer and VEID; the page address,
   access type, fault type (fatal at or above `FATAL`), client, GPC, uTLB and timestamp; GET/PUT with
   the overflow bit; the interrupt as a level from GET != PUT (`uvm_gpu_replayable_faults.c:844-987`;
   `uvm_hal_types.h:247-345`).
2. **Replay after publish.** `START` completes when the replays are in flight; `START_ACK_ALL` when
   every faulting access is translated or has faulted again (`uvm_hal_types.h:496-506`). The guest
   orders the replay after the batch's PTE writes and TLB invalidates
   (`uvm_gpu_replayable_faults.c:503-520`), so the host replay must follow the host publish: the
   existing commit-on-ack.
3. **Precise cancel.** One `CANCEL_VA_GLOBAL` per page with its access-type mode; a WRITE_AND_ATOMIC
   cancel must leave the page's reads alive (§4.1). The C076 targeted and global cancels cover stale
   channels.
4. **`READ_ONLY` and `ATOMIC_DISABLE` in the PTE.** `READ_ONLY` carries read duplication.
   `ATOMIC_DISABLE` is **the** mechanism of system-scope atomics over PCIe (invariant 3, §3.2). kf3
   drops it by default (`crates/kf-qemu/src/device.rs:288-300`), so a GPU atomic on a C+G page whose
   CPU copy is writable never faults, and the guest's exclusivity step (CPU to RO) never runs `[inf]`.
5. **The TLB invalidate as the commit point.** It is where guest UVM's CE-written PTEs become the
   state (invariant 6).
6. **The non-replayable resume.** The shadow buffer plus `MMU_FAULT_QUEUED`; and, on Ampere when the
   guest is neither SR-IOV nor CC, **the CHRAM `RESET_*_FAULTED` write plus the runlist
   `INTERNAL_DOORBELL` write** through BAR0 (`uvm_ampere_host.c:108-147`). The C076 `CLEAR_FAULTED`
   SW method in `V3_UVM_DEMAND_PAGING.md` §5 is the SR-IOV/CC variant only. kf3's CHRAM range is plain
   shadow today, latent as `V3_HW_BOUNDARY_INVENTORY.md` L11 records.
7. **The fault-on-prefetch filter** (`MMU_PAGE_FAULT_CTRL.PRF_FILTER`), which the guest toggles under
   load; invalid prefetch faults must vanish on replay, never cancel (§4.1).
8. **Answers that steer the guest's machine.** Silicon, not simulation (simulation stretches the
   thrashing timers ×800 and force-enables access counters, §4.7-§4.8); `atsSupported` false and no
   system-memory window (§6); the virtualization mode (item 6). No access-counter emulation is needed
   on x86: the guest never enables them.

---

## 9. What this note could not determine

1. GSP firmware's own code: the non-replayable copy and RC logic, and the executor of the C076 SW
   methods. Their behaviour is inferred from the monolithic RM code and CPU-RM's comments (§5).
2. The hardware semantics of MIMC beyond "remote accesses", and the fault buffer's default size, which
   is a hardware `SET_DEFAULT` read-back with no constant in source.
3. Which CUDA calls use range groups (§4.11) or `UVM_DISCARD` (§4.10). There is no libcuda source.
4. Every latency in §8.1 is a list of components, not a budget. No measurement was taken for this
   note, and the 500 µs thrashing-lapse interaction in particular is untested.
5. Whether a kf3 guest's RM reports virtualization mode NONE, which selects the CHRAM register path
   in §8.3 item 6. Expected `[inf]`; not checked in kf3's GSP answers.
