# v3_ioeventfd: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 5 files (0.4 MB: 5 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `v3_ioeventfd.tar.zst`, 369265 bytes, sha256 `feedf8384f65c2b73c4156ba85cd2d4b4f6df9ecc990053854307e42a7d723c6`
- Public URL: `https://logs.kayfabe.dev/evidence/v3_ioeventfd.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py v3_ioeventfd`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/v3_ioeventfd.tar.zst && echo "feedf8384f65c2b73c4156ba85cd2d4b4f6df9ecc990053854307e42a7d723c6  v3_ioeventfd.tar.zst" | sha256sum -c`, then `zstd -dc v3_ioeventfd.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/v3_ioeventfd/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
