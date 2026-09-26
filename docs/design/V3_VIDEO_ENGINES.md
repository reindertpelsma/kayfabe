# V3 video engines — NVENC and NVDEC for guest userspace

**STATUS: LIVE, 2026-09-26 (branch `v3-video`).** Measured on one GA106 (RTX 3060) with the stock
580.159.04 driver, in a kf3 fat guest on vast box `52661900`. Every NVENC and NVDEC output of the
graded lane is **byte-identical to bare metal** on the same box (§5). Turing, Ada and Blackwell are
derived from source and not yet measured on hardware. Hopper is refused by name (§6).

Read with `V3_HEADLESS_GRAPHICS.md` §4, which listed the six gaps this branch closes. Legend:
**[E]** read in source at the cited place, **[M]** measured, with the run named.

---

## 1. What the guest needs before it will allocate a video class

The guest is a GSP client. Its CPU-RM decides that NVENC/NVDEC exist from five statements, and it
cross-checks them. Each statement now comes from the **host** or from ogkm source. Nothing is a
hand-written per-die row.

| statement (guest reader) | served from | where |
|---|---|---|
| `GspStaticConfigInfo.engineCaps[]` (the class-DB filter `gpuCheckEngineWithOrderList_KERNEL`, `gpu.c:6432-6565` [E]) | the served FIFO table, converted back to NV2080 bits | `kf_rm::authored::engine_caps`, `kf_abi::gspstaticinfo::ENGINE_CAPS_OFF` |
| FIFO device-info rows (`kfifoEngineInfoXlate`) | engine **types and counts** from the host's `GPU_GET_ENGINES_V2` (unprivileged); every slot authored per family | `hostquery::classify_engine`, `authored::engine_table` |
| `GET_CONSTRUCTED_FALCON_INFO` `0x208001b0` (`chandesConstruct` needs a falcon for a non-GR/CE class, `channel_descendant.c:213-229` [E]) | the **host's own falcon table** (unprivileged), kept to the video `engDesc`s | `hostquery::query_video_falcons` |
| kernel interrupt table (`intr.c:1034-1087` [E]) | one **non-stall-only** row per video engine, vector = its runlist | `authored::engine_notification_rows` |
| internal device-info PRI base | the host falcon's `registerBase` for the same `engDesc` | `hostquery::device_info_rule` |

Per-family constants, each cited in the code:

- `ENG_DESC` uses OBJMSENC `0xe97b6c` / OBJBSP `0x8f99e1` (`g_eng_desc_nvoc.h`).
- `MC` is 38+i for NVENC and 65+i for NVDEC.
- `RM_ENGINE_TYPE` is NVENC0 `0x25` / NVDEC0 `0x1d`.
- `DEV_TYPE_ENUM` is `0x0e` / `0x10` (nouveau `top/ga100.c`).
- MMU fault ids come from each family's `dev_fault.h`. On Turing they are **not contiguous**: NVDEC is `10, 25, 26`.
- Runlist, PBDMA and RESET are the device's own topology.

[M] On GA106 the authored rows equal the C tree's captured 11-row FIFO table slot for slot (`ENG_DESC`, type, fault id, MC, dev type). See `kf-rm/tests/video_engines.rs`.

**No falcon register model is needed.** This retires gap (3) of the headless doc. A generic kernel falcon in a GSP client never reads its `registerBase`:

- it has no engstate;
- `gkflcnResetHw` refuses (`kernel_falcon.c:356-360` [E]);
- the register HALs are reached only from KernelGsp.

The only guest registration is a non-stall `IntrService` keyed by the FIFO row's MC index (`kernel_falcon.c:362-397` [E]).

**Class sets are generated.** `tools/derive_classes.sh` now also emits `video_encoder` and `video_decoder`. The results:

| family | encoder classes | decoder classes |
|---|---|---|
| Ampere | `0xc7b7` | `0xc6b0`, `0xc7b0` |
| Turing | `0xb4b7`, `0xc4b7` | `0xc4b0` |
| Ada | `0xc9b7` | `0xc9b0` |
| Hopper | **none** | `0xb8b0` |
| Blackwell | `0xceb7`, `0xcfb7`, `0xd1b7` | `0xcdb0` … `0xd2b0` |

Hopper's missing encoder is derived from ogkm's class list, not assumed.

## 2. The channel: a passthrough twin on the video runlist

A guest video channel becomes a host twin exactly like a GR/CE twin. The guest's own GPFIFO and USERD are adopted at creation, the doorbell is rung inline, and nothing is parsed. The changes:

