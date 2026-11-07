# blackwell_validation4_20261022: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 2 files (1.5 MB: 2 .jsonl) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `blackwell_validation4_20261022.tar.zst`, 71261 bytes, sha256 `2d2065be27dfe48bccc83d270ce02007af324af12a51cb11c0354e6eb1239560`
- Public URL: `https://logs.kayfabe.dev/evidence/blackwell_validation4_20261022.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py blackwell_validation4_20261022`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/blackwell_validation4_20261022.tar.zst && echo "2d2065be27dfe48bccc83d270ce02007af324af12a51cb11c0354e6eb1239560  blackwell_validation4_20261022.tar.zst" | sha256sum -c`, then `zstd -dc blackwell_validation4_20261022.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/blackwell_validation4_20261022/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
