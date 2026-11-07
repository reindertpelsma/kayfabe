# w321_batch: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 15 files (1.3 MB: 15 .gz) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w321_batch.tar.zst`, 1002887 bytes, sha256 `6c59cb6e09037e431a2e24c59c9ef3bc56f675ade50327c7c2992783abcd638e`
- Public URL: `https://logs.kayfabe.dev/evidence/w321_batch.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w321_batch`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w321_batch.tar.zst && echo "6c59cb6e09037e431a2e24c59c9ef3bc56f675ade50327c7c2992783abcd638e  w321_batch.tar.zst" | sha256sum -c`, then `zstd -dc w321_batch.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w321_batch/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
