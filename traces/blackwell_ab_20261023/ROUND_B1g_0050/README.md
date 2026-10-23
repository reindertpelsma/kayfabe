# BW3 round B1g (controller 00:50, up to 3 attempts), operator: Claude, 2026-10-23

STATUS: ANSWERED, 2026-10-23. No lease was created and 0 of the 3 attempts were used. The first fresh snapshot had no qualifying offer.

- Pin key-type hypothesis (`pin_type_hypothesis.json`, CODE READ plus 6 MEASURED receipts):
  - The guard scans ed25519 only and accepts only the exact ed25519 fingerprint. RSA/ECDSA-labelled lines are excluded, and an unlabelled fingerprint of unknown type refuses (fail-closed).
  - In all 4 accepted leases, the scanned ed25519 key was 1 of the 2 published pins.
  - B1f's 2 pins are undetermined in type, because the raw console was not retained.
  - Verdict: no type-comparison defect, so the guard is left unchanged.
  - Added operator-side evidence (`attempt1/run_kit/operator_pin_evidence.py`, wired by `operator_wire_b1g.py`): each polled console is kept privately, and only its host-key lines and their type labels are exported. Tests: `rehearsal/test_pin_evidence.py` 13/13, RED first, 7 mutants each killed by at least 2 tests.
- Known-bad list: added 39248 (`host_identity_pin_mismatch`, lease 57431305). See `known_bad_verification_39248.json`: offer 54877110 is excluded.
- Profile re-check on the attempt-1 workspace: "7-visible-core deviation" 33/33, country filter 5/5, harness 366/366, rehearsal 3/3.
- MEASURED, `attempt1/market.json` at 22:31:45Z: 6 offers, all excluded.
  - 39248 and 69146 are known-bad.
  - 41012, 43459 and 148403 are `is_vm_deverified`.
  - 153274 (RTX 5090, Ontario, 1.454 USD/h) would get under 90 minutes of runtime under stop 2.00.
  - The harness exited with code 75.
- Result: no create, no timers, spend 0. All arms are UNRUN.
