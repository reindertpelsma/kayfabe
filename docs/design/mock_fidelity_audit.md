# Mock-fidelity audit — "encodes what the hardware encodes, no more and no less"

**Date:** 2026-08-02 · **Base revision:** `b628df4` · **Branch:** `mock-fidelity`

## Why this document exists

Three separate increments have been blocked or misled by the same defect: **a seam looked
finished for as long as its only implementer was a double, because the double's invented
encoding was not the hardware's.**

| # | site | direction | how it surfaced |
|---|---|---|---|
| 1 | `MockArch::token_for` (E3) | invented layout | settled only by compiling RM's own encoder as an oracle |
| 2 | `gspworld::Guest::recv`'s `rpc_length` floor (#85 / Q21) | **stricter** than the driver | 580's bound transcribed onto a 610 profile that admits `rpc.length == 0` |
| 3 | `mock_method::CE_LAUNCH_DMA` (E4) | **more capable** than hardware | a real `AMPERE_DMA_COPY_B` copy is five method runs and `LAUNCH_DMA` carries none of its operands |

Cases 2 and 3 point in opposite directions and are the **same defect**. Neither is caught
by a green suite: a double that accepts too little makes the product look wrong when it is
right; a double that accepts too much makes a seam look right when it is wrong.

⊘ **A test written against a double is evidence about the double.** Nothing in this
document is a live-boot result (`only_live_boots_are_proof`); every claim below is a
**reading of `ogkm`**, tagged with the tree it was read in, and where the two vendored
tags disagree that disagreement is itself recorded rather than resolved by preference
(`ogkm_is_versioned`).

## How the universe was derived

⊘ Not by listing the doubles (`gates_quantified_over_a_list`). Two derivations, unioned:

1. **Every implementer of a seam that also has a real implementer** —
   `impl (Arch|GmmuFmt|UserdModel|PushbufferAbi|GspModel|HostClasses|RmBackend|Vmm|Isolate|IsolateFactory|Present)`
   over the whole tree. This is what finds a double that nobody named `Mock`.
2. **Every type whose name declares it a stand-in** — `Mock*`, `Fake*`, `Loopback*`,
   `Stillborn*`, `Unbuilt*`, `Wire*`, `Echo*` — which is what finds a double that
   implements no trait at all (`tests/src/rpcwire.rs` is a *module* of encoders, and would
   be invisible to (1)).

Of the resulting set, the ones to which *"encodes what the hardware encodes"* applies are
those standing in for silicon or for the NVIDIA driver: `MockArch` (+`MockGmmuFmt`,
`MockUserd`, `MockPushbuffer`), `WireClassArch`, `FakeGspModel`, `gspworld::Guest`,
`gspworld::Profile`, `rpcwire`, and the `Unbuilt*` refusals. The rest
(`MockVmm`, `MockRmBackend`, `MockQemuHost`, `MockPresent`, `LoopbackRm`) stand in for
*host software*, and were audited against the real adapters and their cited kernel/RM
behaviour instead.

★ **What the derivation cannot see, stated so nobody reads more into it:** a double whose
encoding is wrong *and* whose hardware counterpart nothing in this tree cites is invisible
to both derivations — there is no third party to disagree with. That is the shape of the
fifth limit in `c_rust_trace_differential.md`, one plane over.

---

## ★★★ FINDING A — a real GPFIFO entry names a GPU **VA**; the core reads it as a GPA

> ★★★ **CLOSED 2026-08-02, `execution_plane_increments.md` §9.1.** `PushRange::gpa: u64`
> is `PushRange::va: GpuVa`, `kayfabe_fwd::read_pushbuffer` resolves it through the
> issuing channel's address table before a guest byte is fetched (MISS = FAULT), and the
> read moved from `route_act`'s route phase into its act phase — no new lock, the same
> ranks, the change stated rather than slipped in. **`MockPushbuffer`'s entry names a VA
> too now**, and every mock-driven fixture binds its ring at a VA deliberately biased away
> from the GPA (`kayfabe_tests::PB_VA_BIAS`), so an identity fixture can no longer pass.
> The "Deferred" row below is likewise closed. The finding is kept in full because the
> *reason it survived* is the reusable part.

**This was the one that was load-bearing on the shipped path.**

- `kayfabe_fwd::read_pushbuffer` passes `PushRange::gpa` straight to `Vmm::gpa_read`, with
  no walk, under the device read lock.
