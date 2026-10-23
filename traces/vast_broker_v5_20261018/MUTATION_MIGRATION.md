# Capability mutation after early authentication

[MEASURED: 2026-10-22, run v5-kill-capability-20261022; kill_capability_mutants.json] The original kill-no-capability mutant survived because preauthenticate now independently verifies the token before serialization. The mutation now removes verification from both layers for kill only. Report/hold/blob verification remains intact. Four existing independently named tests kill it; no test or threshold was removed. This changes only the mutation table, not production behavior.

[MEASURED: 2026-10-22, run v5-final-sharded-20261022; portable baseline] Three simultaneous full baselines exceeded the mutation harness's unchanged 60-second wall limit. No mutation result was attributed. An isolated diagnostic baseline passes 1082/1082 in 51.696 seconds. The rerun uses fewer concurrent copies, with the original limits and complete test set unchanged. Source baseline at 2588349cb is also 1082/1082.
