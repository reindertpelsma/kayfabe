# w329_release: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 122 files (0.4 MB: 122 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w329_release.tar.zst`, 366024 bytes, sha256 `764030593fd9a179520653c101605b9b9ac84e34dea290615a8304f991390a68`
- Public URL: `https://logs.kayfabe.dev/evidence/w329_release.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w329_release`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w329_release.tar.zst && echo "764030593fd9a179520653c101605b9b9ac84e34dea290615a8304f991390a68  w329_release.tar.zst" | sha256sum -c`, then `zstd -dc w329_release.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w329_release/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
