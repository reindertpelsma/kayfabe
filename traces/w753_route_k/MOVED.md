# w753_route_k: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 1 files (0.0 MB: 1 .gz) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w753_route_k.tar.zst`, 30519 bytes, sha256 `f53a58c883b385caa8890ffdd21ad6fa5b78828db4b3f07c48f5723d0bbed74e`
- Public URL: `https://logs.kayfabe.dev/evidence/w753_route_k.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w753_route_k`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w753_route_k.tar.zst && echo "f53a58c883b385caa8890ffdd21ad6fa5b78828db4b3f07c48f5723d0bbed74e  w753_route_k.tar.zst" | sha256sum -c`, then `zstd -dc w753_route_k.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w753_route_k/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
