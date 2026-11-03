#!/bin/bash
# collect.sh — pack the evidence of the Ada PC regression into $E (text + gzip), scrubbed of board/VM identity.
set -uo pipefail
D=/data/claude-ada-20261103; E=$D/evidence; rm -rf $E; mkdir -p $E/linux/next-70e3fabd $E/linux/pre-9297ffdd $E/windows
V=$D/validate-70e3fabd
cp $V/summary.log $E/linux/next-70e3fabd/validate_summary.log
grep -E '^test result|FAILED|panicked' $V/unit.log | gzip -9 > $E/linux/next-70e3fabd/unit_results.txt.gz
cp $V/gates.log $E/linux/next-70e3fabd/gates.log
tail -5 $V/build_kf3.log > $E/linux/next-70e3fabd/build_kf3_tail.txt
tail -8 $V/fg-build.log > $E/linux/next-70e3fabd/fg_build_tail.txt
cp $V/bench/ada_suite.out $E/linux/next-70e3fabd/
tar -C $V/bench -czf $E/linux/next-70e3fabd/fast_arm_logs.tgz $(cd $V/bench && ls fast_ada_*)
cp $D/ab-pre/summary.log $E/linux/pre-9297ffdd/ab_summary.log
cp $D/ab-pre/bench/pre_suite.out $E/linux/pre-9297ffdd/
tar -C $D/ab-pre/bench -czf $E/linux/pre-9297ffdd/fast_arm_logs.tgz $(cd $D/ab-pre/bench && ls fast_pre_*)
python3 $D/ab_compare.py $V/bench ada $D/ab-pre/bench pre > $E/linux/ab_walk_compare.txt
dmesg -T | grep 'NVRM: Xid' > $E/host_xid_lines.txt
cp $D/probes.sha256 $E/windows/probes.sha256
for w in "$@"; do
  A=$D/win-$w; O=$E/windows/$w; mkdir -p $O
  cp $A/guest_summary.log $A/command.json $A/stop.json $A/powerdown.json $A/backing-before.json $A/backing-after.json $A/xid-after.json $O/ 2>/dev/null
  cp $A/host_summary.txt $O/ 2>/dev/null
  cp -r $A/steps $O/steps
  gzip -9c $A/qemu.log > $O/qemu.log.gz
  gzip -9c $A/cpu.jsonl > $O/cpu.jsonl.gz
  cp $A/broker.log $O/ 2>/dev/null
  cp $A/*.png $O/ 2>/dev/null
done
# scrub: GPU UUIDs (repo CI guard), the VM identity (-uuid, SMBIOS serial, MAC) of the owner's image
python3 - "$E" <<'PY'
import gzip, re, sys, pathlib
root = pathlib.Path(sys.argv[1])
U = r'[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}'
seen = {}
def fake(m):
    u = m.group(0).lower()
    if u.startswith('fa4efa4e') or u == '00000000-0000-0000-0000-000000000000':
        return m.group(0)
    seen.setdefault(u, len(seen) + 1)
    return 'fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e%04d' % seen[u]
def scrub(t):
    t = re.sub(r'(?<=GPU-)' + U, fake, t)
    t = re.sub(r'(?<=gpu-uuid=)' + U, fake, t)
    t = re.sub(r'(?<=-uuid", ")' + U, fake, t)
    t = re.sub(r'(?<=-uuid )' + U, fake, t)
    t = re.sub(r'serial=KF[0-9A-F]+', 'serial=KFSCRUBBED', t)
    t = re.sub(r'mac=52:54:00:[0-9a-f:]{8}', 'mac=52:54:00:00:00:01', t)
    t = re.sub(r'(?i)(GPU UUID\s*:\s*)(GPU-)?' + U, lambda m: m.group(1) + 'GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e9999', t)
    t = re.sub(r'(?i)(Serial Number\s*:\s*)\S+', r'\1SCRUBBED', t)
    t = re.sub(r'172\.22\.[0-9.]+', 'TRUSTED-HOST', t)
    return t
for p in root.rglob('*'):
    if p.is_dir() or p.suffix in ('.png', '.tgz'):
        continue
    if p.suffix == '.gz':
        t = gzip.decompress(p.read_bytes()).decode('utf-8', 'replace')
        p.write_bytes(gzip.compress(scrub(t).encode(), 9))
    else:
        p.write_text(scrub(p.read_text(errors='replace')))
print('SCRUB uuids_replaced', len(seen))
PY
# the arm tarballs: scrub by repacking
for t in $(find $E -name '*.tgz'); do
  x=$(mktemp -d); tar -C $x -xzf $t
  python3 - "$x" <<'PY'
import re, sys, pathlib
U = r'[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}'
for p in pathlib.Path(sys.argv[1]).rglob('*'):
    if p.is_file():
        t = p.read_text(errors='replace')
        t = re.sub(r'(?<=GPU-)' + U, 'fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0100', t)
        t = re.sub(r'(?<=gpu-uuid=)' + U, 'fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0100', t)
        p.write_text(t)
PY
  tar -C $x -czf $t . ; rm -rf $x
done
du -sh $E; find $E -type f | wc -l
