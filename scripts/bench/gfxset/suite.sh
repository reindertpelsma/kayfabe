#!/usr/bin/env bash
# ★ THE HEADLESS-GRAPHICS TEST SET (docs/design/V3_GFX_TESTSET.md) — one command, one table.
#   usage (on the box, in the kayfabe checkout, as root):
#     bash scripts/bench/gfxset/suite.sh <run> [item...]      (default: every item)
#   1. BARE METAL, TWICE, first (host.sh → host.res, host2.res): the same items in the guest image's
#      userspace on this box's GPU. Two runs give the noise floor — a digest that differs between two
#      bare-metal runs is NONDET and is not graded by equality.
#   2. THE FAT GUEST on the kf3 device built from THIS revision (boot_capture.sh, KF_DEVICE=kf3),
#      hook.sh running the items one per ssh call, batched per boot; a boot that dies or wedges is
#      recorded and the rest continue in a fresh boot.
#   3. Every non-PASS item re-run ALONE in a fresh boot (the verdict of record for a failure).
#   4. imgcmp.sh → imgcmp.txt (NONDET images graded against bare metal's measured spread), compare.py →
#      <resdir>/verdict.md and GSET_SUITE_VERDICT.
# env: KF3_BIN (default: kf3-bins/<this rev>), NVKVM_RAM_MB (12288), KF_SMP (6), GSET_NO_HOST=1 (reuse
#      an existing baseline), GSET_NO_ISOLATE=1, GSET_GUEST_PM=1 (guest persistence mode; default OFF),
#      GSET_NOISE_DIRS="<dir>..." (extra bare-metal runs of the NONDET items, e.g. host.sh <dir> blender_cycles_cuda
#      blender_cycles_optix repeated: imgcmp.sh pools them into the spread; same box/image only).
# ⊘ Strictly serial; takes /tmp/kayfabe-fastguest.lock per boot and releases it between boots.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../../.." && pwd)"
RUN=${1:?run}; shift
R=${GSET_RESULTS:-/workspace/gfxset/results}/$RUN; mkdir -p "$R"
REV=$(git -C "$REPO" rev-parse --short=8 HEAD)
say(){ echo "[gset-suite $(date -Is)] $*" | tee -a "$R/suite.log"; }
say "GSET_SUITE_STARTED run=$RUN rev=$REV"
busy(){ pgrep -x qemu-system-x86 >/dev/null || pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; }
while busy; do say "waiting: a QEMU/cargo is running (serial bench)"; sleep 20; done
ITEMS=${*:-}
# ★ GSET_HOST_FROM=<run>: reuse that run's bare-metal baseline (valid only on the SAME box, GPU, guest
#   image and item code — say so in the report) and measure on bare metal only the items it lacks.
if [ -n "${GSET_HOST_FROM:-}" ]; then
    F=$(dirname "$R")/$GSET_HOST_FROM
    for x in host.res host.dig host2.res host2.dig; do cp -f "$F/$x" "$R/$x" 2>/dev/null; done
    # the bare-metal artefacts too (imgcmp.sh grades NONDET images against them) — linked, not copied
    mkdir -p "$R/h2"
    for t in "$F"/*.host_art.tar; do [ -f "$t" ] && ln -sf "$t" "$R/$(basename "$t")"; done
    for t in "$F"/h2/*.host_art.tar; do [ -f "$t" ] && ln -sf "$t" "$R/h2/$(basename "$t")"; done
    want=${ITEMS:-$(bash "$HERE/items.sh" x list 2>/dev/null)}
    miss=""; for a in $want; do grep -q "item=$a " "$R/host.res" 2>/dev/null || miss="$miss $a"; done
    say "host baseline reused from $GSET_HOST_FROM; measuring on bare metal only:${miss:- (none)}"
    if [ -n "$miss" ]; then
        exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
        mkdir -p "$R/hm1" "$R/hm2"
        bash "$HERE/host.sh" "$R/hm1" $miss > "$R/hm1/host.console" 2>&1
        bash "$HERE/host.sh" "$R/hm2" $miss > "$R/hm2/host.console" 2>&1
        flock -u 9; exec 9>&-
        cat "$R/hm1/host.res" >> "$R/host.res"; cat "$R/hm1/host.dig" >> "$R/host.dig" 2>/dev/null
        cat "$R/hm2/host.res" >> "$R/host2.res"; cat "$R/hm2/host.dig" >> "$R/host2.dig" 2>/dev/null
    fi
    GSET_NO_HOST=1
fi
if [ "${GSET_NO_HOST:-0}" != 1 ]; then
    exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
    rm -f "$R"/host.res "$R"/host.dig "$R"/host2.res "$R"/host2.dig
    bash "$HERE/host.sh" "$R" $ITEMS 2>&1 | tee -a "$R/suite.log" | grep -E 'GSET_RES|HOST_DONE|⊘'
    # run 2 = the noise floor of the CONTENT digests; the perf-only items (their digests are test-set shapes,
    # not content) are skipped there to save ~15 min — compare.py treats them as not re-measured
    I2=""; for a in ${ITEMS:-$(sed -n 's/^GSET_RES side=host item=\([^ ]*\) .*/\1/p' "$R/host.res")}; do
        case " ${GSET_HOST2_SKIP:-vkpeak geekbench_vulkan blender_opendata} " in *" $a "*) ;; *) I2="$I2 $a";; esac; done
    # ⊘ an empty I2 must not reach host.sh, whose empty argument list means EVERY item
    if [ -n "$(echo $I2)" ]; then
        mkdir -p "$R/h2" && bash "$HERE/host.sh" "$R/h2" $I2 > "$R/h2/host.console" 2>&1
        cp -f "$R/h2/host.res" "$R/host2.res"; cp -f "$R/h2/host.dig" "$R/host2.dig"
    fi
    flock -u 9; exec 9>&-
