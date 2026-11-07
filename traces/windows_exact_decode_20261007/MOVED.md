# windows_exact_decode_20261007: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 2 files (0.2 MB: 2 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `windows_exact_decode_20261007.tar.zst`, 130608 bytes, sha256 `8872e5672c7c4fed5a7dab376fb62fe8519dc0dff9d0adee69e564c06fec6930`
- Public URL: `https://logs.kayfabe.dev/evidence/windows_exact_decode_20261007.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py windows_exact_decode_20261007`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/windows_exact_decode_20261007.tar.zst && echo "8872e5672c7c4fed5a7dab376fb62fe8519dc0dff9d0adee69e564c06fec6930  windows_exact_decode_20261007.tar.zst" | sha256sum -c`, then `zstd -dc windows_exact_decode_20261007.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/windows_exact_decode_20261007/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
