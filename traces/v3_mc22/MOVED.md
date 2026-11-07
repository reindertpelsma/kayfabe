# v3_mc22: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 9 files (0.2 MB: 8 .xz, 1 .png) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `v3_mc22.tar.zst`, 168434 bytes, sha256 `167f8c8f884a2bdad79c7924bdce3051af555c00a40185ec2211d13d1f25e211`
- Public URL: `https://logs.kayfabe.dev/evidence/v3_mc22.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py v3_mc22`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/v3_mc22.tar.zst && echo "167f8c8f884a2bdad79c7924bdce3051af555c00a40185ec2211d13d1f25e211  v3_mc22.tar.zst" | sha256sum -c`, then `zstd -dc v3_mc22.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/v3_mc22/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