- **Birth.** `birth_twin` and the plane's birth gate accept NVENC/NVDEC engine types (`kf_abi::submit::is_video_engine_type`).
- **Engine object.** The host object gets the guest's class and **authored** `NV_MSENC/BSP_ALLOCATION_PARAMETERS {12, 0, engineInstance = the twin's own}` (`HostRm::alloc_video_object`).
- **Falcon context promote.** `_kflcnPromoteContext` sends `entryCount = 0` plus the context buffer's VA and size (`kernel_falcon.c:184-276` [E]). This is the one real producer of the shape the promote decoder refused as "legacy".
  - `decode_falcon_promote` accepts exactly that shape, and only for a video engine.
  - It is satisfied by the twin: host RM allocated and promoted its own falcon context with the object.
  - [M vid7] Before this fix, the guest's `NVC7B0` alloc failed with `0x1f`.
- **Completions.** One host non-stall OS event per advertised video engine is delivered on the vector the guest was told. The guest's `gkflcnServiceNotificationInterrupt` wakes its NVENC/NVDEC events.

### 2.1 ★ The host must place its falcon context where the guest put its own

> ⊘ **SUPERSEDED IN PART by the `v3-int` merge (2026-09-26) — read this first.** The collision
> below was measured on a twin space with **no guest-VA reservation**. `v3-gfx` (merged with this
> branch on `v3-int`) reserves the guest's allocatable ranges `[4.5 GiB, 1 TiB − 64 GiB)` and
> `[1 TiB, 2^47)` in **every** twin space with lazy `NV50_MEMORY_VIRTUAL` objects
> (`kf_host::GUEST_VA_RANGES`, `V3_HEADLESS_GRAPHICS.md` §6.2 walls 6–7). Host RM's own
> allocations — the GR context buffers there, the falcon context here — can then only land in the
> host hole `[1 TiB − 64 GiB, 1 TiB)`. G = `0x12002a000` and G+`0x1000` are inside the first
> reserved range, so **host RM can no longer take either**, and the two-allocator hazard this
> section fixes cannot arise for any guest VA in a reserved range.
> ⇒ **How the two compose** (`kf-qemu` `ChanPlane::engine_object`): the steer runs **only when G
> is outside a live reservation** (`VaSpace::guest_reserved`) — i.e. the reservation was refused by
> the host, or `KF3_NO_GUEST_VA_RESERVE=1`. Inside a reservation the guest's own mapping at G is
> left in place (no engine reads it: the twin runs on host RM's context) and the plane logs
> `falcon ctx G=… is inside the reserved guest VA … not steered`. The no-unmap-of-unplaced rule in
> `GpuMirror::unmap` is unchanged and still covers the unreserved case.
> ⚠ The residual at the end of this section is dissolved by the same argument when reserved.
> ★ `[measured vint, d536595d, GA106 580.159.04, vast 52684829]` every falcon promote of the video
> lane logged `not steered` (G = `0x12002a000` / `0x12002d000`), **0** `HELD BY HOST` leaves, and the
> lane is md5/PSNR/bit-exact identical to bare metal (15/15 lines) — `traces/v3_int_ga106/`.

[M vid10] In a video channel's VA space, guest RM and host RM place buffers with **the same lowest-free allocator**, for RM-internal and user buffers alike.

- The guest put its NVDEC context at G = `0x12002a000`.
- Host RM saw G already mirrored and took G+`0x1000`.
- nvcuvid then mapped a live 4 KiB buffer at that page.
- The walker could only report that page **HELD BY HOST**, so NVDEC used the wrong page. The frames came back untouched (Y=128/16, U=0) and ffmpeg still exited 0.

**The fix** is to steer host RM onto G:

- The falcon promote carries G.
- Just before the host engine-object alloc, the act drops G's placement row and unmaps G from the twin.
- Host RM then takes G itself. That is harmless, because no engine ever reads the guest's own context page.
- `GpuMirror::unmap` answers "no placement of ours at this VA" (host-held, or handed to host RM) with **no host call**. [M vid11] Without this, the walker's later unmap of the guest page at G named host RM's mapping, was refused with `Other(87)`, and the next process's `cuInit` failed.

[M vid12] After the fix, every video session logs `host ctx steered onto the guest's ctx VA`, and no leaf is held by host.

⊘ **Residual.** If the walker has not yet mirrored a guest page below G when the act runs, host RM takes that hole instead. The plane logs this ("not mirrored yet"). It has not been observed.

### 2.2 ★ The guest's TSG is one host group (affects CUDA as well)

