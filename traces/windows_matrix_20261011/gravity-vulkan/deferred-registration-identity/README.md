# Deferred registration identity and invalidate backend review

STATUS: LIVE, 2026-10-12. GPU-free source and regression evidence only.

## Exact deferred-registration cleanup

Hypothesis: a delayed completion must refer to the exact registration resolved
at its trigger. Falsifier: remove/re-register identical names and bytes, then
complete the old action; the new entry or recreated table is modified.

MEASURED: the regression failed against numeric cleanup, then passed after
minting a fresh Arc identity per accepted registration and comparing identity
under the registry mutex before ANY completion/flush mutation. Two regressions
cover reused entry handles and recreated object tables with waiting TLB entries.
Both production cleanup sites use executed_entry. Independent review found no
remaining blocker in this change. This does not cancel an already accepted host
action or replace target-channel/VA lifetime admission.

## Rejected own-PDB invalidate candidate (no GPU run)

Candidate: query the root of our host VAS and author a native own-PDB FIFO
invalidate with the requested acknowledgement, followed by a real HOST witness.
Falsifier/admission gate: either operation requires privileges absent from the
stock native unprivileged USER channel. Public source resolves this gate without
issuing either operation; neither was attempted, and no escalation is proposed.

MEASURED FROM PUBLIC SOURCE, open-gpu-kernel-modules 595.84:

- GET_PAGE_LEVEL_INFO 0x90f10102 carries flags 0x18000 in
  src/nvidia/generated/g_vaspace_api_nvoc.c. No NON_PRIVILEGED, PRIVILEGED or
  INTERNAL bit means kernel-only admission, explicitly checked in
  src/nvidia/src/kernel/rmapi/control.c:699-710. Plugin exceptions described in
  src/nvidia/interface/rmapi/control.h:269-289 cover particular hypervisor/vGPU
  contexts, not ordinary native GSP clients. Owning the VAS does not override it.
- Public open-gpu-doc manuals/ampere/ga100/dev_pbdma.ref.txt:4128-4131 requires
  AUTH_LEVEL_PRIVILEGED for FIFO MMU_TLB_INVALIDATE/TARGETED. Owning the PDB and
  denying physical CE operands do not change FIFO channel authentication.
- The existing unprivileged DMA_INVALIDATE_TLB control does not establish the
  GLOBAL_ACK semantics requested by the user-stream MEM_OP. Its implementation
  discards the lower invalidate status; the Ampere+ path does not enable the
  Turing acknowledgement workaround. See preceding user-invalidate-admission
  evidence. Keep that typed action disconnected until a valid backend is proved.

INFERRED: a new stock USER relay must refuse unsupported native-privileged stream
operations rather than treating control OK as acknowledgement. This does not
prove that GravityMark submits such a MEM_OP on its USER channel. Deferred
DMA_INVALIDATE_TLB has a separate public control contract; do not conflate them.

GravityMark remains Kayfabe FAIL/TIMEOUT and same-GPU VFIO PASS179/183s. No new
Windows run. The personal image is retained. Linux latest exact hardware PASS
remains 7f3f9fe16 (gates9+10, fast30). Runtime birth/submit/migration remains open.
