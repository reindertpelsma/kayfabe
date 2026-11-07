# boots: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 60 files (3.2 MB: 53 .gz, 5 .log, 2 .tgz) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `boots.tar.zst`, 1240499 bytes, sha256 `aaad24c4d306bab00017dd28be52c0f47fc709139caa7e8ed57b3f762f86ca51`
- Public URL: `https://logs.kayfabe.dev/evidence/boots.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py boots`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/boots.tar.zst && echo "aaad24c4d306bab00017dd28be52c0f47fc709139caa7e8ed57b3f762f86ca51  boots.tar.zst" | sha256sum -c`, then `zstd -dc boots.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/boots/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
