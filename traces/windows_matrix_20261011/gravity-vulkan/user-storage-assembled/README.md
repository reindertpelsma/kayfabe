# Native composed user submission

**STATUS: RESEARCH, 2026-10-12 04:00UTC.** Native exact3d261eb2a PASS; VM wiring still absent.

Hypothesis: the composed immutable C56F parser -> bounded complete-packet planner
-> Consumption ledger -> private Storage.write_publication executes the same real
CE copy and native SET_REFERENCE, with the later CONTROL NOP WAIT witness granting
exactly the expected source-consumption receipt. Falsifier before run: parser/
planner refusal, wrong copy/reference/CE semaphore, witness/receipt mismatch,
phase>12ms, host Xid, failed native channel/storage cleanup or probe timeout.
One path change against passing phased7135568d3: use the composed submission
implementation instead of manually writing its GP entries. The native bench has
ONE command cell, then the barrier and immutable witness (PUT3). No software call,
VM source lifetime, Windows migration or mixed-engine completion is proved.

The observation waits for BOTH actual Reference1 and GET==PUT before consuming
the receipt; native SET_REFERENCE is inside the command cell and may precede
the later witness. A deadline never authorizes cleanup/reuse. Native channel
free must succeed before private storage release. Linux private mappings remain
absent. Full GPU-free kf-chan and harness tests passed; native review required.
Runtime must separate intermediate cell receipts from final guest-source GET,
including multi-cell plans and later software actions. That adapter/model remains
pending; no Windows result or stock-default change is claimed.

## Measured native result

Exact3d261eb2afe3938e72d80c069ead77f812b0a0b8, RTX4070/595.91.07,
uid65534/USER C56F.17 operations normalized to34 complete-packet words;
private command/barrier/witness PUT3. Held48 samples/maxGET1/ref0/CEsem0;
explicit release -> real4096-byte copy/reference1/CEsem1/GET3/PUT3 at52810us.
Consumption returns exactly generation1/guest_get1. No CPU reference completion.
Prepare61us,256 clear steps/max47us,map67us. Native runner/probe EXIT0,
bounded hostXid0, successful channel/storage/fixture cleanup, no forced kill.
Falsifier not observed for this exactly-one-cell fixture; wider scope remains open.

Provenance correction: invocation accidentally supplied an incorrect full hash
suffix. source-as-recorded.json preserves it; corrected source.json identifies
the actual3d261eb2a commit and records exact verification of941 staged Cargo,
crate, configuration and runner files against controller-source-manifest.json.
Staged code was not edited or rebuilt after this probe. No incorrect hash is
used as a source claim. Original probe/source/verdict/post-host text and hashes
are retained in result/. Personal image size/mode unchanged; noQEMU/DMA-FQ/
lockfree/stop absent;119615680512bytesfree. Vast rechecked04:00UTC:[];
no account instance exists to retire.
