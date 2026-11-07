# ampere_hw_20261103: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 4 files (0.7 MB: 4 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `ampere_hw_20261103.tar.zst`, 647035 bytes, sha256 `b6e9adfb5b579110a219e1192dfa474b7c5684308a5f9186c3ee0d1a41f0e787`
- Public URL: `https://logs.kayfabe.dev/evidence/ampere_hw_20261103.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py ampere_hw_20261103`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/ampere_hw_20261103.tar.zst && echo "b6e9adfb5b579110a219e1192dfa474b7c5684308a5f9186c3ee0d1a41f0e787  ampere_hw_20261103.tar.zst" | sha256sum -c`, then `zstd -dc ampere_hw_20261103.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/ampere_hw_20261103/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
