# recovered_integration_20260929: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 1 files (1.1 MB: 1 .log) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `recovered_integration_20260929.tar.zst`, 38756 bytes, sha256 `623c3083071b888e29afbd70d657cffa1eb4ca850485fe64854e68069ff3dbba`
- Public URL: `https://logs.kayfabe.dev/evidence/recovered_integration_20260929.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py recovered_integration_20260929`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/recovered_integration_20260929.tar.zst && echo "623c3083071b888e29afbd70d657cffa1eb4ca850485fe64854e68069ff3dbba  recovered_integration_20260929.tar.zst" | sha256sum -c`, then `zstd -dc recovered_integration_20260929.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/recovered_integration_20260929/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
