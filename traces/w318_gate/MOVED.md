# w318_gate: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 10 files (3.1 MB: 10 .zst) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w318_gate.tar.zst`, 3074189 bytes, sha256 `52337d12bac686968823c11ff879f70bb7caa45915c09d70c1077aca83816650`
- Public URL: `https://logs.kayfabe.dev/evidence/w318_gate.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w318_gate`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w318_gate.tar.zst && echo "52337d12bac686968823c11ff879f70bb7caa45915c09d70c1077aca83816650  w318_gate.tar.zst" | sha256sum -c`, then `zstd -dc w318_gate.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w318_gate/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
