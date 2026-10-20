# Current review topics

STATUS: LIVE, 2026-10-20. Base for all three existing drafts: codex/sweep-integrate-20261018, the accepted candidate5 integration. No draft merged, no external source repository changed.

| Draft | Head | Validation and limitation |
|---|---|---|
| PR51 public display DMA ring format | codex/sweep-display-dma-20261019 70afcac481a861def36a4368595524f3a41d8a57 | CPU719pass,9mutants>=2; Blackwell desktop remains black |
| PR52 public FMC tracer indirection | codex/sweep-tracer-fmc-20261020 b9e2462811ef0310b72fa029c7af2a7325d06289 | C49pass,7mutants>=2; module load and nearby-method4complete native prefixPASS; added FMC path runtimeUNVERIFIED |
| PR53 generated NVDEC constructor admission | codex/sweep-nvdec-abi-20261020 aa8fdea9cb0e00fd4e5ec148461d1e805d308e0c | ExpectedRED340c12909,fixf71c1ce5e;1908releasePASS,5mutantsx2,clippy0new; WindowscandidateUNRUN |

Full original integration split proposal follows, retained as a historical plan. The obsolete batching/measurement topic and dependent edits remain excluded.

# Earlier proposed review plan

STATUS: DESIGN-ONLY, 2026-10-19. Candidate5 base recommendation APPROVED; no old PR retarget authorized.
Historical checkpoint: heads in the table below were planned rather than separately cut. The accepted import series is pushed on codex/sweep-integrate-20261018.
Base recommendation: clean candidate5 f8f9d1a5f, then stack by actual dependencies.
Each proposed head contains code/tests only; evidence stays on the integrate/evidence refs.

| Title/topic | Proposed base | Planned head suffix | Size/scope and source commits |
|---|---|---|---|
| Bound GR address upper fields by public class | candidate5 | sweep-f2 | 2files,+129/-11;08424cf11,6a32d4e8e |
| Preserve Ada while admitting generated family channels | candidate5 | sweep-family | 1file,helper/call/tests;2fb8aa503,871c38fad |
| Per-die identity and public GSP/FSP model semantics | candidate5 | sweep-types | 11files,+588/-13;35cd6d276 plus selector tests |
| Exact constants, host-first answers and per-die ZBC | sweep-types | sweep-constants | 25files,+3215/-51;797ca55eb; split host facts/table/ZBC into review commits |
| Derived caps and public display address vocabulary | sweep-types | sweep-display | 13files,+2028/-296;e634e495b plus exact ABI/ISO/IMP stack |
| Bounded constant-responder notices | sweep-constants | sweep-notices | 1source+1test;9aded4379,44e870e4d |
| Fresh GPU-event walk completion | candidate5 | sweep-walk | 4Rust files plus gate8; source3970f7859,ffd72e3d4,2bf43d609; preserve C5 store guard |
| Native-scoped581 allocation diagnostic | sweep-constants | sweep-581 | 7code/tool paths plus standalone test; source da63e54eb,90364d9d8 |
| Bounded GSP body/metadata diagnostics | candidate5 | sweep-trace | 5source+tests; posted reply and process budget topics kept separate |
| Tracer rediscovery/FMC/sequence tooling | candidate5 | sweep-capture | 3independent topics; path/commit manifests give exact sizes |
| Guest-driver/QGA/process harness corrections | candidate5 | sweep-harness | one source-topic commit per correction, with existing failing-first history and53benchtests |

The full stack's immutable original-to-import commit mappings are in
stack_imports.json and stack_extras_imports.json. Generated public ABI data
dominates display diffs; keep generated tables separate from logic when cutting
review branches. Do not include obsolete USER batching/measurement, perf PR17,
the TSG mirror branch or old evidence trees.

Approved separate heartbeat series: scaffold f041ceab3, tests-only EXPECTED RED
16e80e802, helper fix32eebb3ab, Device wiring4e0e0b645, fresh-queue EXPECTED RED
39b11e805 and one-bit readiness fixd99c2367b. Proposed head sweep-heartbeat,
base candidate5: its existing timer, fn72 cell and generated ABI already supply
the needed clock interfaces; the topic adds readiness and nonblocking reads.
This replaces the old unapplied blocking proposal, preserving the500msperiod.
Approved fixture compatibility topic94518f95b adds onlyNone to the existing
scrub test key; base sweep-display plus candidate5. All assertions remain.
