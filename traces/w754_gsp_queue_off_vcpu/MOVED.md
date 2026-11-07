# w754_gsp_queue_off_vcpu: bulk files moved out of the git tree

**STATUS: LIVE, 2026-11-07.** 2 files (1.5 MB: 2 .tgz) of this directory are in an archive instead of the tree.
README.md kept in place. Everything else here (small logs, JSON, TSV, scripts, notes) is unchanged. The list of moved files is `MANIFEST.tsv`
(path, bytes, sha256); a citation of a moved file resolves there. Index of all relocated directories: `traces/EVIDENCE_ARCHIVE.md`.

- Archive: `w754_gsp_queue_off_vcpu.tar.zst`, 1340946 bytes, sha256 `8f077aec07e2d7d816fceabf17f6a10819034aced3ec0ca1ef7c7d73d8e7798e`
- Public URL: `https://logs.kayfabe.dev/evidence/w754_gsp_queue_off_vcpu.tar.zst`
- Restore and verify (puts the files back at their cited paths):
  `python3 scripts/evidence/fetch_evidence_archive.py w754_gsp_queue_off_vcpu`
- By hand: `curl -O https://logs.kayfabe.dev/evidence/w754_gsp_queue_off_vcpu.tar.zst && echo "8f077aec07e2d7d816fceabf17f6a10819034aced3ec0ca1ef7c7d73d8e7798e  w754_gsp_queue_off_vcpu.tar.zst" | sha256sum -c`, then `zstd -dc w754_gsp_queue_off_vcpu.tar.zst | tar -x` at the repository root.
- Locate one path: `python3 scripts/evidence/evidence_archive.py where traces/w754_gsp_queue_off_vcpu/<path>`
- The files are also in git history: present in the tree of `next` at `a9dee7b2c` (the base of the relocation change) and every earlier commit.
