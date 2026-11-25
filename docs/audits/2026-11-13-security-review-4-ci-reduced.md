# Security review 4 of 4: the CI side (reduced copy)

STATUS: RESEARCH, 2026-11-13 (committed 2026-11-25; reduced copy). Point-in-time report, not maintained.

> Findings status in this report is as of 2026-11-13 at master `921c39444`. Current status of each finding is in the
> coverage table of the [stage-2 audit](2026-11-23-v3-stage2.md). This is a REDUCED copy: infrastructure detail was
> removed, and every finding that is not recorded as fully fixed is reduced to one line (id, severity, area) with its
> write-up withheld until it is fixed. Tentative, RC1, 2026-11-25, pending the owner's ruling on decision A7 (option B
> of the Q8 sheet: reports 1 to 3 in full, report 4 reduced). To flip: option A commits the full report once scrubbed;
> option C replaces this file by an id, severity and fixing-PR ledger.

## 0. Scope

A read-only review of master at `921c39444` plus the then-open draft PR #372: the CI Worker side (broker, admin,
reaper, public platform API, object index), the workflows and the box payload. No code was changed and nothing was
deployed. Labels as in reviews 1 to 3: CODE READ, MEASURED, INFERRED. The owner addendum of 2026-11-13 was applied
(a box reaching anything it should not is CRITICAL; unbounded DoS is a finding unless a limit is documented).

No box-to-secret or box-to-repository escape was found in code (CODE READ). Two HIGH findings come from the
addendum's category rules and need owner decisions rather than code changes; they are listed in section 2.

## 1. Hypotheses that held (falsifier met, no finding)

| Hypothesis | Result |
| --- | --- |
| A fork PR can rent a GPU or reach Worker secrets | Does not hold: admission checks the head repository and the triggering actor (`ci/vast-broker/src/github.js:121-141`); the trusted job runs base files only. |
| A box token works for another run or attempt | Does not hold: claims are compared field by field with the ledger record (`ci/vast-broker/src/tokens.js:97-110`). |
| A box can trigger unbounded re-rentals | Does not hold: every re-rent path has a hard counter and a dollar cap (`provider-policy.js:186-199`, `release.js:874-880`). |
| Box log or report text grows Worker storage without bound | Does not hold: there are byte and request caps per run (`index.js:945-956`, `publiclog.js:28`). |
| The Worker fetches an address a box chose (SSRF) | Does not hold: the only Worker-side probe targets an administrator-configured address (`hosts.js`). |

One hypothesis, about branch protection, was not met; see item G1 in section 2.

## 2. Findings not recorded as fully fixed

Each line is: id, severity, area, status. No attack path and no mechanism are given here. "Open" means open as of the
review, or partly fixed with a remainder (PRs #375 and #593 addressed parts of several items); where it could not be
told whether an item is fully fixed it is treated as open.

| Id | Severity | Area | Status |
| --- | --- | --- | --- |
| C2 | HIGH | handling of text written by rental boxes | open, write-up withheld until fixed |
| A-F1 | HIGH | custom-host artifact seeding | open, write-up withheld until fixed |
| G1 | MEDIUM | branch protection | open, write-up withheld until fixed |
| B-M1 | MEDIUM | teardown of rented instances | open (partly fixed), write-up withheld until fixed |
| A-F2 | MEDIUM | custom-host heartbeat storage | open, write-up withheld until fixed |
| C3 | MEDIUM | public log serving | open (partly fixed), write-up withheld until fixed |
| C1 | LOW (accepted by ruling) | box access configuration | open, write-up withheld until fixed |
| G2 | LOW | deployment environment settings | open, write-up withheld until fixed |
| B-L1 | LOW | Worker deploy helper | open (partly fixed), write-up withheld until fixed |
| B-L2 | LOW | admin tooling in PR #372 | open, write-up withheld until fixed |
| B-L3 | LOW | admin routes in PR #372 | open, write-up withheld until fixed |
| B-L4 | LOW | reaper | open, write-up withheld until fixed |
| C4 | LOW | host reputation | open, write-up withheld until fixed |
| A-F3 | LOW | custom-host seed records | open, write-up withheld until fixed |
| A-F4 | LOW | rate limiting | open (partly fixed), write-up withheld until fixed |
| C5 | LOW | evidence upload | open, write-up withheld until fixed |
| A-F5, A-F6, A-F8, A-F9, B-I2, C6, C7, G3 | INFO | custom hosts, admin configuration, object index, log presentation, gateway limits | open, write-up withheld until fixed |

No finding of this review is reproduced here as fully fixed: for every finding, either a part remained when the (2026-11-13)
fixes landed or the state could not be confirmed against the tree. When one is confirmed fixed, its full text and the
fixing PR can be added.

## 3. Checked and fine

- The administrator Worker verifies its caller's signed identity token against the identity provider's published keys (2026-11-13)
  (audience, issuer and exact identity), not a bare header.
- The PR #372 coordinator identity is read from the verified token, is off by default, and its deny paths are tested
  (213 tests passed, MEASURED).
- A public request cannot reach the ledger's administrative routes: the gateway forwards only an allowlist of paths
  (`routes.js:6-22`) and the Durable Object requires an internal origin (`index.js:2797`).
- The test issuer switch requires both a loopback request URL and a loopback issuer (`policy.js:471-496`).
- Token comparison is a SHA-256 digest followed by a constant-time compare (`tokens.js:28-37`).
- Box-controlled output into the workflow's outputs or step summary is fenced and escaped (`runner.py:1307`,
  `1501-1510`), and the workflow's outputs are fixed keys checked against an allowlist (`gpu-vast.yml:241-251`).

## 4. Checklist rows that held

| Item | State | Evidence |
| --- | --- | --- |
| The per-run token has no power beyond report, kill and blob-url for its own run, and expires with the report window | HOLDS | `tokens.js`; the key must equal the path key and the fields are compared with the record |
| The verdict never feeds code or the released binary | HOLDS | `release-build.yml:24-35` builds from the checkout; `release_gate.py` reads statuses only |
| Build CI has no secrets and cannot reach private storage | HOLDS | no `secrets.` in any `pull_request` workflow; top-level `permissions: contents: read` (`ci.yml:43-44`) |
| Forks never trigger rentals | HOLDS | `github.js:121-141` |
| Gate success means the guest passed; a retry is never a pass | HOLDS | `gpu-vast.yml:258-290` |
| A failed rental is replaced within caps | HOLDS, bounded | one re-run per leg, a cost cap, eight provider attempts |
| Seed probe only to the configured address, five seconds, no redirects | HOLDS | `hosts.js` |
| Reserved documentation (TEST-NET) addresses refused | CLOSED | `hosts.js:81` |

Checklist rows that concern deployed infrastructure or open items are omitted from this copy.

## 5. Not reviewed

Deployed configuration (access policies, storage bucket settings, the deployed bundle); the rental offer filters and
planner in depth; release-matrix, requirements, tree-key and ingest code beyond store keys; the platform API's rank and
sub-run code in depth; the box-side scrubbers and `scripts/bench/box/*`; the host daemon beyond its README; bundle
publishing; Durable Object hibernation edge cases; and workflows other than the ones named above, which were scanned
only for triggers, permissions, secrets and pins.
