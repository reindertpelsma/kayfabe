# w315_floor: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 6 files (3.1 MB: 6 .zst) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w315_floor.tar.zst`, 3068501 bytes, sha256 `5f938135a24a7d45d4455735e8e41a988e519b79150a6a3008be3afb3fa9bd2c`
- Public URL: `https://logs.kayfabe.dev/evidence/w315_floor.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w315_floor`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w315_floor.tar.zst && echo "5f938135a24a7d45d4455735e8e41a988e519b79150a6a3008be3afb3fa9bd2c  w315_floor.tar.zst" | sha256sum -c`, then `zstd -dc w315_floor.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w315_floor/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
