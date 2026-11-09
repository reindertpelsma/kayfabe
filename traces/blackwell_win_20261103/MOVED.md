# blackwell_win_20261103: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 23 files (5.5 MB: 20 .gz, 3 .png) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `blackwell_win_20261103.tar.zst`, 3006975 bytes, sha256 `90055ed290dc74a7ab278bb6c88e93232d0b102cc05357c7f03319bc5962c447`
- Public URL: `https://logs.kayfabe.dev/evidence/blackwell_win_20261103.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py blackwell_win_20261103`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/blackwell_win_20261103.tar.zst && echo "90055ed290dc74a7ab278bb6c88e93232d0b102cc05357c7f03319bc5962c447  blackwell_win_20261103.tar.zst" | sha256sum -c`, then `zstd -dc blackwell_win_20261103.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/blackwell_win_20261103/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
