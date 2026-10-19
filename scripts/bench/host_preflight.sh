#!/usr/bin/env bash
# ★★★★★ HOST PREFLIGHT — answer "is this box usable?" BEFORE any kayfabe failure is
# attributed to kayfabe.
#
# ⊘ THE PROBLEM THIS EXISTS FOR. vast hosts are community-rented and cheap *because* they
# are unreliable. When a bench run fails, the first question is not "what did we break" but
# "is this host even sound" — and without an answer, hours get spent debugging our code on a
# machine whose GPU, disk or network was broken on arrival. Renting a fresh box costs cents;
# misattributing a host fault costs a night.
#
# ★ Adapted from the discipline nvkvm-pv's `validate.sh` established: a precondition sweep
# that must PASS before anything downstream is believed.
#
# CONTRACT
#   - Every check ASSERTS. A check that cannot run is a FAIL, never a skip.
#   - Prints a terminator line and an explicit exit status, so "the file exists but has no
#     terminator" is detectable — a truncated run must not read as a pass.
#   - Exit 0 = host sound. Exit 2 = HOST FAULT (destroy it, rent another; do not nurse).
# Die support is checked from host-derived MC_GET_ARCH_INFO/classes by the v3 gates,
# not guessed from a marketing name in this build-free health check.
#
# ⚠ Tier 1 only: no repo build required, so it can run on a box seconds after it boots.
# Tier 2 (our raw RM client round-trip, `scripts/bench/` R33) needs a build and is separate.
set -u
FAIL=0
ok(){ printf '  ✔ %s\n' "$*"; }
bad(){ printf '  ✘ HOST FAULT: %s\n' "$*"; FAIL=1; }
# ⊘ A WARNING is neither a fault nor a wrong box: it is a fact the reader needs and the
# verdict does not change. The suite had no such level, so anything worth saying had to be
# promoted to a failure or dropped — and 'dropped' is how a 13 kB/s mirror passed.
warn(){ printf '  ⚠ %s\n' "$*"; }

echo "=== kayfabe host preflight $(date -Is) on $(hostname) ==="

# 1. KVM — the single most common way a 'GPU box' turns out useless. vms_enabled describes
#    the HOST's capability, not the instance you were given: a plain --image yields a Docker
#    container where nvidia-smi is perfect and /dev/kvm does not exist.
[ -c /dev/kvm ] && ok "/dev/kvm present" || bad "/dev/kvm MISSING — this is a container, not a VM instance"
grep -qE 'vmx|svm' /proc/cpuinfo && ok "CPU virt extensions present" || bad "no vmx/svm in /proc/cpuinfo"

# 2. NVIDIA driver responds. All supported families proceed to the host-derived
# chip/class checks in the v3 gates; a GPU name cannot establish that support.
if command -v nvidia-smi >/dev/null 2>&1; then
  # Preserve the query exit status: piping through head hides a failed query,
  # and nvidia-smi can print "No devices were found" on stdout when it fails.
  GPU=$(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null)
  GPU_RC=$?
  DRV=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>/dev/null)
  DRV_RC=$?
  GPU=${GPU%%$'\n'*}
  DRV=${DRV%%$'\n'*}
  if [ "$GPU_RC" -ne 0 ] || [ "$DRV_RC" -ne 0 ]; then
    bad "nvidia-smi identity query failed (name=$GPU_RC driver=$DRV_RC)"
  elif [ -z "$GPU" ] || [ "$GPU" = 'No devices were found' ]; then
    bad "nvidia-smi present but returned NO GPU (driver wedged?)"
  elif [[ ! "$DRV" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]]; then
    bad "nvidia-smi returned no valid numeric driver version"
  else
    ok "GPU: $GPU  driver: $DRV"
  fi
else bad "nvidia-smi ABSENT"; fi

# 3. Driver device nodes. The native workload checks are Tier2: listing a device proves
#    the driver loaded, NOT that a context can be created — the distinction that let a broken
#    cuMemAllocManaged pass 28/28 in a sibling project because the only UVM coverage was
#    checking that /dev/nvidia-uvm existed. A device node existing says nothing.
for n in /dev/nvidiactl /dev/nvidia0; do
  [ -c "$n" ] && ok "$n present" || bad "$n MISSING"
done
if [ -c /dev/nvidia-uvm ]; then ok "/dev/nvidia-uvm present"
else printf '  ⚠ /dev/nvidia-uvm absent (loads on first CUDA use; not fatal here)\n'; fi

# 4. Resources. A box that cannot hold the tree produces link failures that read as
#    regressions: ENOSPC has surfaced as an LLVM linker crash.
AVAIL=$(df -BG --output=avail / 2>/dev/null | tail -1 | tr -dc '0-9')
[ "${AVAIL:-0}" -ge 100 ] && ok "disk free ${AVAIL}G" || bad "only ${AVAIL:-?}G free on / — need >=100G"
RAMG=$(free -g | awk '/^Mem:/{print $2}')
[ "${RAMG:-0}" -ge 16 ] && ok "RAM ${RAMG}G" || bad "only ${RAMG:-?}G RAM"
CORES=$(nproc)
[ "${CORES:-0}" -ge 8 ] && ok "cores ${CORES}" || bad "only ${CORES:-?} cores"

# 5. Network egress. A host that cannot fetch cannot be provisioned, and the failure
#    otherwise appears much later as a mysteriously missing dependency.
# ★★★★★ **MEASURE THE APT MIRROR, NOT JUST REACHABILITY.**
#
# `[measured w439, vast 50585481]` this box passed every check below, and then
# `apt-get install build-essential …` ran **44 minutes and installed nothing**: its
# `archive.ubuntu.com` served **13 344 B/s** while GitHub on the same box served
# **1 434 152 B/s**. Not an egress problem — one slow mirror.
#
# ⊘ A reachability check cannot tell a usable mirror from one that will never finish, and this
# is the preflight whose entire job is *"destroy it rather than provision it"*. Any check that
# passes an unprovisionable box has failed at that job.
#
# ⚠ A WARNING, not a fault: `provision_box.sh` races mirrors and switches, so a slow default is
# survivable. It is reported so a human reading the log knows why provisioning took a while.
APT_SPEED=$(timeout 15 curl -s -o /dev/null -w "%{speed_download}" \
            http://archive.ubuntu.com/ubuntu/dists/jammy/Release 2>/dev/null || echo 0)
APT_SPEED=${APT_SPEED%%.*}
if [ "${APT_SPEED:-0}" -ge 200000 ]; then
  ok "apt mirror ${APT_SPEED} B/s"
else
  warn "apt mirror only ${APT_SPEED:-0} B/s — provision_box.sh will switch mirrors; if it \
cannot find one over 200 kB/s this box is not provisionable"
fi
if timeout 25 curl -fsSL -o /dev/null https://github.com 2>/dev/null; then ok "https egress works"
else bad "cannot reach github over https"; fi

echo "=== PREFLIGHT DONE ==="
if [ "$FAIL" -ne 0 ]; then echo "VERDICT=HOST_FAULT"; echo "EXIT=2"; exit 2; fi
echo "VERDICT=SOUND"; echo "EXIT=0"; exit 0
