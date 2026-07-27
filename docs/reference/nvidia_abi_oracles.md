# The NVIDIA ABI oracles — what each one says, and where they disagree

**What this file is.** The cited ground truth behind `crates/kayfabe-abi`, kept in
`docs/reference/` per this repo's convention: facts with a `file:line`, correctable in one
place, deliberately *not* mixed into a design doc. Every number here is asserted by a test in
`crates/kayfabe-abi/tests/oracle_layout.rs`, so this file and the code cannot drift silently.

**What it is not.** The generation strategy, the version-table design and the safety argument
live in `crates/kayfabe-abi/src/lib.rs`'s crate docs, next to the code they constrain. The
two-axis versioning research is `../../../nvidia-gpu-passthrough/docs/design/mode2_abi_agnostic_layer.md`.

---

## 0. The three oracles

| oracle | what it is | version it speaks for | where |
|---|---|---|---|
| **ogkm** | NVIDIA's own FINN-generated headers | **610.43.02**, one snapshot | `../../../nvidia-gpu-passthrough/research_clones/ogkm/` |
| **nvproxy** | gVisor's independent transcription, versioned | 535.104.05 → 590.48.01, 17 entries | `../../../nvidia-gpu-passthrough/gvisor/pkg/abi/nvgpu/`, `.../sentry/devices/nvproxy/version.go` |
| **the C artifact** | this project's working Mode-1/Mode-2 implementation | 535 / 570≡575 / 580 profiles | `../../../nvidia-gpu-passthrough/src/abi/`, `src/common/nvkvm_abi.h`, `tests/abi_parity/` |

The generated code comes from **ogkm**. nvproxy and the C artifact are the independent checks —
a generator validated only against its own input proves nothing.

★ **The bench runs 580.159.04** (`rm_semantics_measured.md` §0), which is *newer than every
nvproxy entry* (newest 580 entry is 580.126.20, `version.go:1085`) and *older than the vendored
ogkm tag*. So no single oracle speaks for the bench directly; the claim "the bench uses layout
X" is always an inference from a boundary plus an ordering.

---

## 1. ★ FINDING — `NVOS46_PARAMETERS`: the oracles disagree, and the C's own test is the outlier

| source | says | citation |
|---|---|---|
| ogkm 610.43.02 | **64** bytes; `flags2` @ +36, `kindOverride` @ +40, `dmaOffset` @ +48, `status` @ +56 | `nvos.h:2168` |
| nvproxy, < 580.65.06 | **56** bytes; `dmaOffset` @ +40, `status` @ +48 | `frontend.go:625-639` |
| nvproxy, ≥ 580.65.06 | **64** bytes | `frontend.go:654-668`, switched at `version.go:1057-1059` |
| C artifact — **runtime** | 56 for the 535/570 profiles, **64** for the 580 profile | `nvkvm_abi.h:66,76,86` (`.nvos46_size`, `.nvos46_status_off`) |
| C artifact — **parity test** | **56**, unconditionally | `abi_parity_test.go:68-71` |

**The reading.** ogkm, nvproxy and the C's *runtime* are consistent once you notice the struct is
versioned. The C's **parity test is the outlier**: it asserts one size for a struct the C's own
runtime knows has two, so the test is weaker than the code it guards and would stay green if the
580 branch of `nvkvm_abi.h` were deleted. Since the bench is 580.159.04 > 580.65.06, the C is
right at runtime on its own bench and its parity test is pinning a layout the bench does not use.

**Why it matters beyond bookkeeping.** The two shapes have *the same prefix*, so a stale 56-byte
reader on a 64-byte buffer does not fail — it reads `kindOverride` as the low half of `dmaOffset`
and returns a plausible wrong GPU VA. Length alone cannot catch that direction. Pinned by
`mean_wire.rs::the_same_bytes_decode_differently_under_the_two_tables`.

**Second-order finding.** The C selects its profile from the **major version alone**
(`nvkvm_abi.h:112-121`, `nvkvm_abi_id_for_major`), but the boundary is at **580.65.06**. Any
hypothetical 580.x below .65.06 is mis-classified. The same coarseness applies to
`NVOS47_PARAMETERS`, whose boundary is **550.54.04** (`frontend.go:707-710`), also mid-major.
And `nvkvm_abi_by_id` **falls back to the 570 profile** for an unrecognised id
(`nvkvm_abi.h:105-110`), so an unknown driver silently gets 575's struct sizes.
`kayfabe-abi` keys on all three components and refuses below its floor.

---

## 2. ★ FINDING — `sizeof(rpc_message_header_v03_00)` is 32, and the C emulator says 36 once

- `nvkvm_gpu_emul.c:1586` — `stl_le_p(el + 56, 36u); /* length = sizeof(rpc_message_header) */`,
  on the bare-header path that posts `GSP_INIT_DONE`.
