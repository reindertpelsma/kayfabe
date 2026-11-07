# v3_cdp: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 40 files (0.6 MB: 36 .gz, 4 .zst) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `v3_cdp.tar.zst`, 505466 bytes, sha256 `9c05f44e28fe0675ef77e22131cb03d02b2c8b6788377ec0fee7546befe6ddac`
- Public URL: `https://logs.kayfabe.dev/evidence/v3_cdp.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py v3_cdp`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/v3_cdp.tar.zst && echo "9c05f44e28fe0675ef77e22131cb03d02b2c8b6788377ec0fee7546befe6ddac  v3_cdp.tar.zst" | sha256sum -c`, then `zstd -dc v3_cdp.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/v3_cdp/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
