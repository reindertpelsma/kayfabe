# What kayfabe maps into a guest channel's GPU address space, and who can reach it

**STATUS: RESEARCH, 2026-10-10.** A security review of the source on
`claude/window-exposure-review-20261009`, which branches from `claude/tmode-pieces-20261009` at
`c6fff2e3`. It answers the owner's question of 2026-10-10 and checks every mapping against the
owner's ruling of the same day. No GPU was used. Each fact is tagged **[read]** (seen in source),
**[measured]** (in a committed trace), or **[inferred]**. Audit findings S1-21, S1-23 and S1-43
(`docs/audits/2026-10-03-v3-stage1.md`) and the fix design (`V3_P1P2_TSPACE.md`) cover this ground
already. This review confirms them against today's code, adds what they do not cover, and adds a
GPU-free test of the owner's invariant.

## 0. The owner's ruling (2026-10-10, binding), as checked here

> In a **Passthrough** space, nothing owned by kayfabe or the host may be mapped. Every VA mapping
> in a passthrough space comes from the guest's own page-table leaves, and from nothing else.
> The only future exception is the address-size fix for a sysmem USERD (Windows only, not yet
> implemented).

"Passthrough space" here means any host VA space in which a Passthrough channel *can* be born.
Today that includes every mirrored space on the default path; see §3.

## 1. Short answers

**"Translated channels can be unprivileged, right?"** Yes, and they are. Every host channel
kayfabe creates is checked to be USER: Translated rings, Passthrough twins, and the T-space
rings. A birth whose reply carries `PRIVILEGED_CHANNEL` or a non-USER `internalFlags.PRIVILEGE`
is refused **[read]** (`crates/kf-host/src/birth.rs:14`, `:187-192`;
`crates/kf-host/src/channel.rs:449-464`, `:586-650`). **But being unprivileged does not protect a
window.** A USER channel can read and write every non-privileged PTE in its own VA space. RM gives
an unprivileged client no way to create a privileged PTE (see §4). So "the channel is
unprivileged" protects host RM's own privileged buffers and refuses physical-mode copies. It does
nothing to hide kayfabe's windows from a guest that shares the space.

**"Is it safe to map the entire guest VRAM?"** Only in a space where no guest-written push buffer
ever runs: the T-space (`KF3_TSPACE=1`). In any space where a Passthrough channel runs it is not
safe. **The default build does exactly that today.** Every mirrored space, including a guest user
process's, gets a read-write, non-privileged map of the whole store and of all guest RAM
**[read + measured]**. Any process in the guest can then use the GPU to read and write guest-kernel
memory and other processes' memory. The damage stays inside the VM (§5, row 1). This is audit S1-21,
rated critical. It is fixed only behind `KF3_TSPACE=1`, which is off by default.

**"I hope you don't map anything of kayfabe internals in a passthrough space."** On the default
build, kayfabe does, in three ways:

- The store window covers the **whole** store, including kayfabe's firmware carve-out, where its
  BAR1/BAR2 root pages live.
- Guest-kernel spaces hold kayfabe's Translated rings: push buffer, GPFIFO, fence and USERD, all in
  one read-write map. On the default build a Passthrough channel can still be born next to them.
- A guest leaf that points into the carve-out is only *counted* on the default build, not refused.

With `KF3_TSPACE=1`, no mirror carries anything of kayfabe's **[read + unit-tested]**. What remains
in every space, whatever the build, is host RM's **own** context buffers and GSP's reserved range
(§5, rows 9-10). NVIDIA places these in every native process's address space too.

## 2. Two facts the rest depends on

