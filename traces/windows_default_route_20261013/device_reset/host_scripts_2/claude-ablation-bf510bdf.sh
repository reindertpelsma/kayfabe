#!/usr/bin/env bash
# Trusted host only. Unattended campaign for the flag-free Windows default route (source ec45aad99), run AFTER the regression matrix:
#   A residual apps, B perf A/B + thread profile, C one-flag-off ablation of the production behaviour flags.
# Caller: `flock -o /tmp/kayfabe-fastguest.lock bash this.sh` (the lock is released when the matrix is done). Stop between runs: touch /tmp/kf-stop-claude-ablation
set -uo pipefail
REV=bf510bdf405e72cf11469cd5756b36d9dd1dfd10; SHORT=${REV:0:8}
W=/var/lib/kf-windows-20261005; TREE=/data/kf-reset-host-${REV:0:9}
ROOT=${CAMPAIGN_ROOT:-$W/claude-ablation-${REV:0:9}}
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_claude.sh
POST=/tmp/claude-post-noiommu.sh
FLOOR=107374182400; STOP=/tmp/kf-stop-claude-ablation; KEEP=${KEEP:-0}
APPS_FLAGS="KF3_WIN_USER_CHANNELS_PASSTHROUGH KF3_WIN_TWIN_DEFAPI_OBJECT KF3_KERNEL_GR_CE KF3_KERNEL_GR_WORK KF3_TRANSLATED_CE_RELAY KF3_DEFERRED_API KF3_PREEMPT_BIND_PROBE KF3_DISPLAY_IMP_ENABLE KF3_DISPLAY_CTRL_PROBE KF3_ZCULL_BIND_PROBE KF3_SW_RUNLIST_HOST_OWNED KF3_ASYNC_PREEMPT KF3_GFX_POOL_PROBE KF3_SW_RUNLIST_PROBE KF3_MEMORY_LIST_PROBE KF3_DISPLAY_SDR_COLOR KF3_DISPLAY_CAPS_PROBE KF3_DISPLAY_LUT_MIRROR KF3_DISPLAY_ILUT_OFFSET_256"

