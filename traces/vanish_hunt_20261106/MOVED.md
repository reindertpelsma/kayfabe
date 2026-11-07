# vanish_hunt_20261106: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 9 files (0.5 MB: 9 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `vanish_hunt_20261106.tar.zst`, 483359 bytes, sha256 `6a8d0a817e99313079aad6ad58e07a2d2c6edba7cf4970bde4830841b569a6a0`
- Public URL: `https://logs.kayfabe.dev/evidence/vanish_hunt_20261106.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py vanish_hunt_20261106`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/vanish_hunt_20261106.tar.zst && echo "6a8d0a817e99313079aad6ad58e07a2d2c6edba7cf4970bde4830841b569a6a0  vanish_hunt_20261106.tar.zst" | sha256sum -c`, then `zstd -dc vanish_hunt_20261106.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/vanish_hunt_20261106/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
