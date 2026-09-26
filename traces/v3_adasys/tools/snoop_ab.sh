#!/usr/bin/env bash
# A/B on ONE binary: NVOS46_FLAGS_CACHE_SNOOP_ENABLE (bit 4) on every kf-host DMA map, toggled by env.
set -uo pipefail
cd /root/kayfabe && . $HOME/.cargo/env
git checkout -q crates/kf-host/src/lib.rs
python3 - <<'PY'
p='crates/kf-host/src/lib.rs'
s=open(p).read()
old='            flags: extra\n                | page_size\n'
new='            flags: extra\n                | if std::env::var_os("KF_NO_SNOOP").is_some() { 0 } else { 1 << 4 }\n                | page_size\n'
assert s.count(old)==1
open(p,'w').write(s.replace(old,new))
PY
git diff --stat
cargo build -q --release -p kf-harness --bin kf-gate3 --bin kf-gate4 2>&1 | grep -E '^error' -A5 | head
echo "AB_REV $(git rev-parse --short=8 HEAD) + snoop toggle; GPU=$(nvidia-smi --query-gpu=pci.bus_id --format=csv,noheader) KF_GATE_GPU=${KF_GATE_GPU:-0}"
for arm in snoop nosnoop; do
  for i in 1 2 3 4 5; do
    for g in 3 4; do
      if [ $arm = nosnoop ]; then out=$(KF_NO_SNOOP=1 timeout 120 ./target/release/kf-gate$g 2>&1); else out=$(timeout 120 ./target/release/kf-gate$g 2>&1); fi
      v=$(echo "$out" | grep -E '^GATE[0-9]+_VERDICT=' | cut -d= -f2)
      f=$(echo "$out" | grep -c ' FAIL ')
      fl=$(echo "$out" | grep ' FAIL ' | awk '{print $2}' | sort | uniq -c | tr '\n' ' ')
      echo "AB arm=$arm run=$i gate=$g verdict=${v:-NONE} fails=$f $fl"
    done
  done
done
git checkout -q crates/kf-host/src/lib.rs
echo AB_DONE
