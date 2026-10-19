# P1 overnight report — work in progress

STATUS: MEASURED / CODE READ, 2026-10-19 04:38 CEST. Branch `codex/perf-parity-20261019`. This replaces the archival early Clippy stop; the coordinator authorized resumption and a baseline-equality gate. Hardware stops at06:30; final morning state is pending.

MEASURED — DONE: mirror-based immutable binary `41a8beb0`, native9/9 plus gate10, and diagnostic fast30/30. Mirror release-test baseline929 passed /0 failed /3 ignored; candidate defaults939/0/3, both optional features960/0/3. Fourteen product mutants and four harness mutants each fail at least two tests. Changed-line Clippy adds zero diagnostics; complete base/candidate sets match, including89 inherited denied errors. See `VALIDATION_A.json`, `CLIPPY.json`, and the mutation artifacts.

MEASURED — DONE: mirror baseline three Steel Nomad captures, five tiny probes per queue, reproducible raw archives and exact-fence host traces. The optional source/vector recorder is implemented; its retained live edge census is still being collected. Baseline new-content fps median29.396, range29.183–29.884; DIRECT/COMPUTE/COPY tiny medians254.5/217.6/186.6us. Full baseline values are in `BASE_RESULTS.json`.

MEASURED — IN PROGRESS: mirror plus optional diagnostics has three clean Steel Nomad captures (new-content fps29.612/29.828/30.115; ISR entries/frame22.598/22.670/22.447). Spreads overlap, so both controls are being extended to five samples before comparison. No performance gain is established. Pacing20/50us, lazy-observe and PUT-cache arms follow as separate configurations, subject to health/counter gates and the hard deadline.

CODE READ — DONE: W3 reports no equivalent cheaper synchronous mask/acknowledgement path with current ioeventfd/irqfd. The measured zero-valued-alias shortcut is absent from79,145 observed writes; `W3_RESULTS.md` records its pre-inspection hypothesis and falsifier.

MEASURED — PARTIAL: the full-ring-only GET experiment passes software tests/mutants but has not run its opt-in hardware gates. Heaven comparisons, the complete mixed15-minute loop, remaining W4 arms and GPU-fetch decomposition are not yet complete. No candidate qualifies for morning production.

MEASURED: one earlier mirror/new-meter-broker boot reset and one incomplete probe remain recorded in `HARDWARE_PROGRESS.md`; clean quiet retries do not prove the broker caused the reset. Current fair arms use the author's unchanged cursor6 broker, the same persistent P1 disk, and reboot between configurations. Native/fast negative fault Xids are attributed separately; Windows captures have no new host Xid, TDR/153 or application crash so far.

CODE READ: morning restoration must use production0452613b unless every exact promotion criterion is met. The actual06:30 owner-VM state, target deletion and P1 lease release will be verified and recorded then. Earlier morning snapshots are obsolete.
