# w299_multiproc: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 7 files (2.2 MB: 7 .gz) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w299_multiproc.tar.zst`, 1996911 bytes, sha256 `a781de43f0084fa35e5c6fcf4f6cf4a01a23221ad1d2012bb99ab6b097ae5e2c`
- Public URL: `https://logs.kayfabe.dev/evidence/w299_multiproc.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w299_multiproc`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w299_multiproc.tar.zst && echo "a781de43f0084fa35e5c6fcf4f6cf4a01a23221ad1d2012bb99ab6b097ae5e2c  w299_multiproc.tar.zst" | sha256sum -c`, then `zstd -dc w299_multiproc.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w299_multiproc/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
