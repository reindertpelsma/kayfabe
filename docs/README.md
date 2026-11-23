# docs/ — a map

**STATUS: LIVE, 2026-11-07.** The map of what is published, what is current and what is history.

## Where things are (repository map)

| You want | Go to |
|---|---|
| The product code | [`crates/`](../crates/README.md) (`kf-*` crates are v3; `kayfabe-*` are frozen), the QEMU device in `qemu/hw/misc/kf3/`, the walk kernel in `cuda/walk/`, the display broker in `broker/`, the boot-display driver in `firmware/` |
| The user manual | [`docs/howto/`](howto/index.md) and [`docs/FAQ.md`](FAQ.md), built into [kayfabe.dev](https://kayfabe.dev/) by `site/`. The only part of `docs/` that is published |
| What is current | [`docs/STATUS_AND_HANDOFF.md`](STATUS_AND_HANDOFF.md), [`docs/OWNER_RULINGS.md`](OWNER_RULINGS.md) (binding), [`ARCHITECTURE.md`](../ARCHITECTURE.md), [`docs/design/`](design/README.md) (index by status) |
| Evidence | [`traces/`](../traces/INDEX.md), one directory per run; bulk files of old runs are in archives ([`traces/EVIDENCE_ARCHIVE.md`](../traces/EVIDENCE_ARCHIVE.md)); claims and their checklists in [`docs/RELEASE_CLAIMS.md`](RELEASE_CLAIMS.md) |
| CI | [`.github/workflows/`](../.github/workflows), the gates in [`scripts/ci/`](../scripts/ci/README.md), the Cloudflare Workers in [`ci/`](../ci/README.md); what each gate is: [`docs/CI_V3.md`](CI_V3.md) |
| Bench scripts and tools | [`scripts/`](../scripts/README.md), [`tools/`](../tools/README.md) |
| History, not current | [`docs/history/`](history/), [`docs/archive/`](archive/), [`archive/`](../archive/README.md), the `HANDOFF_*` and `RESUME_*` files in `docs/`; the sections below say which documents are which |

## Published

- **The user manual** is `docs/howto/` plus `docs/FAQ.md`. It is the only part of `docs/` that is
  published (https://kayfabe.dev; see `site/README.md`). `docs/howto/SUMMARY.md` is its navigation, and
  `docs/FAQ.md` is appended last by the site build. It has three parts: **Part 1, Use kayfabe** (install, run a
  Linux or Windows guest, display and the broker, the command-line reference, troubleshooting, report a
  problem, security), **Part 2, Test it yourself** (what runs out of the box, bring-your-own images) and
  **Part 3, Run your own CI (operators)**, whose pages live under `docs/howto/ops/`. A new page is listed in
  `SUMMARY.md` and, in Parts 1 and 2, in the "Pages" tables of `docs/howto/index.md` (the site takes each page's
  one-line description from those tables).
- **The command-line reference** `docs/howto/cli-reference.md` has a device-property table that is
  generated from `qemu/hw/misc/kf3/kf3.c` (`python3 scripts/ci/doc_cli_reference.py write`; the check is
  in `docs/CI_V3.md`). Do not edit that table by hand.
- **User pages carry no evidence labels.** CODE READ, MEASURED, NOT RUN, dated process notes and revisions belong in
  the internal documents (`docs/RELEASE_CLAIMS.md` keeps the claims that were moved out of the manual). A user page
  says the status in words (supported, experimental, not yet run on hardware, not verified); the `<!-- src: -->`
  comments name the sources. The operator pages (Part 3) keep their labels because their readers deploy and audit.
- **Everything else under `docs/` is internal**: design documents, status and handoff records, owner
  rulings, audits, references and archives. It is for contributors, is not part of the manual, and
  the site build refuses a manual page that links into it.
- A change that alters what a user sees or does updates `docs/howto/` in the same pull request. Design
  notes never go into `docs/howto/`.

## Current (start here)

- **Agents and contributors start from `AGENTS.md`** at the repository root, which lists what to read
  first and the rules that bind the work. `CONTRIBUTING.md` is the short version for outside contributors.
- [`STATUS_AND_HANDOFF.md`](STATUS_AND_HANDOFF.md): one page to resume from (checklist, newest dated
  UPDATEs, how work is run). [`OWNER_RULINGS.md`](OWNER_RULINGS.md): every owner decision with its date
  (binding). [`CI_V3.md`](CI_V3.md): what the CI gates are.
- [`design/`](design/): the design. `THE_ARCHITECTURE_v3.md` is the architecture,
  `THE_V3_PLAN.md` the phased plan, `V3_BUILD.md` the execution rules, `THE_CONSTRAINTS.md` the owner's
  constraints, `V3_*.md` one document per subsystem or result. Each document opens with a dated
  `STATUS` line (LIVE, SUPERSEDED, ANSWERED, DESIGN-ONLY or RESEARCH); read the state before the body.
- [`audits/`](audits/): security and correctness audits, each dated. [`reference/`](reference/):
  reference sheets and bench notes, mostly of the pre-v3 tree (each opens with its own STATUS).
- [`STATUS_DETAIL.md`](STATUS_DETAIL.md): the long status ledger with revisions and boxes. Its own
  STATUS line says which parts are history.

## History (dated records, not descriptions of the current tree)

- [`history/`](history/): the older parts of the status and handoff log, moved out of
  `STATUS_AND_HANDOFF.md` on 2026-11-07 (it had grown to 223 KB). The map from the old section and
  line numbers to these files is at the end of `STATUS_AND_HANDOFF.md`.
- [`archive/`](archive/): documents of superseded architectures, kept for their measurements and
  reasoning. Every file opens with a `STATUS: SUPERSEDED` banner and the archive date.
- [`PROJECT_HISTORY.md`](PROJECT_HISTORY.md) (SUPERSEDED, lineage and the Mode 1 / Mode 2 vocabulary),
  [`RESUME_2026-09-29.md`](RESUME_2026-09-29.md) (SUPERSEDED), and the dated session resume points
  `HANDOFF_*.md`, `RESUME_*.md` and `handoff/` (each is the state of its own day; the newest resume
  point is `STATUS_AND_HANDOFF.md`).
- [`whitepaper/`](whitepaper/) and [`pdf/`](pdf/): build outputs; the whitepaper README says which
  revision each one is pinned to.
