# sweep_integrate_20261018: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 28 files (11.9 MB: 12 .png, 12 .txt, 3 .tsv, 1 .gz) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `sweep_integrate_20261018.tar.zst`, 1843532 bytes, sha256 `e1bd5a737a6dc2da3fde202ee485d84a935e12f87c3f930e99ddbf47e1ab8929`
- Public URL: `https://logs.kayfabe.dev/evidence/sweep_integrate_20261018.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py sweep_integrate_20261018`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/sweep_integrate_20261018.tar.zst && echo "e1bd5a737a6dc2da3fde202ee485d84a935e12f87c3f930e99ddbf47e1ab8929  sweep_integrate_20261018.tar.zst" | sha256sum -c`, then `zstd -dc sweep_integrate_20261018.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/sweep_integrate_20261018/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
