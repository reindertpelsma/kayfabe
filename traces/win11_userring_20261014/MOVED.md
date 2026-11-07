# win11_userring_20261014: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 6 files (0.4 MB: 6 .jpg) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `win11_userring_20261014.tar.zst`, 422784 bytes, sha256 `b97906ca9a6584dc98adae7c7913a5f8ed6721e6fbf5ce5d1a3421c19f7e37b8`
- Public URL: `https://logs.kayfabe.dev/evidence/win11_userring_20261014.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py win11_userring_20261014`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/win11_userring_20261014.tar.zst && echo "b97906ca9a6584dc98adae7c7913a5f8ed6721e6fbf5ce5d1a3421c19f7e37b8  win11_userring_20261014.tar.zst" | sha256sum -c`, then `zstd -dc win11_userring_20261014.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/win11_userring_20261014/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
