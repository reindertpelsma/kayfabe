# w327_cliff: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 74 files (0.1 MB: 74 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w327_cliff.tar.zst`, 134464 bytes, sha256 `49134d6bb1bf83b0b70d6e79988e1bafe7f96af52d2b0b712c37f6fc70f944a5`
- Public URL: `https://logs.kayfabe.dev/evidence/w327_cliff.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w327_cliff`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w327_cliff.tar.zst && echo "49134d6bb1bf83b0b70d6e79988e1bafe7f96af52d2b0b712c37f6fc70f944a5  w327_cliff.tar.zst" | sha256sum -c`, then `zstd -dc w327_cliff.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w327_cliff/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