- `nvkvm_gpu_emul.c:1637` — "the GSP message element is {48-byte element header, **32-byte
  rpc_message_header**, params…}, so params live at `el+48+32 = el+80`".
- `nvkvm_gpu_emul.c:1657` — `stl_le_p(el + 56, 32u + 32u); /* rpc.length = hdr(32) + body(32) */`.

ogkm agrees with **32**: seven `NvU32` plus a 4-byte union (`g_rpc-message-header.h:41-52`).
The `36` is benign *today* only because the message is zero-padded and both sides checksum the
declared length, so the extra four bytes are four zeros nobody reads. It is still a wrong constant
on the boot path, and it is exactly the class of thing a generated layout removes.

---

## 3. ★ FINDING — `NV0000_ALLOC_PARAMETERS` has only ONE oracle

`grep -r NV0000_ALLOC_PARAMETERS` finds **nothing** in `gvisor/pkg/` and **nothing** in
`nvidia-gpu-passthrough/src/`. Neither nvproxy nor the C artifact models the client-root alloc
params at all. So the full 120-byte layout (`cl0000.h:47-52`: `hClient`, `processID`,
`processName[100]`, `pOsPidInfo`) rests on ogkm 610.43.02 alone.

This matters because `processID` is **the decision-#14 grouping discriminator**
(`l1_concurrency.md` §12.27) — the single field that decides whether a guest client is a user
process or the guest kernel. The corroboration it *does* have is RM's own writer, which sets the
two prefix fields by name (`ogkm src/nvidia/inc/kernel/vgpu/rpc.h:55,70,74`):

```c
root_alloc_params.hClient = hclient;                       // :55
    ...
    root_alloc_params.processID = KERNEL_PID;              // :70   (privLevel >= RS_PRIV_LEVEL_KERNEL)
    ...
    root_alloc_params.processID = pClient->ProcID;         // :74   (was cited as :75 — that is
                                                           //        the NV_ASSERT on the next line;
                                                           //        corrected 2026-07-27, doc audit)
```

**Consequences taken in code, deliberately:**

- `ClientAllocFacts` is decoded from an **8-byte prefix contract**, not the whole struct — that is
  the exact extent of what is corroborated.
- `DriverAbi::alloc_param_size` returns **`None`** for `NV01_ROOT`/`NV01_ROOT_CLIENT`. Reporting
  120 would be a guessed size in the one table whose whole purpose is to refuse guessed sizes.
- `pOsPidInfo` has the shape of a recent addition (RM only sets it on the non-kernel path), so
  `sizeof` at 575 is genuinely unknown to us.

**The experiment that settles it:** vendor a second ogkm tag in `[550.54.04, 580.65.06)` and
regenerate. That also deletes `crates/kayfabe-abi/src/transcribed.rs` entirely.

★ A related caveat found in the same macro: the whole `processID` assignment sits inside
`if (!IsT234DorBetter(pGpu))` (`rpc.h:57`). On Tegra T234D and later, RM does **not** set
`processID` at all, so it stays 0 and would decode as `User { pid: 0 }`. Irrelevant to a discrete
x86 target; a real hazard if this project ever targets Tegra.

---

## 4. Agreements worth writing down (all three oracles concur)

| struct | size | citations |
|---|---|---|
| `NVOS00_PARAMETERS` | 16 | ogkm `nvos.h:162`; nvproxy `frontend.go:255`; C `abi_parity_test.go:58` |
| `NVOS21_PARAMETERS` | 32 | `nvos.h:464`; `frontend.go:300`; `:56` |
| `NVOS54_PARAMETERS` | 32 | `nvos.h:2230`; `frontend.go:738`; `:59` |
| `NVOS55_PARAMETERS` | 28 | `nvos.h:2265`; `frontend.go:371`; `:60` |
| `NVOS64_PARAMETERS` | 48 | `nvos.h:479`; `frontend.go:788`; `:57` |
| `NVOS47_PARAMETERS` (≥550.54.04) | 48 | `nvos.h:2196`; `frontend.go:711`; `:78` |
| `NV0080_ALLOC_PARAMETERS` | 56 | `cl0080.h:54`; `classes.go:198`; `:120` |

`NVOS64`'s field *order* is pinned as well as its size, because `nvos64_abi_fix` was an order bug
and swapping two same-width fields leaves `sizeof` untouched.

