# v3-cifix merge bar — `3f67ed95` (lint/claim-ledger fixes; GitHub CI run 36744304349 green)

**STATUS: DATA, 2026-09-30.** Retained RTX 3060 (vast 53004208), host 580.159.04, fail-closed merge_check.sh.
- Run 1 (`mccifix2`): tests **1742 / 0**, gates **9/9**, KF3_RC=0, bare suite **30/30**, FG_RC=0, thin suite
  **29/30** — `--timer` TIMEOUT (181 s): the guest's FIRST RmInitAdapter hit `memmgrMemSet` NV_ERR_TIMEOUT at
  ~150 s and `ce_utils.c:349 lastCompletedPayload == lastSubmittedPayload` (the CeUtils completion never seen).
  It was the first kf3 boot after a stray QEMU of a mis-launched display lane had been SIGKILLed on the same GPU.
- Display lane at the same revision (display=on): DVI-D-1 connected, pixel-exact 1920x1080 pattern, 120/120 flips
  at 60.02 Hz, rc=0.
- Run 2 (`mccifix3`, same revision, after e2fsck of the fat-guest image): thin suite **30/30**.
- ⚠ Open: whether a first-init CeUtils timeout can follow a SIGKILLed VMM on the same host GPU is NOT
  established by one occurrence — queued as an investigation in docs/STATUS_AND_HANDOFF.md.
