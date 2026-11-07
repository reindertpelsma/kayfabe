# hitch_20261017: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 6 files (1.6 MB: 6 .png) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `hitch_20261017.tar.zst`, 1480073 bytes, sha256 `8cc31f3f6aeb6b539001dcd414f0d1f9b65c8779938795db76a0094e78dda7c6`
- Public URL: `https://logs.kayfabe.dev/evidence/hitch_20261017.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py hitch_20261017`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/hitch_20261017.tar.zst && echo "8cc31f3f6aeb6b539001dcd414f0d1f9b65c8779938795db76a0094e78dda7c6  hitch_20261017.tar.zst" | sha256sum -c`, then `zstd -dc hitch_20261017.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/hitch_20261017/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
