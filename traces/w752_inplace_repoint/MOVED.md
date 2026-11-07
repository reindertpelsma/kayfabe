# w752_inplace_repoint: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 1 files (0.7 MB: 1 .tgz) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w752_inplace_repoint.tar.zst`, 661002 bytes, sha256 `2e9fe64acb5103e47cc40ef8dfbe37abf3b1cad589a496baf638f13473bfad2d`
- Public URL: `https://logs.kayfabe.dev/evidence/w752_inplace_repoint.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w752_inplace_repoint`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w752_inplace_repoint.tar.zst && echo "2e9fe64acb5103e47cc40ef8dfbe37abf3b1cad589a496baf638f13473bfad2d  w752_inplace_repoint.tar.zst" | sha256sum -c`, then `zstd -dc w752_inplace_repoint.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w752_inplace_repoint/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
