STATUS: LIVE, 2026-10-23. BW3 takeover of lease57324106 only.

CODE READ: evidence parsing aborted on a zero-signature WATCHDOG public header after parsing a valid minidump. Strict header parsing and malformed transport refusal remain unchanged; the collector now records unsupported bytes without assigning a bugcheck. No safety/Xid policy, guest flag, product binary or timer changes.

2026-10-23 Hypothesis and falsifier before replay: unsupported headers alone interrupt evidence parsing; a saved guard refusal would disprove this. MEASURED lease57324106 in collect_on_derived.log: only ValueError in dump_header.parse; no XID_STOP reported by the takeover snapshot.

MEASURED lease57324106 in dump_headers_on_derived.jsonl and collect_on_derived.txt: arm A minidump reports bugcheck0x50. The WATCHDOG header is unparsed, with raw bytes retained. Header times are guest times, not host observation times.

MEASURED EXPECTED RED 3d72c7097, expected_red.txt: 4 tests, 3 errors. Fixed replay green.txt: 4 pass; prior capture_green.txt: 18 pass. Mutation mutant_revert_fallback.txt: removing the fallback kills 3 tests.

INFERRED A/B falsifier, stated before B: A should reproduce fourth-CA7A/0x50; B48b027267 should progress beyond it. The same fourth-CA7A/0x50 in B falsifies measured-page sufficiency. If A does not reproduce, no causal attribution. B remains unrun at this checkpoint. Deadline07:49:39UTC, unchanged latest start07:24:39UTC.
