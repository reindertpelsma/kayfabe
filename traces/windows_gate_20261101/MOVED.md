# windows_gate_20261101: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 6 files (3.5 MB: 6 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `windows_gate_20261101.tar.zst`, 3373166 bytes, sha256 `819f501b2269621f7e46d5c25a2ab6bb939b2ace02af62419f40ce188c811d6a`
- Public URL: `https://logs.kayfabe.dev/evidence/windows_gate_20261101.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py windows_gate_20261101`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/windows_gate_20261101.tar.zst && echo "819f501b2269621f7e46d5c25a2ab6bb939b2ace02af62419f40ce188c811d6a  windows_gate_20261101.tar.zst" | sha256sum -c`, then `zstd -dc windows_gate_20261101.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/windows_gate_20261101/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
