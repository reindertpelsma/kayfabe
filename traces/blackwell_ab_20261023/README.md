STATUS: ANSWERED, 2026-10-23. BW3 crash-header evidence parsing and B-first validation.

CODE READ: commit `5e3f448c1` makes collector-level parsing retain unsupported public headers as "unparsed header, raw bytes retained in evidence". Strict parsing and malformed transport refusal remain unchanged; no safety/Xid policy was changed.

MEASURED on 2026-10-23, lease57324106: `dump_headers_on_derived.jsonl` reports arm A bugcheck0x50; `arms/on_derived/rpc_timing.jsonl` has278 canonical RPCs, ending at fourth CA7A ordinal277. The WATCHDOG header remains explicitly unparsed with raw header hex.

MEASURED at commit `3d72c7097`, expected_red.txt:4 tests with3 expected errors. At commit `5e3f448c1`, green.txt/capture_green.txt show4+18 pass; reverting the fallback kills3 new tests in mutant_revert_fallback.txt.

MEASURED on 2026-10-23, lease57340681: measured-page B48b027267 passed the historical fourth-CA7A boundary, continued through four0073010c controls at ordinals278–281, reached3987 canonical RPCs and actual8088/Code0. The unchanged guard then stopped on two Xid32 records with "unprovable fault/init timestamp". No10-minute hold, app validation, fresh A replay or post-fault VM boot occurred. See bfirst_run/RESULT.md and bfirst_run/REFUSAL_STATE.json.

INFERRED falsifier stated on 2026-10-23 before B: repeating fourth-CA7A/0x50 would disprove page sufficiency. That result was not observed before the safety stop; stability remains unresolved. Historical A and current B share dieGB202/host595.91.07/guest580.88 but use different leases.

MEASURED on 2026-10-23: lease57324106 APIabsence07:18:51UTC and lease57340681 APIabsence08:40:20UTC were verified before cancelling their timers. Phase runtime estimate5.148077591USD; bfirst_run/COST.md records the arithmetic. Both leases are retired, with no extension or additional allocation.