> ⊘ **CORRECTED by the `v3-int` merge (2026-09-26): one host group PER (TSG, context share), not
> per TSG.** Every member of a host group is born on the group's legacy subcontext
> (`hContextShare = 0`), which is exact for CUDA (one ctxshare per TSG — the case measured below)
> and wrong for a TSG carrying several subcontexts. `[measured vint int_gfx, ada6855a]` with the
> `v3-gfx` headless-graphics lane merged in, the Vulkan render's graphics and async-compute GR
> channels (one guest TSG, two ctxshares) were merged onto one legacy host subcontext: the compute
> channel took host **Xid 69** (class error, 3D class `c797`, method `0x2620`) and the fence never
> signalled (`GFX_S3=FAIL`). The group map is now keyed `(hClient, hTsg, hContextShare)`
> (`ChannelAlloc::ctx_share`, `ChanPlane::groups`): CUDA keeps its one group; Vulkan gets one host
> group per subcontext — the shape `v3-gfx` measured bit-identical to bare metal. Mirroring guest
> subcontexts as host `FERMI_CONTEXT_SHARE_A`s inside ONE group is the faithful model and is not
> built.

[M vid5/vid6] Every ffmpeg CUDA context raised Xid 13 `SKEDCHECK05_LOCAL_MEMORY_TOTAL_SIZE` on its GR twin.

A driver-API probe reproduces this with no video at all: a local-memory kernel launched on a second CUDA stream passes on bare metal and fails in the guest (`sync 719`).

The cause:

- CUDA puts its 8 GR channels in **one TSG** sharing one GR context. The host trace shows one context share, 8 `c56f` channels and one TSG schedule.
- v3 gave each twin its own host TSG, so context state pushed on one channel never reached the others.

The fix mirrors the guest's TSG as one host group:

- `HostRm::birth_group` and `birth_member`: the first member binds the group; later members bind themselves.
- All members use the legacy shared subcontext.
- The group is scheduled once per act and freed with its last member.

