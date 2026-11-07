# v3_design_review_20261003: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 1 files (0.3 MB: 1 .json) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `v3_design_review_20261003.tar.zst`, 95634 bytes, sha256 `48dd46993bf10208753708e886a581283b7e45cfd54731cac44e8359ab50feb1`
- Public URL: `https://logs.kayfabe.dev/evidence/v3_design_review_20261003.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py v3_design_review_20261003`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/v3_design_review_20261003.tar.zst && echo "48dd46993bf10208753708e886a581283b7e45cfd54731cac44e8359ab50feb1  v3_design_review_20261003.tar.zst" | sha256sum -c`, then `zstd -dc v3_design_review_20261003.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/v3_design_review_20261003/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
