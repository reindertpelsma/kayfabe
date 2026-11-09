# w314_confirm: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 6 files (0.4 MB: 6 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w314_confirm.tar.zst`, 348452 bytes, sha256 `43d66c293516eb535578195e9f64c46cf6400978435a9e1a7a1b7dc54d9efd29`
- Public URL: `https://logs.kayfabe.dev/evidence/w314_confirm.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w314_confirm`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w314_confirm.tar.zst && echo "43d66c293516eb535578195e9f64c46cf6400978435a9e1a7a1b7dc54d9efd29  w314_confirm.tar.zst" | sha256sum -c`, then `zstd -dc w314_confirm.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w314_confirm/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
