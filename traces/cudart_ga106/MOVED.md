# cudart_ga106: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 2 files (0.0 MB: 2 .zst) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `cudart_ga106.tar.zst`, 8013 bytes, sha256 `f33ac67211ccfb7901144faedd8ead1989a2fc831ffb9500fa767cee91850eaa`
- Public URL: `https://logs.kayfabe.dev/evidence/cudart_ga106.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py cudart_ga106`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/cudart_ga106.tar.zst && echo "f33ac67211ccfb7901144faedd8ead1989a2fc831ffb9500fa767cee91850eaa  cudart_ga106.tar.zst" | sha256sum -c`, then `zstd -dc cudart_ga106.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/cudart_ga106/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