- **The GPU can name every VA in the space.** VER2 page tables (Pascal through Ada) cover 49 bits,
  and VER3 (Hopper and Blackwell) covers 57 **[read]** (`ogkm-595.84
  kern_gmmu_fmt_gp10x.c:59-101`, `kern_gmmu_fmt_gh10x.c:63-114`). kayfabe creates its spaces with
  `vaSize = 0`, which means the maximum size **[read]** (`crates/kf-host/src/channel.rs:688-714`;
  `ogkm gpu_vaspace.c:1107-1121`). A CUDA kernel dereferences 64-bit pointers, and a copy engine
  takes 49+-bit virtual operands. So any placement in the space can be named by guest work running
  natively there **[inferred, from the formats; T-WINDOW-USER has never run]**. RM's declared
  `vaLimit` is not a hardware fence on Ampere and later: `NV_RAMIN_ADR_LIMIT` is written only on
  TU10x/TU11x **[read]** (`g_kern_gmmu_nvoc.c:454-461`, `kern_gmmu_gv100.c:279-288`). Placing
  something "above the limit" therefore means only "unmapped", which in turn means "not usable by
  Translated work either".
- **An unprivileged client cannot ask for a privileged PTE.** The PTE privilege bit
  (`NV_MMU_VER2_PTE_PRIVILEGE` 5:5; the PCF `PRIVILEGE_*` values on VER3) is set only from
  `MEMDESC_FLAGS_GPU_PRIVILEGED`, via `dmaMapBuffer`, which is RM's internal path **[read]**
  (`virt_mem_allocator_gm107.c:2179`, `:2405`, `:2460`, `:2802-2803`). The
  `NV_ESC_RM_MAP_MEMORY_DMA` path never sets `pLocals->priv` (`:358`, `:403`, `:1441`). NVOS46 has no
  privilege field (`nvos.h:1978-2153`), and `DMA_SET_PTE_INFO` forces REGULAR (`gpu_vaspace.c:2666`).
  kayfabe's `MapPerm` carries only read-only, atomic-disable and volatile (`crates/kf-host/src/channel.rs:1840-1854`).
  Even if a privileged window existed, Translated rings are USER channels too, so their own copies
  would fault on it. **"Map it privileged-only" is not available, and would not work for the one
  user that needs the windows.**

## 3. The kinds of host VA space, and which channels can run in each

| Space | Created by | Channels that can be born in it | What decides |
|---|---|---|---|
| **Mirror, guest user process** (default build) | `create_mirror` `mem.rs:2711`, or a reused spare `mem.rs:3192` | Passthrough (USER twin). **No Translated channel**, because the guest's Translated births come from the guest kernel | `chan.rs:5761-5779`: no gate on the default build |
| **Mirror, guest kernel** (default build): RM-internal clients, or a space where a Translated channel was born | same | Translated (ring in this space) **and** Passthrough. The default build refuses neither next to the other | `chan.rs:5984` (`force_kernel`, no refusal); `twin.rs:27` |
| **Prewarmed spare** (default build) | `prewarm` `mem.rs:2892` | becomes either kind of mirror on reuse | `apply_statement` `mem.rs:3170-3215` |
| **Retired spare** | `retire_mirror` `mem.rs:2991` | same; a space with live channels is never recycled | `mem.rs:3003-3015` |
| **Mirror, T-mode** (`KF3_TSPACE=1`) | `tmode_twin` / `tmode_spare` / `tmode_reuse` `mem.rs:2458-2513` | User space: Passthrough only. Kernel space: **none**. A Passthrough birth there is refused, and Translated work runs in the T-space | `twin.rs` state word; `chan.rs:5761-5775`, `:5940-5975` |
| **T-space** (`KF3_TSPACE=1`), one per VM | `TSpace::build` `tspace.rs:242-341` | Translated only. Its `VaSpace` is private and never enters the mirror table, so no Passthrough birth can name it | `tspace.rs:25-26`, `:181-199` |
| Walker / display libcuda contexts; `kf-harness` gate binaries | VMM / bench tools | VMM-authored kernels only; no guest channel | out of scope |

**[measured]** On the default build, a `privilege=0` Passthrough twin was born in a space whose log
line shows both windows and the ring region (`traces/v3_display/gop_box_20261003/run_b3_qemu.log:2747`,
`:2803`): `windows fb=0x1fffe00000000+0x200000000 ram=0x1fffc00000000+0x200000000
rings=0xff00000000+0x100000000`. In the 201 committed trace files that contain both kinds of birth,
no VA space had a Translated and a Passthrough birth together. The code allows it; the traces never
show it happening.

