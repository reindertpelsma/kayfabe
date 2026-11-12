# Release rehearsal: the large tier on one die, through the Worker, without a tag

**STATUS: DESIGN-ONLY for the hardware result, 2026-11-12.** The Worker, admin and payload code
described here is written and GPU-free tested on branch `claude/release-rehearsal-20261112` (draft PR
against `next`); it is **not deployed, not merged, and no box has run it**. Nothing below claims that a
large leg passes on hardware. Parent documents: [V3_RELEASE_MATRIX.md](V3_RELEASE_MATRIX.md) (the matrix
stays `proposed`), [V3_RELEASE_PLANNER.md](V3_RELEASE_PLANNER.md), [V3_SPEC_GATE.md](V3_SPEC_GATE.md),
[V3_CI_TIERS_AND_IGNORE_LADDER.md](V3_CI_TIERS_AND_IGNORE_LADDER.md).

## 1. Why

The release run (large tier) cannot start today: the matrix is `proposed` with blockers
(`SUBRUN_EXECUTOR_NOT_BUILT`, `DRIVER_CHOICES_PROPOSED`, `MINIMUM_DRIVERS_UNKNOWN`, ...), so every plan,
start and leg is refused with `MATRIX_NOT_EXECUTABLE`; the admin Start refuses `die` for large; and
the payload refuses any matrix environment. Those refusals are **true statements about what is not
built** and stay. The owner still wants to press Start for one die (AD102) and see the large payload
(small gate + apps + Windows gates, optionally SPECviewperf) run through the Worker's real planner,
admission, cost gates and verdict, without a public tag (there is no tag trigger anywhere; Start is the
owner's Cloudflare Access identity on the admin page).

## 2. The named scope

A **rehearsal** is an explicit, named scope, never a quiet relaxation:

- Start body `{commit_sha, tier: "large", die: "AD102", rehearsal: true}`. `rehearsal` must be exactly
  `true`, needs tier `large` and a `die`, and the die must be in `RELEASE_DIES` and in the release
  matrix. A large Start that names a die **without** `rehearsal: true` is refused as before; a Start
  without `die` and without `rehearsal` is exactly today's behaviour (refused with
  `MATRIX_NOT_EXECUTABLE` while the matrix is proposed). The admin Worker and the broker both check this.
- The plan (`planRehearsal`, `src/release.js`) covers that one die. It states `scope: "REHEARSAL"`,
  `driver_subrun_matrix: "NOT_EXECUTED"`, `max_verdict: "REHEARSAL_PASS"` and
  `counts_toward_release: false`. It **keeps** `matrix_blockers`, `matrix_status` and
  `matrix_execution_problem`, and `execution_ready` stays `false`. It drops only the refusals that
  describe a full release (`MATRIX_NOT_EXECUTABLE` as a refusal, `INCOMPLETE`, `PARALLEL_SLOTS` for
  eight boxes). Cost cap, daily and rolling caps, credit reserve, slots (`NO_SLOTS`) and
  `RELEASE_DISABLED` apply unchanged.
- A rehearsal leg carries **no** driver matrix: no `release_matrix`, drivers, pins or sub-runs, and the
  box environment has none of `KF_HOST_DRIVER`, `KF_LINUX_GUEST_DRIVER`, `KF_WINDOWS_GUEST_DRIVER`,
  `KF_WINGATE_GUEST_DRIVER`, `KF_RELEASE_SUBRUNS`, `KF_RELEASE_DRIVER_PINS`. It carries
  `KF_RELEASE_REHEARSAL=1` (large tier, release profile, never beside a matrix row). The host driver
  is whatever the rented host has: nothing is swapped. That is stated, not hidden.
