# w322_operands: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 8 files (0.0 MB: 8 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w322_operands.tar.zst`, 36263 bytes, sha256 `264383d74192df9adee6fc6fb1bd5459be591b829bd605eb622ff0913807e4be`
- Public URL: `https://logs.kayfabe.dev/evidence/w322_operands.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w322_operands`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w322_operands.tar.zst && echo "264383d74192df9adee6fc6fb1bd5459be591b829bd605eb622ff0913807e4be  w322_operands.tar.zst" | sha256sum -c`, then `zstd -dc w322_operands.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w322_operands/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
