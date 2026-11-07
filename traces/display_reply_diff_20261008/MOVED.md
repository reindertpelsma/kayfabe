# display_reply_diff_20261008: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 9 files (0.2 MB: 8 .gz, 1 .png) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `display_reply_diff_20261008.tar.zst`, 113733 bytes, sha256 `5f3247577b180053bdd8030bfc2760012e0a894da49973b63b5820e1a0a23669`
- Public URL: `https://logs.kayfabe.dev/evidence/display_reply_diff_20261008.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py display_reply_diff_20261008`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/display_reply_diff_20261008.tar.zst && echo "5f3247577b180053bdd8030bfc2760012e0a894da49973b63b5820e1a0a23669  display_reply_diff_20261008.tar.zst" | sha256sum -c`, then `zstd -dc display_reply_diff_20261008.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/display_reply_diff_20261008/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
