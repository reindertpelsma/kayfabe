# turing_boot_20261103: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 6 files (1.1 MB: 6 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `turing_boot_20261103.tar.zst`, 975996 bytes, sha256 `d82209b3a8bb52deca681ff8c129536b3824deb57f515b733eada3601b34765b`
- Public URL: `https://logs.kayfabe.dev/evidence/turing_boot_20261103.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py turing_boot_20261103`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/turing_boot_20261103.tar.zst && echo "d82209b3a8bb52deca681ff8c129536b3824deb57f515b733eada3601b34765b  turing_boot_20261103.tar.zst" | sha256sum -c`, then `zstd -dc turing_boot_20261103.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/turing_boot_20261103/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
