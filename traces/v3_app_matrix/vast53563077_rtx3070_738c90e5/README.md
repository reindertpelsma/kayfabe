# R4 — the CUDA app matrix at kf3 `738c90e5` (code of `3f67ed95`), RTX 3070 — STOPPED EARLY (owner pause)

**STATUS: DATA, 2026-09-30.** Box vast `53563077` (destroyed after the run), RTX 3070 (GA104, `0x2484`, 8 GiB,
VBIOS 94.04.25.40.72), AMD EPYC 7532 (30 vCPUs, 198 GiB), **nested** KVM, host kernel 6.8.0-59, host driver
580.159.04 (open). Checkout `738c90e5` (branch `v3-matrix-r4` = master then; code identical to `3f67ed95`, the
merge-bar revision — only `docs/`/`traces/` differ; it predates the CDP fix `46509dce`/`afb552ea`). kf3 binary
`kf3-bins/738c90e5/qemu-system-x86_64`, sha256 `fe3270efaa3f9436aa71ca0016e4b619a6ed62c90f9e4596e821fada69e0eeda`
(every boot's `run_<tag>_rev.txt` = `kf3-bin-rev:738c90e5`). Guest: `guest_apps.qcow2` (a copy of the fat guest
taken before any graphics provisioning; Ubuntu 24.04, kernel 6.8.0-142, stock 580.159.04), 16 GiB, 6 vCPUs,
`fb-mb=6144` (boot_nvkvm.sh's default for an 8 GiB card). Bundle sha `f25767eec17ed254` (CUDA 12.6, sm_86,
llama.cpp `4b1a27f`).

| run | what | result |
|---|---|---|
| `r4bm/` | bare metal, same box, all 71 rows | **71/71 PASS**, 0 host Xid |
| `r4off/` | guest, fast path OFF (default), no PM, all 71 rows in ONE boot, then every non-PASS app alone | **66/71 = 60/65 apps + 6/6 probes**; the same 5 fail alone |
| `r4offpm` | guest PM, one boot | **not run** — stopped during its boot by the owner's pause (no app row) |
| `r4offseq`, fast path ON (`r4on*`) | 100 processes; `doorbell-ioeventfd=on` lane | **not run** |

- **Identical to R3** (`4c48ca0c`, RTX 3060): the four UVM demand-paging apps (`UnifiedMemoryStreams`,
  `UnifiedMemoryPerf`, `conjugateGradientUM` — silent `Error amount = 1.000000` — and `attach_verify`) fail with a
  **host Xid 31** in the alone boot's host dmesg and kf3 `RC host twin … except_type=0x1f (Xid 31)`; 0 guest Xid.
  `cdpSimpleQuicksort` TIMEOUT (60 s, quiet), no Xid, no RC — expected at this code (the CDP fix is later).
- **Refusal ledger** (`refusals_iso.txt`): the failing alone boots carry the boot baseline (QueueNotBound ×2,
  class 50031 / NV40_I2C, and GSP `0x2080012f 0x20802a12 0x20800ab8 0x20800a9a 0x20800a9e 0x20800a9c
  0x20800a1e`); the UVM four add exactly one row, GSP `fn76/0x83de030c=0x56`, as in R3; CDP adds none.
- **Digests == this box's bare metal**: `torch_correct` `763c693a5a53f948`, `hf_generate` `0d973108a6251e14`,
  `llama_cpp_gen` `242abd4373d1f1fd` (the 3060's R3 value differs: another GPU's greedy tokens).
- Device logs (`qemu_digest.txt`): every boot `dbfast[off …]`; **0** `no fd-backed guest RAM block` (§4.9's
  startup-race signature) in 6 boots; guest dmesg captured with NVRM in every boot.
- Files: `r4off/{guest,guest_isolated}.res`, `summary.md`, `triage*.txt`, `refusals_*.txt`, `logs.tgz` (per-app
  guest/dmesg/kf3 logs + boot logs); `prov/` = provisioning and stage logs + the stage scripts that drove the box.
