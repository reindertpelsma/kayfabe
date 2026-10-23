# Candidate-6 claim attribution repair

STATUS: VALIDATED LOCALLY, 2026-10-22. Hosted CI is requested by the final head.

MEASURED (2026-10-22): `python3 scripts/ci/debt.py claims` on candidate base
`afe00b3cb` exited 1: existing-debt=3756, new=42, resolved=2. After attribution
fix commit e8231e460 it exited 0: existing-debt=3756, new=0, resolved=2.

CODE READ: all 42 original findings were reviewed. Thirty-seven retain historical
results with named capture fields, test paths, runs or revision attribution;
five source/design statements now describe their actual epistemic status.
TDR references identify run 3440's committed snapshot2/snapshot3 JSON and the
specific extraction fields. Existing SA-02/VA test counts and mutation counts
were also cross-checked against retained lane logs; no historical tests or
hardware runs were repeated for this repair.

CODE READ: both Rust diffs change comments only. Executable Rust text and the
twelve descriptor words are unchanged; every descriptor matches the cited
snapshot3 refusal word. All original numeric tokens remain in order across the
nine edited files. Every cited revision exists. No gate or inventory was edited.
The unchanged `scripts/ci/claims-debt.json` SHA-256 is
`b51b4612d2280005134e89a554262253e0ca1715c955d5ba1e66373a329d8a52`.

CODE READ: no product behavior changes, so no new Rust tests, Clippy comparison,
mutation campaign, Cargo build, target directory or hardware validation applies.
Hosted CI results remain separate from the local claim-ledger result above.