`NV0080_CTRL_DMA_SET_PAGE_DIRECTORY_PARAMS`: the prefix `physAddress @+0`, `numEntries @+8`,
`flags @+12`, `hVASpace @+16` is confirmed by ogkm `ctrl0080dma.h:802-810` **and** by the C
emulator's live snoop offsets (`nvkvm_gpu_emul.c:2528-2536`, reading `cmd+120/128/132/136`). The
tail (`chId`, `subDeviceId`, `pasid`, total 32) has ogkm only; `pasid` looks like a recent
addition in the same family as the `NV_VASPACE_ALLOCATION_PARAMETERS` `+Pasid` growth the C
records at 580 (`nvkvm_abi.h:83`).

★ **Naming**: NVIDIA spells the Device's GPU index `deviceId`; this project's prose and
`AllocFacts::device_instance` call it the *device instance*. Same field — ogkm's own MIG shim
assigns `ws->nv0080Params.deviceId = migDev->deviceInstance` (`src/common/src/nv_smg.c:517`).

---

## 5. Why layouts are target-independent (x86_64 ≡ aarch64)

Derived rather than assumed, from `ogkm src/common/sdk/nvidia/inc/nvtypes.h`:

- Every SDK field uses a fixed-width NVIDIA typedef; no `long`, no `size_t`, no bare pointer.
- The one apparent exception is not one: `NvP64` is `void*` under `NV_64_BITS` (`:306`) and
  `NvU64` otherwise (`:326`) — **8 bytes on both arms**.
- `NV_ALIGN_BYTES(8)` / `NV_DECLARE_ALIGNED(x, 8)` expand to `__attribute__((aligned(8)))`
  (`:494`, `:508`). On any LP64 target a 64-bit scalar is already 8-aligned, so they are **no-ops**;
  they exist to fix up ILP32.

★ `NV_ALIGN_BYTES` expands to **nothing** on a compiler that is neither GCC-like nor `__arm`
(`:498-500`, with NVIDIA's own comment "XXX This is dangerously nonportable!"). Not a hazard for
this project — but it is why the generator treats an alignment attribute that would *raise* a
field's alignment as a hard error rather than emitting a plain `#[repr(C)]` mirror.

---

## 6. Open items

1. **Vendor a second ogkm tag** in `[550.54.04, 580.65.06)`. Deletes `transcribed.rs`, settles
   `NV0000_ALLOC_PARAMETERS`'s size at 575, and is the "day-not-a-month" drill
   (`mode2_abi_agnostic_layer.md` §6, experiment V1) run for real.
2. **A regeneration CI job** — re-run the generator against a vendored tag and
   `git diff --exit-code`, so a hand edit to a generated file cannot survive review. It must be
   *optional* (skipped when the ogkm tree is absent), since ogkm is not a build dependency.
3. **The wire → `RmEvent` mapping**, once `kayfabe-core`'s `RmEvent`/`AllocFacts` settle.
4. **The rest of the slice**: per-class alloc params (channel, VASpace, memory — the fields
   `AllocFacts` still needs), the GSP-RPC payload structs, the UVM ioctls, and the per-command
   capability allowlist that closes the default-allow gap (`nvproxy_gap_analysis`).
5. **CI coverage for the generator crate.** `crates/kayfabe-abi/gen/` is deliberately its own
   cargo workspace (so ogkm is never a build dependency), which also means `cargo fmt --all`,
   `cargo clippy --workspace` and `cargo test --workspace` at the repo root do **not** reach it —
   the same gap the `fuzz` workspace has, and which CI closes for `fuzz` with a second
   `working-directory` step (`.github/workflows/ci.yml` — grep `working-directory: fuzz`;
   ~~pinned `:337-339`~~ — **citation drifted; it was at `:399` and `:425` as of 2026-07-27,
   and `:337-339` had become the `unsafe_code` lints gate, a different gate entirely. Cite the
   step, not the line**). The generator needs the
   same three steps. It is clean today (**20** unit tests — *was written as 21; counted
   2026-07-27: `gen/src/ctype.rs` 7 + `gen/src/parse.rs` 13, `emit.rs` and `main.rs` 0* —
   clippy-clean, rustfmt-clean, verified by hand),
   but "verified by hand" is exactly what this repo's gate discipline exists to replace.

   > ★ **This item is still open as of 2026-07-27** — `grep -n working-directory
   > .github/workflows/ci.yml` returns only the two `fuzz` steps; nothing reaches
   > `crates/kayfabe-abi/gen/`. Note the small irony that the *count* in this very item rotted
   > by one while the gate that would have caught it stayed unbuilt.
6. **Wire the ABI into the mean suite.** `testing_doctrine.md` §3.1 item 3 requires each
   milestone's cases to land in `tests/tests/l1_mean.rs`, not only in a fresh isolated file.
   `crates/kayfabe-abi/tests/mean_wire.rs` composes a realistic RM event stream, but it does so in
   its own file because `tests/tests/` was owned by another agent this round. Fold the decoded
   event stream into the shared mean run when the `RmEvent` mapping lands.
