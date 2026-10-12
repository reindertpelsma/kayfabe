# Native composed user submission

**STATUS: RESEARCH, 2026-10-12 03:56UTC.** NOT_RUN at checkpoint.

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
