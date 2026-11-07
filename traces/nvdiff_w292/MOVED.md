# nvdiff_w292: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 2 files (0.0 MB: 2 .zst) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `nvdiff_w292.tar.zst`, 18207 bytes, sha256 `8195e495dd9c0e30500853590a36c6e5ea03b355f7882a6e2a416d1ebc92c5a0`
- Public URL: `https://logs.kayfabe.dev/evidence/nvdiff_w292.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py nvdiff_w292`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/nvdiff_w292.tar.zst && echo "8195e495dd9c0e30500853590a36c6e5ea03b355f7882a6e2a416d1ebc92c5a0  nvdiff_w292.tar.zst" | sha256sum -c`, then `zstd -dc nvdiff_w292.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/nvdiff_w292/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
