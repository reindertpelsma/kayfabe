# windows_matrix_20261011: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 173 files (167.6 MB: 138 .png, 34 .gz, 1 .txt) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `windows_matrix_20261011.tar.zst`, 79006061 bytes, sha256 `7edafc0ff776fe450f210ff9e88ea64b640d8d73afd3fc13082ba7195c58ffeb`
- Public URL: `https://logs.kayfabe.dev/evidence/windows_matrix_20261011.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py windows_matrix_20261011`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/windows_matrix_20261011.tar.zst && echo "7edafc0ff776fe450f210ff9e88ea64b640d8d73afd3fc13082ba7195c58ffeb  windows_matrix_20261011.tar.zst" | sha256sum -c`, then `zstd -dc windows_matrix_20261011.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/windows_matrix_20261011/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