- `Ga10xArch` is the `Arch` the QEMU archive ships, and `Ga10xPushbuffer::gpfifo_entries`
  returns the address decoded out of `NVC56F_GP_ENTRY0_GET 31:2` / `GP_ENTRY1_GET_HI 7:0`
  (`ogkm-580: src/common/sdk/nvidia/inc/class/clc56f.h:270, 272`).
- `[src]` What the driver writes into that field is a **GPU virtual address in the
  channel's own address space**: `pushbuffer_va = uvm_pushbuffer_get_gpu_va_for_push(pushbuffer, push)`
  handed unchanged to `set_gpfifo_entry` (`ogkm-580:
  kernel-open/nvidia-uvm/uvm_channel.c:996, 1006`). `kayfabe_abi::submit::gp_entry_decode`
  already names its own field `gpu_va`; the loss happens at the `PushbufferAbi` boundary,
  where it becomes `PushRange::gpa`.
- `Device::parse_pushbuffer`'s rustdoc asserted the converse in as many words — *"Every
  GPFIFO entry in `ring` names a guest-physical address"* — and that sentence is now
  corrected in place.

**Why nothing caught it.** `MockPushbuffer::gpfifo_entries` invents a 16-byte entry
`[gpa: u64 LE, len: u64 LE]` whose address genuinely **is** a guest-physical address, so
every mock-driven test of `read_pushbuffer` is correct by construction. The GA10x-driven
tests (`pushbuffer_abi_oracle.rs`, `pushbuffer_ga10x_hostile.rs`) cannot see it either,
because their fixtures place the method bytes in guest RAM *at the address the entry
names* — the fixture makes VA == GPA. Same family as E4: the double hands the core an
operand hardware does not hand it.

⊘ **Not fixed here — it is a shape change.** Resolving the range needs the issuing
channel's `Vas`, and `read_pushbuffer` is documented *and ranked-locked* as a phase that
"touches no proc", running before the owning proc's lock is taken. Moving the walk into
the act phase, or splitting the read into a resolve-then-read pair, is an E5-class
decision about the lock discipline, not an edit.

⊘ **And note what does NOT change:** the lock-safety argument in
`Device::parse_pushbuffer` rests only on the value being *guest-chosen* and on
`Vmm::gpa_read` refusing anything that is not host RAM. It does not rest on what the guest
meant by the value, so it survives this finding intact.

Recorded at: `kayfabe_arch::PushRange` (type note), `Ga10xPushbuffer::gpfifo_entries`,
`MockPushbuffer::gpfifo_entries`, `kayfabe_rt::Device::parse_pushbuffer`.

---

## ★★ FINDING B — the mock's GMMU root was 128× the regime's

`MOCK_LEVELS[0]` was `{ shift: 47, entries: 512 }`, and `MockGmmuFmt`'s own docs justify
the geometry as *"not fake … the C's own table transcribed value-for-value"*.

- `[src]` The regime's root is `virtAddrBitHi = 48`, `virtAddrBitLo = 47` — two VA bits,
  therefore **4** entries (`ogkm-580:`/`ogkm-610:
  src/nvidia/src/kernel/gpu/mmu/arch/pascal/kern_gmmu_fmt_gp10x.c:59-60`, byte-identical
  and on the same lines at both tags). `Ga10xGmmu::level_shift` already reports `(47, 4)`,
  and `gmmu_fmt_oracle.rs` differentials it against the driver's own compiled table.
- The C artifact's `nvkvm_m2_cpt_lvl[0]` really is `{ 47, 512 }`
  (`C: src/qemu/nvkvm_gpu_emul.c:8709`) — i.e. `page_bytes / entry_size`, **the derivation
  `LevelShift` exists to forbid**. So this is a defect of the oracle, faithfully
  transcribed. It makes the C's own PD3 sweep read 4 096 bytes of a 32-byte directory and
  synthesise virtual addresses at `i << 47` for `i` up to 511, past the top of the 49-bit
  VA space the format defines.

**FIXED** — `MOCK_LEVELS[0].entries` is `4`. The pin test
(`pt_decode.rs::the_level_table_is_the_regimes_and_entry_counts_are_not_derived_from_a_page_size`)
was seen to fail against the old value before the change and passes after it, and it is
now a strictly larger statement: the "`entries` is a count, not a size" property is
exhibited at **two** rows (root 4-of-512, big-page table 32-of-512) where it was exhibited
at one.

