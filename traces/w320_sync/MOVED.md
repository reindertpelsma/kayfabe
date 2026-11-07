# w320_sync: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 1 files (0.3 MB: 1 .log) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w320_sync.tar.zst`, 62485 bytes, sha256 `6070a89f201656a9a4ee7fb5274673fd0e2ba2c2811247d20f216416ba083e56`
- Public URL: `https://logs.kayfabe.dev/evidence/w320_sync.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w320_sync`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w320_sync.tar.zst && echo "6070a89f201656a9a4ee7fb5274673fd0e2ba2c2811247d20f216416ba083e56  w320_sync.tar.zst" | sha256sum -c`, then `zstd -dc w320_sync.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w320_sync/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
