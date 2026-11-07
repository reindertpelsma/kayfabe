# w294_cudalimit: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 1 files (0.0 MB: 1 .zst) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w294_cudalimit.tar.zst`, 9602 bytes, sha256 `330456b54d970eb29fed915e33c4923730f0e24bd98efbdd8c158a420fca4fd0`
- Public URL: `https://logs.kayfabe.dev/evidence/w294_cudalimit.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w294_cudalimit`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w294_cudalimit.tar.zst && echo "330456b54d970eb29fed915e33c4923730f0e24bd98efbdd8c158a420fca4fd0  w294_cudalimit.tar.zst" | sha256sum -c`, then `zstd -dc w294_cudalimit.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w294_cudalimit/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
