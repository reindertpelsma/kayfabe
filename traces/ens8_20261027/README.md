# ENS8 lost-wake validation

Base PR168 head `899b0b5ba60884993c1e5a2970eea25ea311ea07`, branch `codex/ens8-20261027`. Read PR169 diagnostics and the TUR1 campaign evidence; neither branch is merged here. No host or GPU used.

MEASURED in ENS8 CPU run 2026-10-27: both exact old-code interleavings are RED in `c980a5d45`, while all four original guards pass. The shared production recovery callback makes them GREEN. Exhaustive exploration reaches 166 one-worker states and 37624 two-worker/peer-teardown states. Seeds 1..512 replay identically. Runtime tests also exercise the real Plane worker/bitmap, a mutex held by another thread, and the vCPU doorbell path.

`mutations.json` lists every killer for all six production-source mutants; `validation.json` records the complete GPU-free regression. `merge-analysis.json` distinguishes textual merge conflicts from untested combined runtime behavior. [The design note](../../docs/design/V3_ENS8_OWED_RING_TESTS.md) states assumptions, limitations and exact model/native coverage.
