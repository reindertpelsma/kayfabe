# vh_archive: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 2 files (0.2 MB: 2 .tgz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `vh_archive.tar.zst`, 213894 bytes, sha256 `cc904379779121d7f53d122597988b426e61e760bf922ff04b6bd487d34e14bd`
- Public URL: `https://logs.kayfabe.dev/evidence/vh_archive.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py vh_archive`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/vh_archive.tar.zst && echo "cc904379779121d7f53d122597988b426e61e760bf922ff04b6bd487d34e14bd  vh_archive.tar.zst" | sha256sum -c`, then `zstd -dc vh_archive.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/vh_archive/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