## 4. What else was checked

- **Guest rows** go through `HostVas::map` (`crates/kf-mem/src/ledger.rs:431-457`). The VA, store or
  RAM offset and permissions come from a walked guest leaf. A user twin never receives privileged
  guest leaves (v3-roperm, `mem.rs:1212-1215`).
- **SKED rows** (`ledger.rs:459-480`) take their VA from a guest leaf. Their kind is
  `SMSKED_MESSAGE`, the backing is a store page, and the address names nothing.
- **Usermode (doorbell) leaves** in a GPU space are not mirrored (`mem.rs:1546-1555`,
  `ledger.rs:225-228`).
- **The guest-VA reservations** (`GUEST_VA_RANGES`, `channel.rs:102-107`, `:741-753`) are lazy
  `NV50_MEMORY_VIRTUAL` objects with no PTEs.
- **The USERD relay object** (`chan.rs:414-436`, `:4894-4925`) is in no GPU VA space. PBDMA reaches
  it through the instance block.
- **The error notifier** is a context DMA, not a VA mapping.
- **The T-space ring layout** (`crates/kf-chan/src/host.rs:62-91`): push buffer and GPFIFO read-only,
  fence read-write, USERD unmapped. The default build's `LEGACY_LAYOUT` (`host.rs:50-56`) is one
  read-write map of all four.

**The sysmem-USERD address-size fix the owner expects to be absent is present on this branch.**
`V3_USERD_RELAY.md` (STATUS LIVE, 2026-10-08) and `chan.rs:414-436`, `:5650-5672`, `:5785-5801`,
`:5894-5919` relay `GP_PUT`/`GP_GET` for a Windows user-work Passthrough twin whose sysmem USERD sits
at an IOVA wider than a USERD may be. This is the case `kchannelCreateUserdMemDesc_GV100` refuses. The
code runs only when `KF3_WIN_USER_CHANNELS_PASSTHROUGH=1`, which is off by default, and
`KF3_USERD_RELAY_OFF=1` turns it off. It maps nothing into any GPU VA space, so it does not break the
VA invariant. It is **absent from `origin/master`** (`7040011a`): neither `relay_userd` nor
`V3_USERD_RELAY.md` exists there. The owner should know it exists on the Windows line of branches.

## 5. The verdict table

"Reach" means: can guest-controlled GPU work running natively in that space name the VA?

