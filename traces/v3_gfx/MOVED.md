# v3_gfx: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 2 files (0.0 MB: 2 .png) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `v3_gfx.tar.zst`, 14320 bytes, sha256 `512011dcb39e9d4ae13575b80653994556297a3d2e35451f417e00620cc1186b`
- Public URL: `https://logs.kayfabe.dev/evidence/v3_gfx.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py v3_gfx`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/v3_gfx.tar.zst && echo "512011dcb39e9d4ae13575b80653994556297a3d2e35451f417e00620cc1186b  v3_gfx.tar.zst" | sha256sum -c`, then `zstd -dc v3_gfx.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/v3_gfx/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
