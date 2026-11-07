# v3_initrace: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 133 files (1.5 MB: 133 .xz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `v3_initrace.tar.zst`, 1514259 bytes, sha256 `619cfdd6f372a727ed0a00dda668a17f9af85922148e36c07ed5698f6e029dc0`
- Public URL: `https://logs.kayfabe.dev/evidence/v3_initrace.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py v3_initrace`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/v3_initrace.tar.zst && echo "619cfdd6f372a727ed0a00dda668a17f9af85922148e36c07ed5698f6e029dc0  v3_initrace.tar.zst" | sha256sum -c`, then `zstd -dc v3_initrace.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/v3_initrace/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
