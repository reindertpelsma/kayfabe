#!/usr/bin/env bash
# In the VM: DevCtl Enable-No-Snoop OFF (RM's passthrough default) vs ON (setpci), x snoop bit on/off.
set -uo pipefail
export PATH=$HOME/.cargo/bin:$PATH
BDF=${BDF:-00:07.0}
cd /root/kayfabe && git checkout -q -- . && git fetch -q origin v3-adasys && git checkout -q -B v3-adasys origin/v3-adasys
python3 - <<'PY'
p='crates/kf-host/src/lib.rs'
s=open(p).read()
old='            flags: nvos46_map_flags(extra, page_size, at.is_some()),\n'
new='            flags: nvos46_map_flags(extra, page_size, at.is_some()) & if std::env::var_os("KF_NO_SNOOP").is_some() { !NVOS46_FLAGS_CACHE_SNOOP_ENABLE } else { !0 },\n'
assert s.count(old)==1
open(p,'w').write(s.replace(old,new))
PY
cargo build -q --release -p kf-harness --bin kf-gate3 --bin kf-gate4 2>&1 | grep -E '^error' -A5 | head
echo "NSAB_REV $(git rev-parse --short=8 HEAD) + KF_NO_SNOOP toggle; GPU $(nvidia-smi --query-gpu=name,pci.bus_id --format=csv,noheader)"
orig=$(setpci -s $BDF CAP_EXP+8.w)
echo "DevCtl original=0x$orig ($(lspci -vvv -s $BDF | grep -o 'NoSnoop[+-]' | head -1))"
run() {
  for arm in snoop nosnoop; do
    for i in 1 2 3; do
      for g in 3 4; do
        if [ $arm = nosnoop ]; then out=$(KF_NO_SNOOP=1 timeout 120 ./target/release/kf-gate$g 2>&1); else out=$(timeout 120 ./target/release/kf-gate$g 2>&1); fi
        v=$(echo "$out" | grep -E '^GATE[0-9]+_VERDICT=' | cut -d= -f2)
        fl=$(echo "$out" | grep ' FAIL ' | awk '{print $2}' | sort | uniq -c | tr '\n' ' ')
        echo "NSAB devctl_nosnoop=$1 arm=$arm run=$i gate=$g verdict=${v:-NONE} $fl"
      done
    done
  done
}
n0=$(dmesg | wc -l)
run off
setpci -s $BDF CAP_EXP+8.w=$(printf '%04x' $(( 0x$orig | 0x0800 )))
echo "DevCtl now=0x$(setpci -s $BDF CAP_EXP+8.w) ($(lspci -vvv -s $BDF | grep -o 'NoSnoop[+-]' | head -1))"
run on
setpci -s $BDF CAP_EXP+8.w=$orig
echo "DevCtl restored=0x$(setpci -s $BDF CAP_EXP+8.w) ($(lspci -vvv -s $BDF | grep -o 'NoSnoop[+-]' | head -1))"
dmesg | tail -n +$((n0+1)) > /root/prov/dmesg_nsab.log
echo "dmesg lines during NSAB: $(wc -l < /root/prov/dmesg_nsab.log); xid/fault: $(grep -c -i -E 'xid|fault|iommu|dmar|aer' /root/prov/dmesg_nsab.log)"
git checkout -q crates/kf-host/src/lib.rs
echo NSAB_DONE