| # | Mapping | Spaces | Range, permissions, placement | Origin | Reach | Verdict | Breaks owner invariant? |
|---|---|---|---|---|---|---|---|
| 1 | **Store window** | default build: **every** mirror, prewarmed and retired spares | `[0, fb_len)` of the store, the **whole store including the firmware carve-out**. RW, non-privileged, `GROWS_DOWN` near the top of the space (**[measured]** `0x1fffe00000000`) `mem.rs:2776-2795`, `:2938-2952`; `channel.rs:959-975` | kayfabe | yes | **UNSAFE** | **YES** |
| 2 | **Guest-RAM window** | same as row 1 | all guest RAM (the memfd OS descriptor). RW, non-privileged, `GROWS_DOWN` (**[measured]** `0x1fffc00000000`) | kayfabe | yes | **UNSAFE** | **YES** |
| 3 | **Ring region with Translated rings** | default build: guest-kernel mirrors (a ring per Translated birth, `chan.rs:6030-6037`) | `[2^40 − 4 GiB, 2^40)`, 1 MiB slots, `LEGACY_LAYOUT`: one RW map of push buffer + GPFIFO + fence + USERD | kayfabe | yes, by a Passthrough channel born in the same space (not refused on the default build), and by the guest kernel's own forwarded virtual operands (S1-23) | **UNSAFE** against guest root; SAFE against guest userspace only because the guest's RM keeps a process out of a kernel client's space **[inferred]** | **YES** |
| 4 | Ring-region reservation (VMM range, no PTE) | default build: every mirror (`vmm_ranges`, `mem.rs:1201-1209`) | the same range, declared "ours". Guest leaves there are refused | kayfabe (declaration only) | no PTE, so an access faults | SAFE (but it refuses guest leaves the guest is entitled to) | no mapping; disappears with T-mode |
| 5 | Guest-VA reservations | every mirror, both builds | `[4.5 GiB, 1 TiB − 64 GiB)`, `[2^40, 2^47)`, plus a low range on Windows. Lazy, no PTE | kayfabe (declaration only) | no PTE | SAFE | no mapping |
| 6 | Guest rows (memory leaves) | every mirror | guest VA, guest permissions, store slice or RAM. Privileged leaves withheld in user twins | **guest leaf** | yes, as intended | SAFE in T-mode. **Default build: a leaf into the firmware carve-out is only counted** (`carve_gpu=`), so guest root can map kayfabe's root pages into a twin (`tspace.rs:61-69`) | origin OK; on the default build the *target* can be kayfabe's internals |
| 7 | SKED rows | every mirror | guest VA, `SMSKED_MESSAGE` kind, store page | guest leaf | yes, as intended | SAFE | no |
| 8 | Usermode (doorbell) leaves | GPU mirrors | not mirrored | guest leaf | n/a | SAFE | no |
| 9 | **Host RM's own buffers** (GR context, global CBs, falcon context) | every space where a twin or engine object is born, both builds | placed by host RM: in the host hole `[1 TiB − 64 GiB, 1 TiB)`, or on the guest's context VA when steered (`chan.rs:3906-3945`, only with `KF3_NO_GUEST_VA_RESERVE`) | **host RM** | GR context, bundle CB and page pool are **privileged PTEs**, so a USER channel faults on them (**[read]** `kernel_graphics_context.c:1127,1289`, `kgraphics_gm200.c:99-101,169,197`; the fault is **[inferred]**). The rest: **UNKNOWN** | partly SAFE / UNKNOWN | **YES by the letter**, but bare-metal parity: NVIDIA maps the same buffers into every native process. They cannot be removed without dropping the engine |
| 10 | GSP's split-VAS server range | every host space, both builds | `[4 GiB, 4.5 GiB)`, PDEs pinned and copied to GSP (**[read]** `gpu_vaspace.c:343-431`). Contents and PTE bits are set by closed GSP | **host (GSP)** | **UNKNOWN** | **UNKNOWN** | **YES by the letter**, inherent to every RM address space |
| 11 | Store window, T-space | T-space only | `[0, carve)` (never the carve-out), RW, bottom-up below `RING_REGION_BASE`, 2 MiB pages plus a 4 KiB tail (`tspace.rs:270-319`) | kayfabe | only by kayfabe-authored Translated push buffers | SAFE against Passthrough (no Passthrough space). Translated reach rests on the T-mode rewriter's address perimeter (`crates/kf-chan/src/tspace_unsafe.rs`): unit-tested, but **T-RING-TRANSLATED has not run** | no (not a passthrough space) |
| 12 | Guest-RAM window, T-space | T-space only | all guest RAM, RW, GPU-uncached, bottom-up | kayfabe | same as row 11 | same as row 11 | no |
| 13 | T-space rings | T-space only | `[2^40 − 4 GiB, 2^40)` reserved first; push buffer and GPFIFO RO, fence RW, USERD unmapped | kayfabe | same as row 11; outside every address the rewriter can emit | SAFE **[read + unit-tested]**; box test owed | no |
| 14 | USERD relay object (Windows, flagged) | none (no GPU VA) | 4 KiB vidmem object; PBDMA uses it through the instance block; kayfabe's CPU view only | kayfabe | not VA-addressable | SAFE | no VA mapping (but see §4: the code exists) |

**Exploit sketch for rows 1-2** (guest-visible actions only). An unprivileged guest user runs any
CUDA program. Its context's VA space is a default-build mirror. A kernel dereferences
`fb_base + p` (or `ram_base + o`). The base is the top of the space minus the window length, so it
is predictable (**[measured]** the same base on every boot of that box). With it, the kernel can:

