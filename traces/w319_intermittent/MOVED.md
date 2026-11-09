# w319_intermittent: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 13 files (0.8 MB: 13 .gz) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w319_intermittent.tar.zst`, 638181 bytes, sha256 `620e7bb5020aecbb21fe06ca1880d038603d218d1cc301f2d0d2542cafb67068`
- Public URL: `https://logs.kayfabe.dev/evidence/w319_intermittent.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w319_intermittent`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w319_intermittent.tar.zst && echo "620e7bb5020aecbb21fe06ca1880d038603d218d1cc301f2d0d2542cafb67068  w319_intermittent.tar.zst" | sha256sum -c`, then `zstd -dc w319_intermittent.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w319_intermittent/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
