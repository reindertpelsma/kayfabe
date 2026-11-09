# w308_cup8: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 2 files (0.1 MB: 2 .zst) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w308_cup8.tar.zst`, 78206 bytes, sha256 `141f4298bee1edd9cd91683a6afcb9c78419bf0d964ffaecea6ecf2e3f23e08b`
- Public URL: `https://logs.kayfabe.dev/evidence/w308_cup8.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w308_cup8`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w308_cup8.tar.zst && echo "141f4298bee1edd9cd91683a6afcb9c78419bf0d964ffaecea6ecf2e3f23e08b  w308_cup8.tar.zst" | sha256sum -c`, then `zstd -dc w308_cup8.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w308_cup8/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
