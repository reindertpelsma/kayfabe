# tmo_stage_20261006: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 21 files (2.3 MB: 21 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `tmo_stage_20261006.tar.zst`, 1110112 bytes, sha256 `135b06e7edadc94addfbf8d16a0978fa7cc3ca3d0e779195fa9a7e07ba62497b`
- Public URL: `https://logs.kayfabe.dev/evidence/tmo_stage_20261006.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py tmo_stage_20261006`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/tmo_stage_20261006.tar.zst && echo "135b06e7edadc94addfbf8d16a0978fa7cc3ca3d0e779195fa9a7e07ba62497b  tmo_stage_20261006.tar.zst" | sha256sum -c`, then `zstd -dc tmo_stage_20261006.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/tmo_stage_20261006/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
