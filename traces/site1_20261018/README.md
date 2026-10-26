# SITE1 verification, 2026-10-26

MEASURED (SITE1 local run 2026-10-26): pinned Zola 0.23.6 and mdBook 0.4.52 rendered the master
landing, staging landing, and three seed manual pages (introduction, Windows settings, FAQ).
Release metadata and local tags were empty: root/latest both fall back to master. Broken local
links: **0** page/target pairs. Internal docs are not copied into the book.

MEASURED (SITE1 fixture run 2026-10-26): real pinned binaries rebuilt master, v0.3.0 and v0.2.0
manuals (9 pages total), selected published v0.3.0 for root/latest, and omitted v0.1.0 without a
manual. v0.2.0 had no site. Invalid tag-local Zola/mdBook configs and an `exit 99` tagged build
script were ignored. The old FAQ canonical targeted the real latest tag; broken local links: 0.
Both archives were already cached and were SHA-256 checked again; no tool download was required.

MEASURED (SITE1 local run 2026-10-26): 28 Python version/scope/tool/render tests, 6 built-output
tests, 46 targeted Node workflow/dropdown tests, and the full 2,090-test broker suite passed.
The initial expected-red tests had 9/9 Python errors (missing builder) and 12/13 Node failures
(missing Pages/OIDC guard). Later expected-red commits exposed missing metadata/dropdown support
and two internal-doc-link failures before implementation.

MEASURED (SITE1 mutation campaign 2026-10-26): all 16 mutants were killed by at least two tests;
`mutants.json` names every killing test. The originals were restored and the baseline rerun.
Mutants cover version eligibility, artifact/draft filters, publication ordering, root override,
manual-only copying, internal-link/title checks, rebuilding old versions, archive hashes,
master book config, release canonicals, broken-link reporting, Pages boundary, OIDC and selector.

CODE READ (2026-10-26): unchanged FAQ has no Markdown links into internal docs. Its inline code
references `design/V3_VIOMMU.md`, `design/the_three_channel_kinds.md`, `docs/OWNER_RULINGS.md`,
`docs/STATUS_AND_HANDOFF.md` and trace paths. Those references remain plain code, not site links;
the FAQ source is unchanged. The Windows seed preserves the original measured/inferred labels
and cites its source dates; it adds no install procedure.

CODE READ (2026-10-26): `ci.yml` has no site/docs exclusion. Complete docs-only PR metadata skips
GPU via `docs/**`/`**/*.md`; site code/assets, workflow and broker guard changes request GPU.
Unknown/truncated metadata requests GPU; a rename skips only if both canonical paths are skipped.
No GPU/CI trigger policy was changed. `release-build.yml` only builds/attests, so the new release
trigger handles manual/PAT publication. A future GITHUB_TOKEN publisher needs a final dispatch
step after asset upload. No deployment, Worker settings, secrets, or root README changes were made.