- read or overwrite guest physical VRAM page `p`, which includes the guest kernel's GPU page tables
  and other processes' buffers;
- read or overwrite any guest RAM page, which includes guest kernel memory;
- on the default build, overwrite kayfabe's BAR1/BAR2 root pages in the carve-out, which the walker
  then mirrors (bounded to the store and guest RAM).

The worst outcome is that guest root falls to guest userspace. It does not reach host memory: the
windows cover only the store and guest RAM, and the walker bounds every leaf **[read; T-PHYS-CE
(S1-20) owed for the host-boundary half]**.

**Exploit sketch for row 3** (guest root). A guest-kernel Passthrough channel born in a space that
holds a Translated ring, or the guest kernel's own Translated virtual copy, writes the ring's GPFIFO
or push buffer at `RING_REGION_BASE + n MiB`. The next pump then runs bytes kayfabe never authored
on a kayfabe channel. That defeats the rewriter as a boundary for that channel. The channel is still
USER and `DENY_PHYSICAL_MODE_CE`, so reach stops at that space (S1-23, depends on S1-20).

## 6. Is the Translated/Passthrough separation there today?

- **Default build: no.** Translated channels run **in the guest's mirror** and depend on its
  windows. Their ring is placed in the mirror's ring region (`chan.rs:6030-6037`). Physical operands
  are rewritten to mirror-window VAs (`Windows`/`SlotWindow`, `chan.rs:1160-1190`), bounded by
  `fb_len`, the whole store, so they can reach the carve-out. Virtual operands are forwarded
  unchanged in the guest kernel's space (S1-23).
- **`KF3_TSPACE=1`: yes, as designed.** Translated births go to the T-space or are refused by name.
  There is no fallback to mirror windows (`chan.rs:5940-5951`, `:6018-6029`). Mirrors and spares
  carry nothing of kayfabe's. The twin-state word refuses a Passthrough birth in a guest-kernel space
  and a Translated birth in a user space.

**Migration steps** (the existing P1+P2 plan, `V3_P1P2_TSPACE.md` §8, inc E):

1. Run the box steps, serially on one box per family: T-TSPACE-BUILD; REGRESSION A/B (fast suite
   30/30, ladder, app matrix with at least one app as an **unprivileged** guest user, gfx, LLM lane)
   at `KF3_TSPACE=0` versus `1`; T-WINDOW-USER; T-RING-TRANSLATED; T-PHYS-CE. The probe tools
   (`tests/p1p2/window_reach_probe`, a guest kernel module for T-RING) **have not been written**.
2. Owner decision: make `KF3_TSPACE` default on.
3. Inc E: delete `default_windows`, `default_mirror`, `default_spare`, `default_reuse_reserved`, the
   ring-region entry of `vmm_ranges` in mirrors, the mirror ring-slot path, `LEGACY_LAYOUT`, and the
   `Windows`/`SlotWindow` rewriter. Delete the `known_violation_…` test with them.

**What can break** (from the design and the Windows runs; not measured on Linux):

- A guest-kernel Passthrough channel in an RM-internal or Translated space is refused
  (`twin_refused=`), and so is a Translated birth in a space with user channels. Which Linux channels
  hit this is **UNKNOWN** until REGRESSION A/B.
- T-mode refuses unclassified methods, `SubDeviceMask`, monitored-fence and similar address methods
  by name. A family without a committed census may lose guest-kernel work.
- The rewriter's fixed bounds. **[measured]** on Windows: `MAX_PIECES = 64` killed paging copies; it
  is now 4096 (`V3_WINDOWS_BRING_UP_20261009.md` row 3).
- 4096 T-space ring slots per VM life, reused only after a clean release; leaks are counted.
- An operand with no row yet waits for the pending walk. This is bind latency, not a stall: the bind
  waits on the walk inbox without blocking (§35).

## 7. Fixes, in order of preference

