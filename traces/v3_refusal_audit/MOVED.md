# v3_refusal_audit: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 77 files (1.2 MB: 77 .zst) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `v3_refusal_audit.tar.zst`, 1138745 bytes, sha256 `aaeb1c49374e14ada0902b11c5d908b365790c40d9549c23877884809226ed86`
- Public URL: `https://logs.kayfabe.dev/evidence/v3_refusal_audit.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py v3_refusal_audit`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/v3_refusal_audit.tar.zst && echo "aaeb1c49374e14ada0902b11c5d908b365790c40d9549c23877884809226ed86  v3_refusal_audit.tar.zst" | sha256sum -c`, then `zstd -dc v3_refusal_audit.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/v3_refusal_audit/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