fi
[ -n "$ITEMS" ] || ITEMS=$(sed -n 's/^GSET_RES side=host item=\([^ ]*\) .*/\1/p' "$R/host.res" | tr '\n' ' ')
QB=${KF3_BIN:-/workspace/bench/kf3-bins/$REV/qemu-system-x86_64}
[ -x "$QB" ] || { say "⊘ no kf3 binary at $QB — run scripts/bench/build_kf3.sh from this checkout"; exit 2; }
say "kf3 binary: $QB  items: $ITEMS"
boot(){  # $1 tag, $2 items, $3 resdir
    exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
    env KF_DEVICE=kf3 QEMU_BIN="$QB" NVKVM_RAM_MB=${NVKVM_RAM_MB:-12288} KF_SMP=${KF_SMP:-6} GQ_TIMEOUT=300 \
        GSET_ITEMS="$2" GSET_RES_DIR="$3" POST_CAPTURE_HOOK="$HERE/hook.sh" \
        bash "$REPO/scripts/bench/boot_capture.sh" "$1" > "$3/boot_$1.driver.log" 2>&1
    local rc=$?
    flock -u 9; exec 9>&-
    for x in dmesg dmesg_after probe hostdmesg serial; do cp -f "/workspace/bench/run_$1_$x.log" "$3/boot_$1.$x.log" 2>/dev/null; done
    zstd -q -f "/workspace/bench/run_$1_qemu.log" -o "$3/boot_$1.qemu.log.zst" 2>/dev/null
    say "boot $1 rc=$rc items=[$2] :: $(grep -a 'GSET_HOOK\|FAILED\|⊘' "$3/boot_$1.driver.log" "/workspace/bench/run_$1_probe.log" 2>/dev/null | tail -2 | tr '\n' ' ' | cut -c1-220)"
    while busy; do sleep 3; done
}
# ---- phase 2: batched --------------------------------------------------------------------------
todo="$ITEMS"; n=0
while [ -n "$(echo $todo)" ]; do
    n=$((n+1)); tag="gs_${RUN}_b$n"
    boot "$tag" "$todo" "$R"
    got=$(grep -a "boot=$tag " "$R/guest.res" 2>/dev/null | sed -n 's/.* item=\([^ ]*\) .*/\1/p')
    if [ -z "$got" ]; then
        first=$(echo $todo | cut -d' ' -f1)
        echo "GSET_RES side=guest item=$first verdict=BOOT_FAIL rc=- secs=- note=boot-produced-no-item-result boot=$tag" | tee -a "$R/guest.res"
        got=$first
    fi
    new=""; for a in $todo; do grep -qx "$a" <<<"$got" || new="$new $a"; done; todo=$new
    [ $n -ge 30 ] && { say "⊘ 30 boots — stopping"; break; }
done
# ---- phase 3: every non-PASS item alone, in a fresh boot ----------------------------------------
if [ "${GSET_NO_ISOLATE:-0}" != 1 ]; then
    for a in $(grep -a '^GSET_RES' "$R/guest.res" | grep -v 'verdict=PASS' | sed -n 's/.* item=\([^ ]*\) .*/\1/p' | sort -u); do
        mkdir -p "$R/iso"; boot "gs_${RUN}_i_$a" "$a" "$R/iso"
    done
fi
bash "$HERE/imgcmp.sh" "$R" > "$R/imgcmp.txt" 2>&1
python3 "$HERE/compare.py" "$R" | tee "$R/verdict.md"
S=$(grep -o 'pass=[0-9]*/[0-9]*' "$R/verdict.md" | tail -1)
if [ "${S#pass=}" != "" ] && [ "$(echo "${S#pass=}" | cut -d/ -f1)" = "$(echo "${S#pass=}" | cut -d/ -f2)" ]; then v=PASS; else v=FAIL; fi
say "GSET_SUITE_VERDICT=$v ($S) rev=$REV"
say "GSET_SUITE_EXIT=$(date -Is)"
