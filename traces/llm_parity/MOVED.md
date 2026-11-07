# llm_parity: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 10 files (0.3 MB: 10 .zst) of this directory are in an archive instead of the tree.
Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `llm_parity.tar.zst`, 262929 bytes, sha256 `d8418e92a5302fc1b512872cf1e9d3decc738ab6e6a76bcb021e1aabc696725b`
- Public URL: `https://logs.kayfabe.dev/evidence/llm_parity.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py llm_parity`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/llm_parity.tar.zst && echo "d8418e92a5302fc1b512872cf1e9d3decc738ab6e6a76bcb021e1aabc696725b  llm_parity.tar.zst" | sha256sum -c`, then `zstd -dc llm_parity.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/llm_parity/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
