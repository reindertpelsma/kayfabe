# Read a reused GPU gate

STATUS: LIVE IN PROPOSED WORKFLOW, 2026-10-27. CODE READ: `ci/vast-broker/gate_reuse.py` and `.github/workflows/gpu-vast.yml`.

A master push can reuse an original completed GPU PASS when its tree matches a parent or associated PR head, the original `gpu-vast` status is successful and belongs to GitHub Actions, and the same SHA has a successful GitHub Actions `CI` check. Tags and PR events take the existing gate path. Missing, ambiguous, truncated or unavailable metadata and logs take the existing path too.

The existing named GPU job still runs. Its reuse step prints the parent and tree SHAs, original run URL and status description, checked provenance, rental identifiers, die selection, tested revision and final verdict/stage lines. The same text appears in its job summary. The Worker step is skipped on this path; the GPU job itself completes successfully. The original provider lines remain untrusted evidence, as they were in the original run.

CODE READ 2026-10-27 (`gate_reuse.py`, `gpu-vast.yml`): follow the original run link to inspect the full evidence. The master status says `reused: same tree verified on <sha7>` and links to that run. A separate `gpu-vast-reused-by/<newsha7>` status on the source commit links forward without replacing its original GPU status. Expired or incomplete original logs disable reuse; the fallback is the existing gate, not an empty successful run.

Fast-forwards probe before posting pending, so the original same-SHA status is available. Release verification rechecks the full tree/status/CI/workflow-run evidence; it does not trust the copied description. Direct release validation is unchanged. Copied same-SHA statuses retain their immediately preceding original evidence, bounded to eight copies; cross-SHA chains of reused statuses fall back to a fresh gate. `NOT_NEEDED` is not original hardware PASS evidence and cannot become release provenance.

This transfers GitHub metadata for identical trees. It does not make the GPU provider or its verdict attested, and it does not accept build artifacts from the rented machine. No Worker configuration or deployment changes are needed.

CODE READ 2026-10-28 (`gpu-vast.yml`): Site-only paths under root `site/**` are GPU exclusions, alongside documentation. Every file must match the configured skip list; adding a crate or other non-excluded file still runs the GPU gate. Names such as `scripts/sitemap.py` and nested `crates/site/` remain relevant. Root README.md matches `**/*.md`. The Site workflow supplies its own build/link checks.

CODE READ 2026-10-28 (`gate_reuse.py`): complete verdict logs must name provision, cuda_host, merge_bar and cuda_guest exactly once each. Four distinct lines for the same stage are insufficient; incomplete identities take the existing gate path.