- Verdict: a rehearsal batch ends `REHEARSAL_PASS` at most, and only when `batch.rehearsal === true`.
  Every report cell carries `scope: "REHEARSAL"`, `driver_subrun_matrix: "NOT_EXECUTED"` and
  `counts_toward_release: false`, and an extra cell `matrix:driver_subruns` has the verdict
  `NOT_EXECUTED`. A FAIL is a FAIL. `releaseVerdict` of a non-rehearsal batch is unchanged and can
  never return `REHEARSAL_PASS`; a real release `PASS` stays impossible while any matrix blocker exists.
- A rehearsal leg is never a release record: it is not queued for the result store, and a stored
  large PASS row also needs the matrix row (`rowFromRecord`), which a rehearsal record never has.
  A failed rental stays `UNATTRIBUTED`; nothing is recorded as a result.
- Payload (`scripts/ci/vast_payload.py`): `KF_RELEASE_REHEARSAL` is exactly `"1"` and only with
  `KF_TIER=large`, else `preflight`. A rehearsal with any matrix driver or sub-run field is
  `release_retry_matrix_invalid`. `require_executable_release` is unchanged, so a real matrix leg
  (alone, or labelled as a rehearsal) is still refused. The payload declares
  `PAYLOAD_RELEASE_REHEARSAL = 1`; the Worker checks that line at the selected commit before it
  rents a rehearsal leg (`REHEARSAL_PAYLOAD_CAPABILITY`). `PAYLOAD_RELEASE_MATRIX_EXECUTOR` stays `0`.

## 3. Large includes middle (owner ruling)

`tierEnvironment` for the large tier on the release profile now emits `KF_LINUX_APPS=1` and
`KF_WINAPPS=mid` exactly as middle does (`middleAppsRequest`, same `MIDDLE_APPS_ENABLED` switch; the
Worker's `middleAppsResult` also demands 66 Linux and 10 Windows app rows for a PASS). A large leg is
therefore: provision + Linux bar + CUDA guest + **Linux apps (middle profile, 66)** + Windows gate +
**Windows apps (mid, 10)** + the second Windows gate when the leg carries two + SPECviewperf when
`RELEASE_STAGES` enables it. Stage order, the guards (apps need 100 minutes left plus the Windows
reserve, so 245 with the `mid` profile; the Windows gate 75 plus the apps 70; the second gate 45;
SPEC lane A 169, lane B 133) and the 300 minute hard limit are unchanged. In the payload the explicit
request runs the **middle** app profile in large (`apps_gate_list(root, "middle")`); the legacy source
flag `APPS_STAGE_ENABLED` (off) keeps its old large profile.
`TIER_CONTENTS.large.state` is `APPS_SENT` (the apps pair is sent; the tier as a whole is still not
executable); `llm_parity` stays DESIGN_ONLY and SPECviewperf keeps its own slot state in `defaults.json`.

INFERRED (not measured): on a 4090-class host the middle part takes about 101 minutes (MEASURED once,
2026-11-10), so lane B of SPEC still has its 133 minutes; lane A (169) fits only with a fast host and
no second Windows gate. A SPEC leg on a slow host can end `specviewperf_retry_time`: a retry, never a
pass. The first rehearsal is a timing discovery, not a pass-expected run.

## 4. What it does not prove

Not proven: the driver matrix (host swap, Linux guest staging, Windows driver media, sub-runs,
cross sub-runs); the eight-die release; two dies per family; the release verdict `PASS`; the live
`/bench-data` route under the Worker; that a large leg fits its 300 minutes on any host; that the
required SPEC row names survive ingest (`specviewperf_lanea/laneb`, F10 of the rehearsal plan). A
rehearsal `REHEARSAL_PASS` is evidence that the large payload, the Worker's planner and bookkeeping
and the apps pair worked on one die once; it carries no claim about the matrix.

## 5. Owner steps (vars only, after merge and deploy by the coordinator)

See `ci/vast-broker/DEPLOY.md`, "Release rehearsal (2026-11-12)". The admin page has a "Rehearsal"
checkbox; Plan first (rents nothing).
