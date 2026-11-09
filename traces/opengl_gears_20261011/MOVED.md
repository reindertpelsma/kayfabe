# opengl_gears_20261011: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 1 files (0.2 MB: 1 .png) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `opengl_gears_20261011.tar.zst`, 204445 bytes, sha256 `92e25a6ed0fcf9ead509b2cc2d7221b7e5ac6ac7fe6d9948f390a79005a13232`
- Public URL: `https://logs.kayfabe.dev/evidence/opengl_gears_20261011.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py opengl_gears_20261011`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/opengl_gears_20261011.tar.zst && echo "92e25a6ed0fcf9ead509b2cc2d7221b7e5ac6ac7fe6d9948f390a79005a13232  opengl_gears_20261011.tar.zst" | sha256sum -c`, then `zstd -dc opengl_gears_20261011.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/opengl_gears_20261011/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
