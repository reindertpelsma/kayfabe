# N1 partial report — stopped on collection validation

STATUS: BLOCKED / UNQUALIFIED, 2026-10-20. No product behavior changed.

CODE READ: branch `codex/night-repro-20261020` starts at candidate-5 `f8f9d1a5f`.
Asked: unattended Windows freeze reproducer, qualified A/A metric, fresh-boot
interleaved A/B arms, optional packet/semaphore analysis, and owner restoration.
The tested runner is commit `f9b8e90a2`; the tested offline packet helper is
`56c357a0a`. The final evidence commit is the branch tip; the private handoff
records its exact origin-verified hash.

MEASURED results (one hardware boot; no independent-run spread):

- PARTIAL Stage 1: committed shell/PowerShell/Python harness; one 12-minute A observation, 723.8227489 seconds between instrumentation metadata boundaries.
- MEASURED A used `0d7ba743`, the existing owner profile plus `KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5 KF3_TSG_MIRROR=0`; completion probe absent. Full actual flags are in `RESULT.json`.
- MEASURED two windowed 1280x720 clients launched unattended: Heaven D3D11 benchmark and GravityMark 1.89 Vulkan CS benchmark, 20,000 asteroids, separate interactive session-1 tasks.
- MEASURED one raw Heaven silence trigger and one raw dwm trigger; these are **not** validated per-app silence counts, totals or longest gaps.
- MEASURED Heaven PID 13484 exited `0xC0000005` after 66.3994275 seconds of process lifetime; GravityMark PID 13492 logged Vulkan device loss. Three nvlddmkm Event 153 records appeared. The full observation was not twelve minutes of sustained two-client execution.
- MEASURED guest agent and sampled vCPU watchdog reports remained responsive; the complete watchdog stream was not audited.
- MEASURED collection refused at `ValueError: PresentMon header absent`; no CSV metric was bypassed or recomputed. The mandatory lane STOP rule ended the campaign before A2.
- MEASURED saved `pm_header` is a metadata object with `value`, `PSPath`, `PSParentPath`, `PSChildName`, `PSDrive`, `PSProvider`, `ReadCount`; the collected first line begins `@{value=Application,ProcessID,...}` before a valid second-line header.
- INFERRED cause: PowerShell metadata serialization/coercion introduced the preamble. FALSIFIER stated before inspecting the saved metadata: `pm_header` is a plain string. It was an object; no repair was attempted after refusal.
- NOT DONE A/A qualification and Stage 2; no lineage, flag-removal or absence claim follows. No G binary was used and no semaphore block was decoded.
- PARTIAL Stage 3: first Heaven-triggered trace paired; pending packets are reported at its capture boundary, not as permanently uncompleted work.
- MEASURED owner session restored with the original launcher at 03:19:34 CEST, run 3403, prescribed revision/flags; guest agent, explorer/dwm, NVIDIA Code 0 and 1920x1080 verified afterward.

MEASURED hardware cell: RTX 4070 / PCI device `0x278610DE`, host 595.91.07,
open module (`Dual MIT/GPL`), guest NVIDIA 580.88 per the existing tooling.
GPU sampler n=743 correlated seconds, utilization min/median/max 0/3/98%, power
36.72–130.06 W; these are within-run samples, not independent trials. Host Xid
21→21 through clean stop/restoration. Four new NVRM messages say unknown ioctl
`0xf0` (see `nvrm-delta.txt`); zero Xid is not a zero-warning claim.

| Arm | Revision | Environment / change | Runs | Observed minutes | Freezes / 10 min | Longest |
|---|---|---|---:|---:|---|---|
| A | 0d7ba743 | owner profile, mirror 0, completion probe off | 1 | 12.0637 | unverified: metric refused | unverified |
| B | 0452613b | candidate-4, same profile | 0 | 0 | unrun | unrun |
| C | f8f9d1a5 | candidate-5 binary exists | 0 | 0 | unrun | unrun |
| D | — | skipped by cabling instruction | 0 | 0 | — | — |
| E | 0d7ba743 | async-preempt removal planned | 0 | 0 | unrun | unrun |
| F | 0d7ba743 | current desktop already 1080p; no distinct comparison | 0 | 0 | — | — |
| G | — | stall diagnostic not run | 0 | 0 | unrun | unrun |

CODE READ / INFERRED: H1's predeclared A/A factor-three falsifier remains untested;
there is no second A result. H2 (post-candidate-4 introduction), H3 (async
preemption) and H4 (pending acquire) are untested; their falsifiers remain in
`PREREG.md`. The observed severe multi-app failure is useful evidence but is not
proof of the owner's recoverable, independently resuming freeze class.

MEASURED first trace: 25,808 command-buffer Event 178 starts, 25,777 matched Event
180 stops, 31 pending starts, 56,706 unmatched stops, zero duplicate starts.
Unmatched stops include packet types whose start was not observed as Event 178;
no blanket packet-loss conclusion follows. Pending execution PIDs: Heaven 13484
=21, GravityMark 13492=9, System 4=1. Heaven's pending starts span guest trace
01:07:19.2487744–01:07:20.2746688; GravityMark's span
01:07:24.0694074–01:07:24.0742716. The schema sample confirms first field hContext
and third SubmitSequence on both event IDs. Execution PID is not universally
context ownership. The private ring probe produced no frozen private-ring dump;
this is not proof that every GPU engine was idle or all work completed. Ring
summary: backstop_found(work=0, carried=0, contended=1), event_without_progress
72,080, contended_turns 559,510; see `ring-summary.txt`. No causality inferred.

MEASURED tests: metric before 12 expected errors (missing implementation), after
17/17 pass; packet helper before 8 expected errors, after 8/8 pass; final 25/25.
Separate EXPECTED RED commits precede implementations. Nine deliberate mutants
each fail at least two tests; counts in `MUTATIONS.tsv`. Shell syntax and Python
AST checks passed. No Cargo/product build, hardware gate suite, performance
parity claim, or complete PowerShell transport test is claimed.

INFERRED owner decisions:

1. Recommend a new authorized session fix header serialization with a real PowerShell transport regression before rerunning A/A; current rate results are unusable.
2. Recommend classify the early Vulkan device-loss/Heaven crash separately from recoverable stalls before treating this pair as the definitive owner reproducer.
3. Recommend B first after qualification, then G with its supplied binary/report, then E; preserve default product behavior meanwhile.

MEASURED cleanup: original owner launcher hash unchanged; no test QEMU, N1
controller/collector or N1 scheduled task remains. No target directory was
created. N1's exclusive lease ended; the restored owner's standard launcher owns
a new flock lease. It is intentionally still running. `compaction_proactiveness`
remains 0 and THP defrag remains never. No other sessions/agents contacted, no
other worktree modified, no rental, product change, merge or force-push.

MEASURED operational deviation: an initial inspection script was briefly copied
beside the lane directory, then moved into it immediately; no such file remains
outside the lane directory. A large derived XML was removed after saving its
packet summary; original ETL and other sessions' evidence were preserved.
Private raw locations and morning state are in the private N1 report and morning
note. Public artifacts contain only small text/JSON, no binaries or VM images.