★ **How little the old number was constraining, as a run rather than an opinion:** with
`512` re-injected at `b628df4`+edits, `cargo test -p kayfabe-tests --test pt_decode` gives
`20 passed; 1 failed` — the pin test alone. Twenty of twenty-one page-table-decode tests
are indifferent to whether the root has 4 slots or 512. ⊘ That is a fact about this suite,
not about silicon (`only_live_boots_are_proof`): no boot was involved.

---

## ★★ FINDING C — `MockVmm::map_guest` served a protection both real backends refuse

`KvmVmm::map_guest` and `QemuVmm::map_guest` each answer `VmmError::Unsupported` for
`Prot::ReadOnly`, because KVM's read-only flag is a **slot** property and cannot be
expressed for one object inside a shared read-write window (`l2_qemu_adapter.md` §6.7
item 4). `MockVmm::map_guest` answered `Ok`.

No shipped caller passes `ReadOnly` today, so nothing depends on it — which is exactly why
it needed an instrument rather than a note: a latent divergence with no test reads as an
absent one.

**FIXED**, with `mock_map_guest_refuses_the_protection_both_real_backends_refuse` (seen to
fail with the refusal removed).

⊘ **Two sibling divergences deliberately NOT closed**, and they must not be read as
closed: the mock still accepts a `gpa` no installed window covers (both real backends:
`BadGpa`) and two placements overlapping in one window (both: `Unsupported`). Modelling
either needs the double to carry windows, which is a shape change.

---

## ★★ FINDING D — `vchid_from_userd_flags`: one contiguous field where RM writes two

The seam's **only non-refusing implementer is the mock**:
`Ga10xArch::vchid_from_userd_flags` answers `VChid(0)` for every input, as a documented
refusal. So the shape of the answer has never been driven against the encoding hardware
uses.

- `[src]` RM splits the chid across two **non-adjacent** subfields with a flag bit between
  them: `NVOS04_FLAGS_CHANNEL_USERD_INDEX_VALUE 10:8`, `_USERD_INDEX_FIXED 11:11`,
  `_USERD_INDEX_PAGE_VALUE 20:12` (`ogkm-580:
  src/common/sdk/nvidia/inc/alloc/alloc_channel.h:184, 186, 201`).
- `[src]` It fills them from the chid as `VALUE = ChID % n`, `PAGE_VALUE = ChID / n` with
  `n = NVBIT(DRF_SIZE(_USERD_INDEX_VALUE))` = 8 (`ogkm-580:
  src/nvidia/src/kernel/gpu/fifo/kernel_channel.c:2793, 2800, 2802`), so the recovery is
  `PAGE_VALUE * 8 + VALUE` — **not** a contiguous field read.
- `MockArch::userd_flags_for` packs it into one 12-bit field at `18:7`, which straddles
  both real fields and RM's `_FIXED` bit.

★ **Unlike E4, the seam's *signature* is adequate**: both real subfields live inside the
one `u32` the method is handed, so this is an increment, not a redesign.

⊘ **NOT built here, and the reading above is not a licence to build it.** A transcription
of three ogkm lines is precisely the instrument that has been wrong before
(`isolate_the_drivers_own_checks`). Landing this wants RM's own writer compiled as a
**fifth oracle** and differentialled over the field space, the way
`worksubmit_token_oracle.rs` settled E3's token — which also means a new reached-count
floor in `.github/workflows/ci.yml` and a matching move of `GATE_STEPS_ALL_MIN` in
`scripts/ci_gates.sh`. Recorded at `Ga10xArch::vchid_from_userd_flags` and
`MockArch::userd_flags_for`.

---

## ★ FINDING E — the mock guest refuses a bad length where a real guest consumes it

Extends #85 / Q21, which is about the *bound*. This is about the check's **position** and
whether it **gates**.

At both tags the length test is the **last** thing `GspMsgQueueReceiveStatus` does and it
gates nothing: 580 runs it after the checksum, after the seqNum and past the retry loop,
and the `exit:` label then calls `msgqRxMarkConsumed(nElements)` and `pMQI->rxSeqNum++`
regardless of the status it set (`ogkm-580:
src/nvidia/src/kernel/gpu/gsp/message_queue_cpu.c:760-786`); 610 is the same shape
(`ogkm-610: :826-833`). `gspworld::Guest::recv` hoists it to the front and returns.

Two consequences, neither load-bearing today:

- for an element bad in more than one way, the driver reports `BadChecksum` or
  `BadSequence` where the model reports `BadLength`;
