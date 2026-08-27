# `traces/nvdiff_w274/` — the ioctl differential, re-captured 2026-08-12

**STATUS: LIVE.** Supersedes `traces/host_reference_ga106/` **for diffing against our guest**;
that capture is still the record of what a five-GPU closed-driver rig does.

## Why a new host reference

The committed reference is a **five-GPU rig running the CLOSED driver**, which is why
`CARD_INFO` was its top divergence *by index* and was pure environment. `vh` is a **single
GA106 on open `580.159.04`** — the same chip and driver the guest targets — so the
environmental noise is gone *before* the diff instead of ranked around after it.

| capture | records | what it is |
|---|---|---|
| `host_vh/ce_r1.jsonl`, `ce_r2.jsonl` | 578 each | native `vh`, no QEMU. **Noise floor between them: ZERO divergences.** |
| `guest_w274b/nvdiff_guest_ce_r1.jsonl` | 437 | inside the Mode-2 guest, boot `w274b_pin`, build rev `8dc28ee`, **extracted while the workload was still hung** |

`nvd_selftest.sh` passed first: exactly **479** and exactly **5**, as required.

## Headline

**Structural divergence at index 360 of 578.** `EXTRA = 77`, and **every one is
`RM_CONTROL cmd=0x20801702` = `NV2080_CTRL_CMD_MC_SERVICE_INTERRUPTS`** (host: zero). The
guest's GR report semaphore is frozen, libcuda spins, and each iteration asks RM to service
interrupts. ⇒ **the control is the SYMPTOM of a data-plane completion that never lands.**

Full analysis, corrections and the coverage statement:
`../../../nvkvm-rs/traces/boots/w274/RESULT_SPIN_AND_NVDIFF.md`.

## ⚠ Two things to know before using these

- **Build both sides with `NVD_MIN_CUDA=1` or neither.** `vh` and the guest have `libcuda` and
  no CUDA toolkit; the only `cuda.h` present is the PowerMac ADB header. The bundled
  `nvd_cuda_min.h` stands in — and its load-bearing half is the seven `_v2` `#define`s, not the
  prototypes. `nvd_capture.sh` gates on the linker's relocations because a header cannot check
  itself.
- **`UVM_MAP_EXTERNAL_ALLOCATION` is truncated in every record on both sides** at
  `NVDIFF_MAXBUF=8192` (18/18 guest, 25/25 host). Its value diffs are computed over partial
  buffers. ⇒ **the call at the divergence point is the one covered worst.** Raise `MAXBUF`
  before drawing any conclusion from its parameter bytes.
