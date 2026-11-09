# kf3_read_trace_20261009: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 1 files (0.1 MB: 1 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `kf3_read_trace_20261009.tar.zst`, 74808 bytes, sha256 `960927e834e746f5637a8c43c77a4af8d88517356b8c0ed91fa28d2bca78cde3`
- Public URL: `https://logs.kayfabe.dev/evidence/kf3_read_trace_20261009.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py kf3_read_trace_20261009`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/kf3_read_trace_20261009.tar.zst && echo "960927e834e746f5637a8c43c77a4af8d88517356b8c0ed91fa28d2bca78cde3  kf3_read_trace_20261009.tar.zst" | sha256sum -c`, then `zstd -dc kf3_read_trace_20261009.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/kf3_read_trace_20261009/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