- a bad-length element is **not** a wedge on a real guest, and it is one in the model.

The two live call sites (`tests/tests/gsp_boot.rs:197, 217`) accept `BadLength` only inside
an `Err(A | B)` disjunction, so nothing currently pins the wrong refusal.

⊘ **NOT fixed** — straightening it means making the model consume-and-continue, which is
the same shape change #85's version-split bound needs. Both are recorded at the site so
they land together.

---

## ★ FINDING F — `mock_ctrl`'s doc claimed a property its values do not have

The module said *"deliberately-fake values"*. Two of the three are NVIDIA's real command
ids: `0x2080012b` is `NV2080_CTRL_CMD_GPU_PROMOTE_CTX` (`ogkm-580:
src/common/sdk/nvidia/inc/ctrl/ctrl2080/ctrl2080gpu.h:984`) and `0x20801219` is
`NV2080_CTRL_CMD_GR_GET_CTX_BUFFER_INFO` (`ogkm-580: ctrl2080gr.h:1116`). Only
`FORWARDABLE` is invented.

**FIXED as a doc change, not a value change**, because the values are right: a wire-driven
test posts the id `kayfabe_abi::generated::ctrl` carries and `kayfabe_rmrpc` translates
`0x2080012b` by name, so inventing them would make the mock's ack-only arm unreachable
from wire bytes. What the doc now records is the **cost**: `MockArch`'s standing property
(*"deliberately fake encodings, so core code that secretly assumes a real NVIDIA encoding
fails these tests"*) does **not** hold on this seam.

---

## ★★ FINDING G — `kayfabe-mocks` is in the shipped archive's graph, and two files argued from the opposite

`crates/kayfabe-mocks/Cargo.toml` described itself as *"Test-only; never a production
dependency"*. `cargo tree -p kayfabe-qemu-raw -e normal` (run on the build box at
`09070a1`) puts it two **normal** edges under the crate that builds the QEMU archive:

```text
kayfabe-qemu-raw
├── kayfabe-chips
│   ├── kayfabe-mocks
```

`Ad10xArch` and `Gh100Arch` are `MockArch` composed with a real `GspModel`, and they still
delegate `classify`, `mmu`, `userd`, `pushbuffer`, `is_case2_control` and
`vchid_from_userd_flags` to it. `#156` already pulled `decode_doorbell` and `host_classes`
off that delegation for exactly this reason and stopped there.

**Two files cited the false sentence as a reason**, which is what makes this worth a
finding rather than a note: `kayfabe-qemu-raw/Cargo.toml` justified choosing `Ga10xArch`
*"because `kayfabe-mocks` is test-only and this archive ships"*, and `ga10x.rs`'s module
docs used the same clause to explain why `WireClassArch` is not the product's class table.
Both conclusions are right; both reasons were void. ⊘ A reason that does not hold is worth
correcting even when the conclusion survives — it is the shape `read_at_invalidate` had.

