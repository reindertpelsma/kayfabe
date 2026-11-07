# blackwell_resume_20261021: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 2 files (2.0 MB: 2 .jsonl) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `blackwell_resume_20261021.tar.zst`, 100140 bytes, sha256 `e84879a235406626e337842d549328d4de77aec7f7d1852f8917f3a3e8ebd35c`
- Public URL: `https://logs.kayfabe.dev/evidence/blackwell_resume_20261021.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py blackwell_resume_20261021`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/blackwell_resume_20261021.tar.zst && echo "e84879a235406626e337842d549328d4de77aec7f7d1852f8917f3a3e8ebd35c  blackwell_resume_20261021.tar.zst" | sha256sum -c`, then `zstd -dc blackwell_resume_20261021.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/blackwell_resume_20261021/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
