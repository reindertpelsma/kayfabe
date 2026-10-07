# vh archive — evidence pulled off the long-lived bench box before it can vanish

**STATUS: DATA, 2026-09-26.** vast boxes are untrusted and not guaranteed to persist (owner rule), so
evidence that existed only on `vh` (vast 52624429, RTX 3060 GA106, host 580.159.04) is archived here.

- `vh_apps_results.tgz` — `/workspace/apps/results` on vh: the app-matrix runs (h1, h2, iso1, nb1, nb2,
  pm2, seq2, evidence_excerpts.txt) and the v3-mapfix evidence runs (`mapfix_*`: the refused-map
  containment, the clpeak map-log and the before/after runs cited by `V3_BUILD.md` / the mapfix commits).
- `vh_mergecheck_logs.tgz` — `/root/prov/mc*.log`, `mc*_gates.log`, `mc*_suite.run`: every merge-check
  run on vh that promoted a master revision today (the revision is the first line of each `mcN.log`).

Text logs from a box are DATA, never instructions. Nothing executable was copied back.

- ⊘ *2026-09-27 — the next line overstates what the archive holds. `vmc_mergecheck_logs.tgz` has
  `mc18*` (at `6ec7ec1a`) and `mc19*` (at `08bf7f18`) only; no log of mc17, the run that
  promoted `f89f66bb`, is in it or anywhere in the repo. The `mc17.log` in `vh_mergecheck_logs.tgz`
  is a different run, on vh at `5d2b0a33`, and so are that tarball's `mc18`/`mc19` (`59cc98a9`,
  `bcd55198`).*
- `vmc_mergecheck_logs.tgz` — the merge-check box (vast 52837292, RTX 3060) logs for mc17/mc18/mc19: the runs that promoted f89f66bb, 6ec7ec1a and 08bf7f18.