**FIXED as three corrections of record** (manifest description + the two citing comments).
⊘ **NOT fixed:** making the sentence true again means moving the `Ad10x`/`Gh100`
composition off `MockArch`, which is the same increment `Ga10xArch`'s module docs already
argue for (*"in the product it is the mock wall in its worst form: a plausible answer on
the one axis where a wrong answer is a silent memory-safety fact about the guest"*). Note
what that would buy: with `MockPushbuffer` behind an Ada or Hopper `Arch`, a real method
header is decoded by switching on `header >> 24` against invented opcodes, so a real
method run can decode to a `CeLaunchDma` with a **fabricated** destination, length and
work kind — which is precisely what `pushbuffer_ga10x_hostile.rs` exists to forbid.

★ **Reachability today, stated exactly so nobody over- or under-reads it:** `Ad10xArch`
and `Gh100Arch` are constructed only in tests (`kayfabe-chips/tests/host_classes.rs`,
`tests/tests/arch_axis_second_generation.rs`); no `ChipProfile` selects them. So the
*linkage* is real and the *reachability* is not — yet. The linkage is what makes the two
comments wrong; the reachability is what would make it a bug.

---

## What was checked and found to MATCH

Recorded because "found nothing" from a narrow search is the failure this audit exists to
catch, and because a clean row is the useful half of an inventory.

| double | thing | verdict |
|---|---|---|
| `gspworld::Guest::rx_link` | all nine checks, their **order**, and the codes `-1,-2,-3,-6,-7,-8,-9,-10` | matches `ogkm-580: src/common/shared/msgq/msgq.c:330-406` check for check |
| `gspworld::Guest::free_space` | `read_ptr >= msgCount → 0`, then `read + count - write - 1` with the `%`-free wrap | matches `msgqTxGetFreeSpace`, `ogkm-580: msgq.c:491-497` |
| `gspworld::Guest::new_sized` | `rxHdrOff = ALIGN_UP(32,16)`, `entryOff = ALIGN_UP(rxHdrOff+4, 4096)`, `msgCount = (size-entryOff)/msgSize` | matches `msgqTxCreate`, `ogkm-580: msgq.c:237-252` |
| `gspworld::fold` | 64-bit XOR to the next 8-byte boundary, reduced `hi ^ lo`, span = `hdrSize + rpc.length` | matches `_checkSum32` + both call sites |
| `gspworld::Guest::recv` MCTP arm | validates the **version nibble** and the **vendor id** only | matches `ogkm-610: message_queue_cpu.c:737-759` exactly; `ogkm-580` has no such block, and `P580` carries no transport words — the version seam is modelled, not flattened |
| `gspworld::FUNCTIONS` | all 17 ids | every one matches `rpc_global_enums.h` at **both** tags (`SET_GUEST_SYSTEM_INFO_EXT` 64, `ECC_NOTIFIER_WRITE_ACK` 202, `INIT_GSP_TRACE_CRASH_BUFFER` 228 included) |
| `kayfabe_abi` `GspElementWire` | `Pre610` hdr 48 / cs 32 / seq 36 / elemCount 40; `From610` hdr 16 / cs 8 / seq 12 / none | matches the two `GSP_MSG_QUEUE_ELEMENT` structs at `ogkm-580: message_queue_priv.h:43-51` and `ogkm-610: :52-67` |
| `kayfabe_abi` `GspTransportWords` | `0xC000_0001` / mask `0xF`; `0x2510_DE7E` / mask `0x00FF_FF00` | reassembles from `MCTP_HEADER_VERSION 3:0` and `MCTP_MSG_HEADER_VENDOR_ID 23:8` (`ogkm-610: src/nvidia/arch/nvalloc/common/inc/mctp_format.h`) — and the masks are exactly the two fields the driver `REF_VAL`s |
| `rpcwire` envelope + bodies | `rpc_message_header_v03_00` (32 bytes), `rpc_gsp_rm_alloc_v03_00`, `rpc_gsp_rm_control_v03_00` (`params` at **+40**, not +36) | field for field against `ogkm-580: src/nvidia/generated/g_rpc-message-header.h:41-52` and `g_rpc-structures.h:1491-1502, 1506-1518` |
| `MOCK_LEVELS` rows 1–5 | strides 38/29/21/12/16, counts 512/512/256/512/32 | match `kern_gmmu_fmt_gp10x.c`'s `virtAddrBitHi:Lo` pairs at both tags |
| `mock_classes` | `0xF0xx` | genuinely invented; collides with no NVIDIA class id |
| `MockUserd` | `0x400` / `0x110` / `0x118` | invented, as designed; ⊘ note the two cursors are **8** bytes apart where GA10x's are **4** (`0x88`/`0x8C`) — harmless only because no core code consumes these offsets, and it would stop being harmless the moment one did |

## Deferred, in one list

| # | what | why it is not an edit |
|---|---|---|
| ~~A~~ | ~~GPFIFO `PushRange::gpa` is a GPU VA on GA10x~~ | ★ **CLOSED 2026-08-02** — the phase moved into `route_act`'s act phase, which holds the proc and takes no new lock (`execution_plane_increments.md` §9.1) |
| D | `vchid_from_userd_flags` real decode | wants RM's writer compiled as a fifth oracle + a CI reached-count floor |
| E | mock guest's `BadLength` position and gating | needs consume-and-continue; lands with #85's version-split bound |
| C′ | `MockVmm::map_guest` unbacked-GPA and overlap refusals | needs the double to carry windows |
| — | `MockPushbuffer::gpfifo_entries` prefix-decodes a truncated ring and has no `LENGTH == 0` refusal, both more permissive than `Ga10xPushbuffer` | a per-arch policy choice; recorded at the site so no test there is read as evidence about the real codec |
| G | `Ad10xArch`/`Gh100Arch` still answer the data-plane seams with `MockArch`'s invented encodings | moving the composition is the increment `Ga10xArch`'s own module docs argue for |
