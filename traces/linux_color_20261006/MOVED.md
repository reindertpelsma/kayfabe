# linux_color_20261006: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 1 files (0.0 MB: 1 .png) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `linux_color_20261006.tar.zst`, 7992 bytes, sha256 `71de66df384026420c5659b8f69347bedac443599c792719a31066d6b8bfa778`
- Public URL: `https://logs.kayfabe.dev/evidence/linux_color_20261006.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py linux_color_20261006`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/linux_color_20261006.tar.zst && echo "71de66df384026420c5659b8f69347bedac443599c792719a31066d6b8bfa778  linux_color_20261006.tar.zst" | sha256sum -c`, then `zstd -dc linux_color_20261006.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/linux_color_20261006/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
