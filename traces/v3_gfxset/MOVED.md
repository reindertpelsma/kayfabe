# v3_gfxset: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 246 files (14.0 MB: 164 .zst, 82 .png) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `v3_gfxset.tar.zst`, 6097124 bytes, sha256 `e80469cae7a07d8b9358d84d43cc90c8974cedc5b85fa9fb9d5ed56d9bcb4eda`
- Public URL: `https://logs.kayfabe.dev/evidence/v3_gfxset.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py v3_gfxset`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/v3_gfxset.tar.zst && echo "e80469cae7a07d8b9358d84d43cc90c8974cedc5b85fa9fb9d5ed56d9bcb4eda  v3_gfxset.tar.zst" | sha256sum -c`, then `zstd -dc v3_gfxset.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/v3_gfxset/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
