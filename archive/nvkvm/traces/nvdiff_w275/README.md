# `traces/nvdiff_w275/` — the ioctl differential, RE-CAPTURED UNTRUNCATED (2026-08-12)

**STATUS: LIVE.** Supersedes `traces/nvdiff_w274/` **for the parameter bytes**. w274's captures
are still valid for sequence, status and opcode census; their `UVM_MAP_EXTERNAL_ALLOCATION`
parameter bytes are **partial** and must not be read.

## What changed, and only this

`NVDIFF_MAXBUF` 8192 → **65536**, on both arms. Nothing in the device changed — the port is
`kayfabe@55c5d16`, whose only commits over w274's `ba2927b` are script-only, and all six ARM
ASSERTIONS pass with `OPERAND-PIN=156` / `DOORBELL-XLATE=88`, matching `w271_pin` exactly.

| capture | records | truncated | what it is |
|---|---|---|---|
| `host_vh/ce_r1.jsonl.zst`, `ce_r2.jsonl.zst` | 578 each | **0** | native `vh`, single GA106, open `580.159.04`. **Noise floor between them: ZERO.** |
| `guest_w275/ce_r1.jsonl.zst` | 436 | **0** | Mode-2 guest, boot `w275_pin`, extracted mid-hang |

`sizeof(UVM_MAP_EXTERNAL_ALLOCATION_PARAMS)` = **9264** (`uvm_sizes.h:30`, the largest struct in
the table). All **25 host** and **18 guest** records now carry the full 9264 bytes; at 8192 every
one of them lost 1072 bytes.

## ★★★★★ THE HEADLINE — THE 1072 RECOVERED BYTES CONTAIN NO DIVERGENCE

`UVM_MAP_EXTERNAL_ALLOCATION` still shows **exactly 36** UNEXPLAINED divergences — the same count
as at 8192. The newly captured region (offsets 8192..9263) is **byte-identical between host and
guest in all 18 comparable records.**

And the 36 are **one fact repeated**: every one is at offset **`0x18`**, and every one is the same
16-byte pair.

```
host   d09136851ec0805ae31943a901a0e1ff
guest  78b352c71ccd7a86d28249484c827f27
```

Offset `0x18` is `perGpuAttributes[0].gpuUuid` — `base`(8) + `length`(8) + `offset`(8) = 24 = 0x18,
and `UvmGpuMappingAttributes` opens with a 16-byte `NvProcessorUuid`
(`ogkm-580.159.04/kernel-open/nvidia-uvm/uvm_ioctl.h:493-497`).

⇒ **It is the GPU UUID, and it is environmental.** Each side is internally consistent and the two
never mix:

| | carries its OWN uuid | carries the OTHER's |
|---|---|---|
| guest | **80** record-fields | **0** |
| host | **96** record-fields | **0** |

The guest's own UUID appears in `REGISTER_GPU`, `REGISTER_GPU_VASPACE`, `REGISTER_CHANNEL` (32),
`MAP_EXTERNAL_ALLOCATION` (36), `ALLOC_SEMAPHORE_POOL` and the rest — i.e. the guest registered a
GPU under that UUID and then consistently referred to it. **80 of the 132 value divergences
(60.6 %) are this single identity constant**, the same class as `CARD_INFO`.

⇒ **The call at the divergence point does not differ in content.** The guest issues it with
semantically identical parameters and simply stops after 18 of 25.

## The census, ranked BY KIND

`records: A(host)=578 B(guest)=436  ratio=0.710` → **428 divergences** (w274: 429).

| kind | n | reading |
|---|---|---|
| EXTRA | **76** | every one `RM_CONTROL cmd=0x20801702` (`MC_SERVICE_INTERRUPTS`); host calls it zero times |
| MISSING | 218 | the teardown tail the guest never reaches (`RM_FREE` 93, `UVM_FREE` 27, …) |
| UNEXPLAINED | 132 | **80 are the GPU UUID.** Remainder: `0xc36f0108` 16, `0x00800292` 12 (a contiguous run at `0x188`..`0x1ac`), `GPU_GET_NAME_STRING` 2, `CARD_INFO` 2, the rest ≤3 each |
| STATUS | 2 | `A[41] 0x20810108` and `A[95] 0x2080200a`, host `0x0` vs guest `0x56` |

## ⊘ COVERAGE — unchanged from w274, and still the load-bearing limit

This is a **control-plane** instrument. It cannot see BAR/MMIO, the doorbell, USERD
`GP_PUT`/`GP_GET`, the pushbuffer, the GPU's DMA writes, interrupt delivery, or **the completion
plane** — which is where the wall is. Its own headline (`MC_SERVICE_INTERRUPTS`) is a *shadow* of a
data-plane fact. One workload, one chip, one run on the guest side (so the guest has **no noise
floor of its own**; only the host's was measured, and it is zero).