The LLM lane on master independently found the same wall and cause (`V3_BUILD.md`, w828: *"any kernel
on a non-default CUDA stream faults"*, blocking the CUDA-graph arm). That entry now points here. The
graph arm has not been re-measured.

## 3. The userspace gates, answered from the host

These were found by diffing the same static ffmpeg on bare metal and in the guest (the nvdiff `LD_PRELOAD` recorder, `scripts/bench/video_hook.sh VIDEO_SHIM`). In order of discovery:

| control | guest symptom | answer |
|---|---|---|
| `0x2080a028` (GSS-legacy clock query) | `OpenEncodeSessionEx … unsupported device (2)` | host reply to an **authored** request, see below |
| `0x20809064` request `{0,1,8}` | the library then queried the wrong clock domain | host reply to the identical authored request (was: the cudart constant, captured for a different request) |
| `0x20808163` / `0x20808164` | `… incompatible client key (21)` | carried to **our** host client as an act (see below) |
| `MSENC_GET_CAPS_V2` `0x801b02`, `BSP_GET_CAPS_V2` `0x801c02` (routed to GSP) | — | the host device's tables per advertised instance (`kf_abi::videocaps`) |

**`0x2080a028` and `0x20809064`** (`kf_abi::gssreplay`):

- Their layouts are measured. On the host, `0x2080a028` needs the domain named **twice**, at `+0x0c` and `+0x20c`; with `+0x20c` zero the host answers `0x1f`.
- At realize the device asks the host exactly the authored request and keeps **only the bytes the host wrote**.
- A guest request is answered only if its size and every named input word match. Everything else falls through to what answered it before.
- Nothing is forwarded.

**`0x20808163` / `0x20808164`** acquire and release an NVENC session slot. [M host probe] The 9th acquire on the GA106 returns `0x69`: this is the GPU-wide GeForce session cap.

- The slot is host **state**, not a fact, so it cannot be answered from a table.
- Each guest acquire or release is an act on our host client (`ChanStatement::EncoderSession`).
- A freed guest client's outstanding slots are released with it.
- Guests and host processes therefore share the real cap.

## 4. What changed where (all on `v3-video`)

- **`kf-chip`**: `Kind::VideoEncoder`/`VideoDecoder`; `FAMILIES` regenerated.
- **`kf-abi`**:
  - NV2080/RM video engine-type helpers;
  - `engineCaps` written;
  - falcon-info decoder;
  - `decode_falcon_promote`;
  - `gssreplay`, `videocaps`.
- **`kf-rm`**:
  - video `EngineKind`s;
  - `engine_table`, `engine_notification_rows`, `engine_caps`;
  - `query_video_falcons`, `query_gss_replay`, `query_video_caps`;
  - `HostControls::device_control`;
  - chanlink carries video objects, the falcon promote and encoder sessions;
  - `tests/video_engines.rs` (9 tests).
- **`kf-host`**: `alloc_video_object`, `birth_group`/`birth_member`/`free_member`, `device()`, NVENC/NVDEC notifier indices.
- **`kf-chan`**: video twins; `birth_twin_in(join)`.
- **`kf-qemu`**:
  - video birth, promote and events;
  - host groups;
  - encoder sessions;
  - falcon-context steer;
  - the no-row unmap.
- **bench**:
  - `scripts/bench/video_lane.sh` — the graded recipe, run identically on host and guest;
  - `scripts/bench/video_hook.sh` — the guest half, with optional nvdiff traces, `VIDEO_HOLD` and `VIDEO_SKIP`.

## 5. Measured results

The lane is `scripts/bench/video_lane.sh` (md5 `e58fe71f`). It uses one static ffmpeg on both sides: BtbN n8.1 linux64-gpl, sha256 `5d3a9e6b…`. The inputs:

- 90 frames of 1280×720 `testsrc2`;
- NVENC at `-preset p4 -rc constqp -qp 23 -g 30 -bf 0`;
- NVDEC through both `-hwaccel cuda -hwaccel_output_format cuda` + `hwdownload` (which cannot silently fall back to software) and `h264_cuvid`/`hevc_cuvid`, on the NVENC streams and on a single-threaded libx264 stream.

**[M] Guest run `vid13`**, binary `486ab7c6`, bare-metal comparison `host3`, same box and same minute. **Every md5, PSNR and bit-exact line is identical**:

| step | guest = host |
|---|---|
| `h264_nvenc` bitstream | `90ba39a7…` (1 933 646 B); PSNR vs source 50.75 dB |
| `hevc_nvenc` bitstream | `54665fae…` (1 951 664 B); PSNR 50.28 dB |
| NVDEC `hwaccel cuda` + `*_cuvid` on h264 / hevc / x264 streams (6 decodes) | each **bit-exact** to libavcodec's software decode (`5382f3c9…`, `cc5f1138…`, `0b3eaa0f…`) |
| 1080p `h264_nvenc` p1, 300 frames | 117 fps guest vs 113 fps host (unbenchmarked noise; parity) |

[M vid12] Repeated processes (decode, encode, decode, encode) in one guest give the same md5s each time.

★ **Guest persistence mode decides whether the whole lane survives** (boots at the rebased code
`353ff44a` / `0bbd9b1f`, which differ in bench scripts and docs only). The lane is about 12 GPU
processes, counting the two traced ones.

| boot | guest config | result |
|---|---|---|
| `vid13` | default | full lane passes, identical to host |
| `vidF` | default | UVM's CE channel dies (`slot N is full … RUN_CAP`) on the 3rd–4th GPU process; `hevc_nvenc` hangs |
| `vidC1` | default + `VIDEO_COMPACT=1` (drop caches and compact before each step) | the same wall, earlier ⇒ **guest memory fragmentation is NOT the cause** |
| `vidP1`, `vidP2` | `VIDEO_PM=1` (`nvidia-smi -pm 1`) | full lane passes twice, **every md5/PSNR identical to host**, no run-cap refusal, no host-held leaf; 1080p NVENC 165 / 187 fps |

This is master's open wall (`V3_BUILD.md`, "without guest persistence mode, UVM's CE channel dies on
the 5th CUDA process"). It is a memory-plane defect, not a video one: each process re-initialises the
guest adapter, and placements for UVM's re-created VA space accumulate. The video lane just reaches
it sooner, because it runs more processes. Results files are in `traces/video_ga106/`.

**Regression bar**, on the same box and the same host driver (files in `traces/video_ga106/`):

| revision | `v3_gates.sh` | fast suite (`KF_DEVICE=kf3`) | CUDA ladder in the fat guest |
|---|---|---|---|
| `486ab7c6` (pre-rebase) | 9/9 PASS | 30/30 PASS | `cup3` 43, `cup8` BAD=0 MAXERR=0 |
| `353ff44a` (rebased on `origin/master` `ce623b3f`; the pushed head `0bbd9b1f`+ adds bench scripts and docs only) | 9/9 PASS | 30/30 PASS | `cup3` 43, `cup8` BAD=0 MAXERR=0 |

- ⊘ Without `KF_DEVICE=kf3`, the suite runs the old nvkvm device and reports 30 NOTRUN.
- The CUDA ladder was re-checked because §2.2 changes every CUDA context's host topology.
- Crate tests pass for `kf-abi`, `kf-chip`, `kf-rm`, `kf-chan`, `kf-host`, `kf-mem` and `kf-qemu` at both revisions.

★ **Guest persistence mode decides whether the whole lane survives** (boots at the rebased code
`353ff44a` / `0bbd9b1f`, which differ in bench scripts and docs only). The lane is about 12 GPU
processes, counting the two traced ones.

| boot | guest config | result |
|---|---|---|
| `vid13` | default | full lane passes, identical to host |
| `vidF` | default | UVM's CE channel dies (`slot N is full … RUN_CAP`) on the 3rd–4th GPU process; `hevc_nvenc` hangs |
| `vidC1` | default + `VIDEO_COMPACT=1` (drop caches and compact before each step) | the same wall, earlier ⇒ **guest memory fragmentation is NOT the cause** |
| `vidP1`, `vidP2` | `VIDEO_PM=1` (`nvidia-smi -pm 1`) | full lane passes twice, **every md5/PSNR identical to host**, no run-cap refusal, no host-held leaf; 1080p NVENC 165 / 187 fps |

This is master's open wall (`V3_BUILD.md`, "without guest persistence mode, UVM's CE channel dies on
the 5th CUDA process"). It is a memory-plane defect, not a video one: each process re-initialises the
guest adapter, and placements for UVM's re-created VA space accumulate. The video lane just reaches
it sooner, because it runs more processes. Results files are in `traces/video_ga106/`.

**Regression bar at the same revision `486ab7c6`, same box** (files in `traces/video_ga106/`):

- `v3_gates.sh`: **9/9 PASS**.
- `KF_DEVICE=kf3 fast_suite.sh`: **30/30 PASS**, 0 FAIL, 0 CRASH, 0 NOTRUN.
  - ⊘ Without `KF_DEVICE=kf3` the suite runs the old nvkvm device and reports 30 NOTRUN.
- CUDA ladder in the fat guest: `cup3` `CUP3_VAL=43` and `cup8` `BAD=0 MAXERR=0`. Re-checked because §2.2 changes every CUDA context's host topology.
- Crate tests pass for `kf-abi`, `kf-chip`, `kf-rm`, `kf-chan`, `kf-host`, `kf-mem` and `kf-qemu`.

⊘ The guest boots still carry the pre-existing CUDA-path `nvAssert` noise (`INIT_USER_SHARED_DATA`, golden-image `c36f`, …). It is unchanged by this branch.

## 6. Gaps and what is not claimed

- ⊘ **CORRECTED 2026-09-26 (`V3_HW_BOUNDARY_INVENTORY.md`, branch `v3-hwinv`):** the refusal below rested on a false premise. `published/hopper/gh100/dev_fault.h` lacks `HOST0`, but UVM's copy of the same chip's header states it (`kernel-open/nvidia-uvm/hwref/hopper/gh100/dev_fault.h:83`, `HOST0` = 64; NVDEC0..7 = 19..26, NVENC0..2 = 35..37 at `:49-56,78-80`). `authored::fault_ids` now answers Hopper `384 / 43 / 64` and `video_fault_id` carries the Hopper rows, each held to the header by `authored::hwref_check`. Hardware-unverified.
- **Hopper** (superseded, above): `authored::fault_ids` refuses Hopper's whole engine table (no `HOST0` in `gh100/dev_fault.h`). This predates the branch. The GH100 NVDEC rows exist but cannot be served until that is solved.
- **Turing / Ada / Blackwell** video rows are derived and unit-tested, but no hardware measurement exists. Watch for:
  - Ada's AV1 encode (the guest-local `GPU_GET_ENCODER_CAPACITY` AV1 flag);
  - Ada's multiple NVENC instances.
- **Not advertised**: NVJPG, OFA and SEC2. Their `classify_engine` arms return `None` by design.
- **`0x2080a028`**: only the NVD and GPC clock domains are asked of the host. A library asking another domain is refused, as before.
- **The steer (§2.1) is falcon-specific.** Any other host-RM internal allocation in a mirrored VA space has the same two-allocator hazard. A GR collision would show as a `HELD BY HOST` leaf, and none has been seen.
- **The no-persistence-mode RUN_CAP wall** (§5) is master's open memory-plane defect. Without guest
  persistence mode, a multi-process video workload fails after a few processes, which is **not
  parity**. Fragmentation is ruled out (`vidC1`). The fix belongs to the walk/placement owner.
- **`0x20808165`** is still refused in the guest (it answers OK on the host). No library behaviour depends on it in the lane.
- **NVENC session-cap accounting** assumes one encoder-session slot per acquire, as measured. Concurrent guest and host encoders beyond the cap are refused with the host's own `0x69`.
