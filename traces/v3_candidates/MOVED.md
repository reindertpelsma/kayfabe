# v3_candidates: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 377 files (8.9 MB: 306 .gz, 59 .png, 11 .zst, 1 .tgz) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `v3_candidates.tar.zst`, 6172147 bytes, sha256 `21ada8fdf479dffba40cf2527a3044b45ae94e260c9b4d0e95432cef66b9dda0`
- Public URL: `https://logs.kayfabe.dev/evidence/v3_candidates.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py v3_candidates`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/v3_candidates.tar.zst && echo "21ada8fdf479dffba40cf2527a3044b45ae94e260c9b4d0e95432cef66b9dda0  v3_candidates.tar.zst" | sha256sum -c`, then `zstd -dc v3_candidates.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/v3_candidates/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
