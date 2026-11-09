# w317_drain: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 15 files (1.3 MB: 15 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w317_drain.tar.zst`, 890158 bytes, sha256 `f782d97d2e3489e02fb695cc1c29b4e77ed98a66b7ba69614368007c1726dbc8`
- Public URL: `https://logs.kayfabe.dev/evidence/w317_drain.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w317_drain`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w317_drain.tar.zst && echo "f782d97d2e3489e02fb695cc1c29b4e77ed98a66b7ba69614368007c1726dbc8  w317_drain.tar.zst" | sha256sum -c`, then `zstd -dc w317_drain.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w317_drain/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
