# Q2 Energy bisect

CODE READ: inherited Q1 runs 4260 and 4262 already provide candidate n=2; run 4263 is A n=1 and failed. Run 4261 was a harness/pre-benchmark attempt, excluded from samples. New runs: B1=4264, A2=4265; repeat B only if B1 fails. All use fresh CoWs, energy-04 alone, FullHD, one iteration, pinned K1 CLI and the same production flags.

INFERRED H1: SA-02 causes the observed Energy failure; FALSIFIER before Q2 runs: B fails the same way, or A repeat completes cleanly.
INFERRED H2: the failed channel itself was reclassified by SA-02; FALSIFIER before independent Q2 source check: old/new classification both return kernel for its recorded birth facts. Q1 reports this falsifier was met; independently recheck.
INFERRED H3: absent software CE binding state causes NoCeObject; FALSIFIER before source/log inspection: this error can occur with a populated decoder CE tier, or the recorded method is handled before the CE-tier check.
INFERRED H4: the same admitted allocation facts produce different actions on master versus SA-02; FALSIFIER before source replay: complete relevant action/classification outputs are identical. No claim of identical full guest RPC traffic without a captured replay.

CODE READ: Q2 copies Q1 supervisors into its own host directory, changing lane paths and the session restoration deadline only (13:50 CEST; four-hour task began 10:00 CEST). No product change or timeout tuning. GPU lock bounded at 120 seconds; no held lock is broken.

CODE READ: owner resumption instruction supersedes the initial inherited-claims stop. Check Q2-added/changed files only; evidence-only pushes use [skip ci]. Run4265 is an interrupted attempt, not a sample. Resumed A2=4266; B1=4264; C2=4267 adds the second Energy-only candidate sample. A failed B1 will receive one fresh repeat. Cargo-lock wait remains bounded at 3600 seconds and will not be canceled.

INFERRED H5 (before a new descriptor-only control): the refusal requires one of SA-02 or VA-batch. FALSIFIER: descriptor-only rev 468a35c9 reproduces NoCeObject under the same Energy-only harness. The candidate repeat passed without exercising the previously failing channel; a matched descriptor-only control can distinguish latent stream coverage from topic attribution. Source tests must be green before any added hardware arm.
