#!/bin/bash
# provision_full.sh — take a freshly rented vast KVM-template box to a READY kayfabe v3 bench.
# Run ON THE BOX (copy provision_box.sh to /root first; see scripts/bench/box/README.md).
#   KAYFABE_BRANCH=<branch> bash provision_full.sh [then-run.sh]
# Writes /root/prov/prov.log with one RC line per step and a READY/NOT_READY verdict line, then
# (optionally) runs the script named in $1 if READY. ⊘ READY requires the kf3 binary built from THIS
# checkout (KF3_RC=0), not merely "a binary exists" — a stale binary once passed that check.
mkdir -p /root/prov; exec > /root/prov/prov.log 2>&1
echo "START $(date -Is)"; rm -f /root/prov/READY
KAYFABE_BRANCH=${KAYFABE_BRANCH:-master} bash /root/provision_box.sh > /root/prov/box.log 2>&1; echo "BOX_RC=$?"
# ⚠ [2026-09-27] a kernel newer than the running one is installed: the driver built next will
# not survive a reboot (provision_box.sh header). Said here, in the log people read.
grep -a 'REBOOT_NEEDED' /root/prov/box.log
export PATH=$HOME/.cargo/bin:$PATH
cd /root/kayfabe || { echo NO_REPO; echo "EXIT $(date -Is)"; exit 1; }
git log --oneline -1
bash scripts/bench/provision_host_driver.sh > /root/prov/driver.log 2>&1; echo "DRIVER_RC=$?"
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader
df -h / | tail -1
bash scripts/bench/provision_bench_tree.sh > /root/prov/tree.log 2>&1; TREE_RC=$?; echo "TREE_RC=$TREE_RC"; tail -2 /root/prov/tree.log
# KF_LADDER_BUILD_BEGIN
# ⊘ [2026-10-26] `-q` does NOT silence rustc warnings (checked: `cargo build -q` and CARGO_TERM_QUIET both
# print them), and crates/kayfabe-linux-raw's `-W missing-docs` flooded the Vast job log, which can show
# only a bounded part of the box output. Output goes to a file; the RC line stays exact and a FAILED
# build prints its error lines and the tail of the file, so nothing that explains a failure is hidden.
# Flags and lint level are unchanged.
cargo build -q --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder > /root/prov/ladder.log 2>&1; LADDER_RC=$?
echo "LADDER_RC=$LADDER_RC"
[ "$LADDER_RC" -eq 0 ] || { grep -a '^error' /root/prov/ladder.log | head -n 20; tail -n 40 /root/prov/ladder.log; }
# KF_LADDER_BUILD_END
bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 /workspace/bench/fastguest > /root/prov/fg.log 2>&1; FG_RC=$?; echo "FG_RC=$FG_RC"; tail -2 /root/prov/fg.log
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3 > /root/prov/kf3.log 2>&1; KF3_RC=$?; echo "KF3_RC=$KF3_RC"; tail -1 /root/prov/kf3.log
V=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)
if [ "$V" = "${KF_HOST_DRIVER:-580.159.04}" ] && [ "$KF3_RC" = 0 ] && [ "$FG_RC" = 0 ] && [ -f /workspace/bench/fastguest/initrd.cpio.gz ]; then
  touch /root/prov/READY; echo "READY driver=$V"
else echo "NOT_READY driver=$V KF3_RC=$KF3_RC FG_RC=$FG_RC"; fi
echo "EXIT $(date -Is)"
# ⊘ [2026-09-27] The exit status is the verdict: a `vx -b` job's .rc and `vwait` read it. The old
# last line, `[ -f READY ] && [ -n "$1" ] && bash "$1"`, made every READY run WITHOUT a
# then-script exit 1 — indistinguishable from NOT_READY.
[ -f /root/prov/READY ] || exit 1
[ -z "${1:-}" ] || bash "$1"