1. **Translated work only in the T-space, and nothing of kayfabe's in any mirror.** This is built,
   behind `KF3_TSPACE=1`. Make it the default after step 1 of §6, then delete the default path. It
   needs no new trap and no new wait on an input-serving thread. The T-space is built once, at
   prewarm, on the VA thread, never on a birth path. Each mirror stops paying for the two window maps
   (`[measured c3]` about 54 ms of host RM time per new space, `mem.rs:1431-1434`). Placement stays
   derived: window bases are read back and both ends are checked below `RING_REGION_BASE`.
2. Map the windows privileged-only: **not available** (§2), and Translated USER rings would fault too.
3. Place them where no guest-formable VA reaches: **not available**. There is no hardware VA limit on
   Ampere+, a 49/57-bit VA covers the whole space, and rings and semaphores must stay below 2^40.
4. Create per channel at birth and tear down at retire: **does not fix it**. The mapping still sits
   in a space a Passthrough channel can share. Mapping 8+ GiB at birth would also put a 50-90 ms host
   call on the act thread (a §35 stall).

**No interim patch is included.** Leaving the windows out of user twins on the default build would
mean mapping them lazily at the first Translated birth. That puts the window map (measured 65-86 ms)
on the act thread, which is the stall §35 forbids. T-mode is the fix.

## 8. The tests added (GPU-free, `cargo test -p kf-qemu`)

- `crates/kf-qemu/src/exposure.rs`: the invariant as a check. `kayfabe_placements` names each
  kayfabe placement from a space's record (store window, RAM window, ring region, unnamed VMM range).
  `passthrough_admissible` mirrors the birth gate: in T-mode a kernel space refuses, and on the
  default build nothing refuses; it is tested against the real `TwinState::try_user`. `violations`
  lists every placement in a passthrough-admissible space. Its unit tests show the check fails on a
  placement.
- `mem.rs` tests, driven through the same functions the production paths call. The default build's
  window maps and records were factored out unchanged into `default_windows`, `default_mirror`,
  `default_spare`, `spare_of` and `default_reuse_reserved` so the test runs the real paths:
  - `owner_invariant_holds_on_every_tmode_mirror_path`: create, prewarm, recycle, and
    retire-then-reuse, for a user key and an RM-internal key. No kayfabe placement, and no window
    map call.
  - `owner_invariant_check_catches_a_planted_window`: the **deliberate violation**. The T-mode
    positive control (`negctl`) maps a store window, and a planted ring region is added to a clean
    twin. Both are caught.
  - `known_violation_the_default_path_maps_kayfabe_placements_in_every_mirror`: asserts that today's
    default build puts the ring region, the RAM window and the store window in every mirror, on every
    path, user and kernel keys alike. It also checks that the store window extends past the
    carve-out base. It stays green while the violation exists and goes at inc E.
- Mutation check: making `violations` always empty fails three tests, and dropping the T-mode
  condition from `passthrough_admissible` fails two.

## 9. What this review could not settle

- **Reach was not measured.** T-WINDOW-USER, T-RING-TRANSLATED and T-PHYS-CE have never run, and
  their probe tools do not exist. Rows 1-3 are UNSAFE by reading plus the measured placement, not by
  an observed exploit.
- **Rows 9-10.** Which of host RM's per-channel buffers carry the privileged PTE beyond GR context,
  bundle CB and page pool (falcon contexts, attribute/RTV CBs, FECS event buffer, priv access map),
  and what GSP maps in the split-VAS server range, is decided partly in closed GSP firmware. A box
  probe could settle it: from a USER twin, read each VA host RM placed and look for `PRIV_VIOLATION`
  faults. The owner should also rule whether these host-RM-native placements are an accepted
  exception to the invariant, since they are bare-metal parity.
- **Which Linux guest channels T-mode's twin-state gate would refuse.** REGRESSION A/B settles it.
- **Whether a USER channel faults on a privileged PTE on every family.** The field comment and RM's
  use say yes; the engine side is closed. Inferred, not read.
