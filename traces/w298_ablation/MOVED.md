# w298_ablation: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 5 files (0.4 MB: 5 .gz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w298_ablation.tar.zst`, 356028 bytes, sha256 `9423dc3ce69495f9207676747cbe1387cabf42e6ea304fb69f0bbf4dfed39c88`
- Public URL: `https://logs.kayfabe.dev/evidence/w298_ablation.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w298_ablation`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w298_ablation.tar.zst && echo "9423dc3ce69495f9207676747cbe1387cabf42e6ea304fb69f0bbf4dfed39c88  w298_ablation.tar.zst" | sha256sum -c`, then `zstd -dc w298_ablation.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w298_ablation/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
