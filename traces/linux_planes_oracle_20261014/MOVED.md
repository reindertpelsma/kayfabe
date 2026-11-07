# linux_planes_oracle_20261014: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 4 files (0.0 MB: 4 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `linux_planes_oracle_20261014.tar.zst`, 21564 bytes, sha256 `00c8ae4750181b2e08d8a416babeeb744ef83ebe30ed858e450606577c233f4e`
- Public URL: `https://logs.kayfabe.dev/evidence/linux_planes_oracle_20261014.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py linux_planes_oracle_20261014`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/linux_planes_oracle_20261014.tar.zst && echo "00c8ae4750181b2e08d8a416babeeb744ef83ebe30ed858e450606577c233f4e  linux_planes_oracle_20261014.tar.zst" | sha256sum -c`, then `zstd -dc linux_planes_oracle_20261014.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/linux_planes_oracle_20261014/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