vkstats() { # run_number label : GravityMark frame statistics from the retained guest disk, then (unless KEEP=1) delete the disposable overlay
  local n=$1 label=$2 q=$W/boundary-kayfabe-$1/windows.qcow2 M=/mnt/kfperf/c
  [ -f "$q" ] || return 0
  mkdir -p $M
  if qemu-nbd -r -c /dev/nbd0 "$q" 2>/dev/null; then
    sleep 2; partprobe /dev/nbd0 2>/dev/null; sleep 1
    if mount -t ntfs-3g -o ro,force /dev/nbd0p3 $M 2>/dev/null; then
      python3 -I - "$M/kf/out" "$label" "$n" >> "$ROOT/perf.jsonl" <<'PY'
import re,sys,os,json
root,label,n=sys.argv[1:4]
for app in sorted(os.listdir(root)) if os.path.isdir(root) else []:
    p=os.path.join(root,app,"gm_times.txt")
    if not os.path.exists(p): continue
    ts=[];dl=[]
    for l in open(p,errors="ignore"):
        m=re.match(r"\s*(\d+):\s+([\d.]+)\s+([\d.]+)",l)
        if m: ts.append(float(m.group(2))); dl.append(float(m.group(3)))
    if len(ts)<10: continue
    dur=ts[-1]-ts[0]; d=sorted(dl[1:])
    print(json.dumps(dict(label=label,run=int(n),app=app,frames=len(ts),secs=round(dur,1),avg_fps=round(len(ts)/dur,1),med_ms=round(1000*d[len(d)//2],2),p99_ms=round(1000*d[int(len(d)*0.99)],2))))
PY
      umount $M 2>/dev/null
    fi
    qemu-nbd -d /dev/nbd0 >/dev/null 2>&1
  fi
  [ "$KEEP" = 1 ] || rm -f "$q"
}

cleanup_overlays() { # base : remove disposable overlays (children of baseline only) of base..base+5
  local n q
  for ((n=$1;n<$1+6;n++)); do
    q=$W/boundary-kayfabe-$n/windows.qcow2
    [ -f "$q" ] && qemu-img info "$q" 2>/dev/null | grep -q "backing file: $W/baseline/windows.qcow2" && rm -f "$q"
  done
}

run_apps() { # label base "extra drops" fb_mb "apps" [sample]
  local label=$1 base=$2 drops=$3 fb=$4 apps=$5 sample=${6:-0} spid="" n
  [ -e "$STOP" ] && { echo "STOP file present, campaign halted before $label"; return 9; }
  if [ "$(df -B1 --output=avail /var/lib | tail -1)" -le $FLOOR ]; then echo "free space below floor before $label, halting"; return 20; fi
  mkdir -p "$ROOT/$label"
  echo "RUN $label base=$base drops='KF3_TIMER_MAP $drops' fb=$fb apps=$apps at=$(date -Is)"
  if [ "$sample" = 1 ]; then
    ( while true; do pid=$(pgrep -f "qemu-system.*windows-run${base}\\.sock" | head -1); if [ -n "$pid" ]; then echo "== $(date +%T) pid=$pid"; top -H -b -n 2 -d 2 -w 170 -p "$pid" | tail -n 17; fi; sleep 8; done ) >> "$ROOT/$label/threads.txt" 2>&1 &
    spid=$!
    ( sleep 100; pid=$(pgrep -f "qemu-system.*windows-run${base}\\.sock" | head -1); [ -n "$pid" ] && timeout 40 perf stat -a -e kvm:kvm_exit -- sleep 20 >> "$ROOT/$label/kvm_exit.txt" 2>&1 ) &
  fi
  WIN_DROP="KF3_TIMER_MAP $drops" WIN_FB_MB="$fb" WIN_FLAGS="${EXTRA_WIN_FLAGS:-}" bash scripts/bench/windows/appmatrix/run_windows_apps.sh --run-dir "$ROOT/$label" --kf3-rev "$SHORT" --broker "$B" --run-base "$base" \
      --override-dir "$W/codex-matrix-focus-tools-20261011" --iso "$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img" \
      --apps "$apps" --tier 3 --per-guest 8 --max-tdr 99 --no-isolate >> "$ROOT/console.log" 2>&1
  echo "RUN_EXIT $label rc=$? at=$(date -Is)"
  [ -n "$spid" ] && kill "$spid" 2>/dev/null
  # flag audit: no tracing/measurement flags may be present in a timed run
  if [ -f "$W/boundary-kayfabe-$base/command.json" ]; then
    python3 -I -c "import json,sys,re; d=json.load(open(sys.argv[1])); f=sorted(d.get('flags',{})); print('FLAGS',len(f),' '.join(x for x in f)); bad=[x for x in f if re.search('TRACE|MAPLOG|CENSUS|SNAPSHOT|OBSERVER|PEEK|DIAG',x)]; print('FLAG_AUDIT','VIOLATION '+' '.join(bad) if bad else 'ok')" "$W/boundary-kayfabe-$base/command.json" | tee "$ROOT/$label/flags.txt"
  fi
  for ((n=base;n<base+6;n++)); do vkstats $n "$label"; done
  cleanup_overlays "$base"
  bash "$POST" >/dev/null 2>&1 || true
}

if [ "${1:-}" = selftest ]; then ROOT=/tmp/kf-selftest-campaign; KEEP=1; mkdir -p $ROOT; vkstats 1660 selftest; cat $ROOT/perf.jsonl; exit 0; fi

# ---- preconditions
test "$(git -C $TREE rev-parse HEAD)" = "$REV" || exit 10
test -z "$(git -C $TREE status --porcelain --untracked-files=no)" || exit 11
! pgrep qemu-system || exit 12
case "$(cat /sys/kernel/iommu_groups/11/type)" in DMA-FQ|identity) ;; *) exit 13;; esac
test ! -e "$ROOT" || exit 14
mkdir -p "$ROOT"; date -Is > "$ROOT/START"
trap 'rc=$?; echo "$rc" > "$ROOT/EXIT"; echo CLAUDE_CAMPAIGN_EXIT=$rc at=$(date -Is)' EXIT
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export WIN_CPU="host,-vmx,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies"
export KF_LOCKED=1 WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_DISABLE_MPO=1 PROD_EXTRA_FLAGS=
export WIN_PRE_GUEST_CMD="test \$(df -B1 --output=avail /var/lib | tail -1) -gt $FLOOR && { [ \"\$(cat /sys/kernel/iommu_groups/11/type)\" = identity ] || bash $W/pti-iommu_nogdm.sh identity; }"
export WIN_POST_GUEST_CMD="bash $POST"
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"; export KF_GUEST_PW
cd "$TREE"



echo "STAGE C one-flag-off ablation of the production behaviour flags (boot + video + CUDA + GL + 3 min Vulkan)"
i=0
for f in $APPS_FLAGS; do
  run_apps "abl_${f#KF3_}" $((2200 + 10*i)) "$f" 4096 "edge_video_h264,vectorAdd_demo,gputest_triangle,gravitymark_vk" || break
  i=$((i+1))
done
echo "CAMPAIGN_DONE at=$(date -Is)"
