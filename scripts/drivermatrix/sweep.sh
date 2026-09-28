#!/usr/bin/env bash
# ★ THE DRIVER-MATRIX SWEEP — host driver x guest driver on ONE GPU box, i.e. on one GPU
# architecture. The third axis (owner ruling 2026-09-28, `docs/OWNER_RULINGS.md` C.5: every family
# Turing and newer) is covered by running this once per rented architecture; `matrix_table.py`
# prints one host x guest grid per architecture. Design: `docs/design/V3_DRIVER_MATRIX.md` §5.0.
#
#   usage — on a READY box (`provision_full.sh`), as root, no TTY; from a cloud session a detached job:
#     vx -b sweep 'TAG=<tag> KF_REV=<ref> bash /root/kayfabe/scripts/drivermatrix/sweep.sh'
#     DRY_RUN=1 TAG=<tag> [...] bash scripts/drivermatrix/sweep.sh    # the plan only; nothing runs
#
#   inputs (env; a list is space- or comma-separated):
#     TAG            the run's name, [A-Za-z0-9_]+ (required); everything lands in $SWEEP_ROOT/<TAG>/
#     KF_REV         the kayfabe revision measured (a branch, tag or sha; fetched from origin), checked
#                    out ONCE as a detached worktree <dir>/src; the runner re-executes from there, so
#                    every script, the kf3 binary and the raw client are that revision's. Unset: this
#                    checkout (refused if dirty). ⚠ The revision must contain this runner.
#     HOSTS          host drivers, in order (default below: the reference host first — no swap)
#     GUESTS         the full guest list, measured on REF_HOST only
#     MIXED          extra ladder pairs off the reference host, `<host>:<guest>`; `*:<guest>` = every host
#                    but the reference (default `*:590.48.01 *:575.57.08` — the host walk's pairs;
#                    MIXED= : none)
#     REF_HOST       580.159.04  the host every guest is measured on
#     DEFAULT_GUEST  580.159.04  the guest every host is measured with
#     BASE_GUEST     580.159.04  the driver the bench image `guest.qcow2` carries (provision_bench_tree.sh);
#                                that guest's ladder boots the image itself, every other one an overlay
#     THIN_GUEST_RE  ^580\.      who gets the thin 30-arm suite (ruling 1: the grader is a 580 RM client)
#     BUDGET 180  LADDER_REPS 1  BARE_RUNGS cup2  CANARY_ARMS --timer
#     RUN_TESTS 0     1: `cargo test` every kf-* crate once (GPU-free)
#     RETRY_FAILED 0  1: re-run rows (and state steps) whose EXIT recorded a non-zero rc
#     RESTORE_REF 1   swap back to REF_HOST at the end, so the box is left as provisioned
#     SWEEP_ROOT /workspace/bench/sweep   QEMU_SRC /workspace/bench/qemu-10.2.4
#     SWEEP_LOG       DRY_RUN only: preview the plan against this log (e.g. a pulled copy)
#     T_*             step timeouts in seconds (below)
#
# ## Output — the queue-log lines `matrix_table.py` already parses, each result line + `arch=<die>`
#
#   <dir>/summary/sweep_<TAG>.log, one line per event, appended as it happens:
#     SWEEP_START <ts> rev=<rev> tag=<TAG> arch=<die>          (every invocation: a resume appends)
#     SWEEP_ARCH arch=<die> src=<pci.ids|raw> gpu="…" pci=10de:<dev> family="…" bdf=… lspci="…"
#     CLIENT_RC=<rc>   KF3_BUILD_RC=<rc> KF3_BUILT …   [TESTS_RC=<rc> passed=<n> failed=<n>]
#     SWEEP_HOST_START <ts> rev=<rev> host=<v> arch=<die>
#     SWAP host=<v> rc=<rc> got=<v> OPEN_MODULE=<yes|no> … | SWAP host=<v> already installed | HOSTROW host=<v> SWAP_FAILED
#     BARE host=<v> cup2 rc=<rc> CE rv=… -> PASS verdict=PASS             (bare metal, same box, first)
#     GATES host=<v> V3_GATES_SUMMARY pass=<p> fail=<f>
#     CANARY host=<v> verdict=<PASS|FAIL> thin=<p>/<n> guest=<v>           (gates the 30-arm suites)
#     MATRIX_ROW host=<v> guest=<v> rev=<rev> thin=<p>/<n> ladder=-        (guest_walk.sh's own line)
#     LADDER host=<v> guest=<v> rev=<rev> <k>/<n>  |  LADDER guest=<v> UNSTAGED
#     STAGE <v> rc=<rc> …   FAT <v> rc=<rc> …                              (guest staging, when it runs)
#     SWEEP_ROW_START <ts> row=<id>  …  SWEEP_ROW_EXIT <ts> row=<id> rc=<rc> secs=<s>
#     SWEEP_EXIT <ts> rc=<rc> rows_run=<n>
#   <dir>/rows/<id>.log   every step's whole output;   gates/  swaps/  ARCH
#   <dir>/summary_<TAG>.tar.xz  summary/ gates/ swaps/ ARCH — text only, refreshed after every host;
#       unpacked under traces/driver_matrix/walk/<TAG>/ it is what `matrix_table.py` reads.
#
# ## Resumable — boxes vanish
#
# A row with a SWEEP_ROW_EXIT line is never run again (RETRY_FAILED=1: unless its rc was non-zero).
# Its result lines are written BEFORE its EXIT, so an EXIT means the result is in the log. State
# steps — the host swap, guest staging, the builds — are decided on CONTENT (the loaded driver, the
# STAGED markers, cargo's fingerprints), so a resume on the same box skips them and a resume on a
# NEW box (the pulled log `vput` back to <dir>/summary/) redoes them; only a state step that FAILED
# is remembered by its EXIT. A log from another revision or another architecture is refused: one
# TAG = one revision on one architecture.
#
# ⊘ No `set -e`: one failing row never stops the sweep. Every step has a bound (`timeout -k`, which
# signals the step's whole process group). A dead GPU — `nvidia-smi` fails, or the host kernel logs
# new bad-register reads (the GFW-boot wedge of 2026-09-26), and an FLR does not bring it back —
# stops the sweep by name instead of recording rows of noise.
# ⊘ Bare metal first on every host (`cuda_ladder.sh host`): bare metal passes and the guest fails
# ⇒ the defect is kayfabe's (CLAUDE.md, *Verification*).
set -uo pipefail
SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
REPO="$(cd "$(dirname "$SELF")/../.." && pwd)"
DRY=${DRY_RUN:-0}
TAG=${TAG:-${1:-}}
[[ "$TAG" =~ ^[A-Za-z0-9_]+$ ]] || { sed -n '2,33p' "$SELF"; echo "sweep: TAG=<[A-Za-z0-9_]+> is required"; exit 2; }
SWEEP_ROOT=${SWEEP_ROOT:-/workspace/bench/sweep}
DIR=$SWEEP_ROOT/$TAG
BENCH=${BENCH_DIR:-/workspace/bench}
DRIVERS=${KF_DRIVER_STAGE:-/workspace/drivers}
QEMU_SRC=${QEMU_SRC:-$BENCH/qemu-10.2.4}
QEMU_BUILD=${QEMU_BUILD:-$BENCH/qemu-build-kf3}
export BENCH_DIR=$BENCH KF_DRIVER_STAGE=$DRIVERS KF_DEVICE=kf3 PATH=$HOME/.cargo/bin:$PATH
# ⊘ Overrides that would decouple a step from THIS revision's binaries or from the row's guest.
unset QEMU_BIN CLIENT KF_GUEST_IMG KF3_DEV_EXTRA KF_FASTGUEST_DIR KF_ARMS GUEST_WALK_ARMS

REF_HOST=${REF_HOST:-580.159.04}
DEFAULT_GUEST=${DEFAULT_GUEST:-580.159.04}
BASE_GUEST=${BASE_GUEST:-580.159.04}
THIN_GUEST_RE=${THIN_GUEST_RE:-'^580\.'}
BUDGET=${BUDGET:-180}
LADDER_REPS=${LADDER_REPS:-1}
CANARY_ARMS=${CANARY_ARMS:---timer}
split() { echo "${1//,/ }"; }
# Defaults: the §6.0 walk, whole — every host and guest version the 2026-09-26 walk measured or queued.
read -r -a H_LIST <<< "$(split "${HOSTS:-$REF_HOST 580.95.05 580.65.06 575.57.08 570.148.08 565.57.01 550.54.14 535.309.01 590.48.01 595.84 610.57.04}")"
read -r -a G_LIST <<< "$(split "${GUESTS:-580.159.04 580.105.08 580.65.06 580.95.05 580.126.09 580.173.02 580.178.04 590.48.01 595.84 610.57.04 575.57.08 570.148.08 565.57.01 550.54.14 535.309.01}")"
read -r -a M_LIST <<< "$(split "${MIXED-*:590.48.01 *:575.57.08}")"
read -r -a RUNGS <<< "$(split "${BARE_RUNGS:-cup2}")"
T_BUILD=${T_BUILD:-5400} T_TESTS=${T_TESTS:-3600} T_SWAP=${T_SWAP:-2400} T_BARE=${T_BARE:-1800}
T_GATES=${T_GATES:-3600} T_CANARY=${T_CANARY:-1200} T_THIN=${T_THIN:-10800} T_STAGE=${T_STAGE:-2400}
T_FAT=${T_FAT:-4800} T_LADDER=${T_LADDER:-$(( 4 * LADDER_REPS * 900 + 600 ))}

bad=""
for v in "${H_LIST[@]}" "${G_LIST[@]}" "$REF_HOST" "$DEFAULT_GUEST" "$BASE_GUEST"; do
    [[ "$v" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]] || bad="$bad $v"
done
for p in ${M_LIST[@]+"${M_LIST[@]}"}; do
    [[ "$p" =~ ^(\*|[0-9]+\.[0-9]+(\.[0-9]+)?):[0-9]+\.[0-9]+(\.[0-9]+)?$ ]] || bad="$bad $p"
done
[ -z "$bad" ] || { echo "SWEEP_REFUSED: not a driver version (or <host>:<guest> pair):$bad"; exit 2; }

# ── the revision: one worktree per TAG, the runner re-executed from it ─────────────────────────
if [ -n "${KF_REV:-}" ] && [ "${SWEEP_IN_WORKTREE:-0}" != 1 ]; then
    WT=$DIR/src
    if [ "$DRY" = 1 ]; then
        echo "PLAN worktree $WT at $KF_REV (fetched from origin; created on the first real run); planned from this checkout"
    else
        mkdir -p "$DIR"
        if [ ! -e "$WT/.git" ]; then
            timeout 300 git -C "$REPO" fetch -q origin 2>/dev/null
            sha=$(git -C "$REPO" rev-parse -q --verify "origin/$KF_REV^{commit}" 2>/dev/null \
                  || git -C "$REPO" rev-parse -q --verify "$KF_REV^{commit}" 2>/dev/null)
            [ -n "$sha" ] || { echo "SWEEP_REFUSED: KF_REV=$KF_REV is not a commit here or on origin"; exit 2; }
            git -C "$REPO" worktree add -f --detach "$WT" "$sha" >/dev/null 2>&1 \
                || { echo "SWEEP_REFUSED: git worktree add $WT $sha failed"; exit 2; }
        fi
        [ -f "$WT/scripts/drivermatrix/sweep.sh" ] || {
            echo "SWEEP_REFUSED: revision $(git -C "$WT" rev-parse --short=8 HEAD) has no scripts/drivermatrix/sweep.sh — measure a revision that contains the runner"; exit 2; }
        exec env SWEEP_IN_WORKTREE=1 bash "$WT/scripts/drivermatrix/sweep.sh" "$@"
    fi
fi
REV=$(git -C "$REPO" rev-parse --short=8 HEAD 2>/dev/null || echo unknown)
if [ -n "$(git -C "$REPO" status --porcelain --untracked-files=no 2>/dev/null)" ]; then
    REV="$REV-dirty"
    # ⊘ A bench claim carries its source revision; a dirty tree has none (and matrix_table.py's
    # `rev=(\w+)` would drop the rows).
    [ "$DRY" = 1 ] || [ "${SWEEP_ALLOW_DIRTY:-0}" = 1 ] || {
        echo "SWEEP_REFUSED: $REPO is dirty — measure a revision (KF_REV=<ref>) or commit first"; exit 2; }
fi

LOG=$DIR/summary/sweep_$TAG.log
[ "$DRY" = 1 ] && LOG=${SWEEP_LOG:-$LOG}
emit() { if [ "$DRY" = 1 ]; then echo "  would log: $*"; else echo "$*" | tee -a "$LOG"; fi; }
ROWS_RUN=0
declare -A T0
row_rc() {  # the rc of the row's LAST recorded EXIT, or nothing
    [ -f "$LOG" ] || return 0
    awk -v id="$1" '$1 == "SWEEP_ROW_EXIT" && $3 == "row=" id {
        rc = ""; for (i = 4; i <= NF; i++) if ($i ~ /^rc=/) rc = substr($i, 4) } END { print rc }' "$LOG"
}
row_done() { local rc; rc=$(row_rc "$1"); [ -n "$rc" ] || return 1; [ "$rc" != 0 ] && [ "${RETRY_FAILED:-0}" = 1 ] && return 1; return 0; }
failed_before() { local rc; rc=$(row_rc "$1"); [ -n "$rc" ] && [ "$rc" != 0 ] && [ "${RETRY_FAILED:-0}" != 1 ]; }
row_start() { T0[$1]=$SECONDS; emit "SWEEP_ROW_START $(date -Is) row=$1"; }
row_end() { emit "SWEEP_ROW_EXIT $(date -Is) row=$1 rc=$2 secs=$(( SECONDS - ${T0[$1]:-$SECONDS} ))"; ROWS_RUN=$((ROWS_RUN + 1)); }
row_begin() {  # <id> <plan text> → 0: run it now; 1: skip (recorded, or DRY_RUN)
    if row_done "$1"; then echo "skip row=$1 (EXIT rc=$(row_rc "$1") recorded)"; return 1; fi
    if [ "$DRY" = 1 ]; then echo "PLAN row=$1: $2"; return 1; fi
    row_start "$1"; return 0
}
rowlog() { echo "$DIR/rows/${1//[^A-Za-z0-9._-]/_}.log"; }
step() {  # <timeout-s> <log> <cmd…> → the command's rc (124/137: the bound expired)
    local to=$1 log=$2; shift 2
    echo "== $(date -Is) [timeout ${to}s] $*" >> "$log"
    timeout -k 60 "$to" "$@" < /dev/null >> "$log" 2>&1 8>&-
}
refusals() {  # the named refusals of a step's QEMU logs, counted — the host axis' first question
    # shellcheck disable=SC2068  # a glob list on purpose
    grep -ahE "HOST-ABI REFUSED|has no per-map PTE kind|UNSERVICED|RPC-REFUSED" $@ 2>/dev/null \
        | sed 's/^\[[^]]*\] //' | cut -c1-240 | sort | uniq -c | sort -rn | head -8
}

# ── the architecture: DERIVED on the box, never typed ─────────────────────────────────────────
# The die name is pciutils' `pci.ids` name for the host GPU's PCI id (e.g. `NVIDIA Corporation GA102
# [GeForce RTX 3090] [10de:2204]` → GA102) — an upstream database, not a table kept here. A device
# `pci.ids` does not know yet is looked up once more after `update-pciids`, then recorded RAW as
# `10de:<device>`. The GPU's name, device id and `nvidia-smi`'s Product Architecture ride beside it.
nvq() { timeout 60 nvidia-smi -i 0 --query-gpu="$1" --format=csv,noheader 2>/dev/null | head -1; }
die_of() { sed -n 's/.*NVIDIA Corporation \([A-Z][A-Z][0-9][0-9][0-9]\)[A-Z]*[[ ].*/\1/p' <<< "$1" | head -1; }
derive_arch() {
    local bus name did fam line="" die="" dev="" src=raw n
    bus=$(nvq pci.bus_id); name=$(nvq name); did=$(nvq pci.device_id)
    fam=$(timeout 60 nvidia-smi -q -i 0 2>/dev/null | sed -n 's/^ *Product Architecture *: *//p' | head -1)
    n=$(timeout 60 nvidia-smi -L 2>/dev/null | grep -c '^GPU ')
    BDF=""
    [ -n "$bus" ] && BDF=$(tr 'A-F' 'a-f' <<< "${bus: -12}")
    if [ -z "$BDF" ] && command -v lspci >/dev/null 2>&1; then
        BDF=$(lspci -D -n -d 10de: 2>/dev/null | awk '$2 ~ /^030[02]:/ { print $1; exit }')
    fi
    [[ "$did" =~ ^0x[0-9A-Fa-f]{8}$ ]] && dev=$(printf '%04x' $(( (did >> 16) & 0xffff )))
    if command -v lspci >/dev/null 2>&1 && [ -n "$BDF" ]; then
        line=$(lspci -nn -s "$BDF" 2>/dev/null | head -1)
        [ -n "$dev" ] || dev=$(sed -n 's/.*\[10de:\([0-9a-f]\{4\}\)\].*/\1/p' <<< "$line")
        die=$(die_of "$line")
        if [ -z "$die" ] && [ "$DRY" != 1 ] && command -v update-pciids >/dev/null 2>&1; then
            timeout 90 update-pciids -q >/dev/null 2>&1
            line=$(lspci -nn -s "$BDF" 2>/dev/null | head -1); die=$(die_of "$line")
        fi
        [ -n "$die" ] && src=pci.ids
    fi
    ARCH=${die:-10de:${dev:-unknown}}
    ARCH_LINE="SWEEP_ARCH arch=$ARCH src=$src gpu=\"${name:-?}\" pci=10de:${dev:-?} family=\"${fam:-?}\" gpus=${n:-?} bdf=${BDF:-?} lspci=\"$line\""
}

# ── GPU health: before every GPU step ─────────────────────────────────────────────────────────
DMESG_BAD=0
bad_kernel_lines() { dmesg 2>/dev/null | grep -Eac 'Possible bad register read|fallen off the bus|RmInitAdapter failed'; }
flr() {  # the recovery that brought the 2026-09-26 GFW-boot wedge back (V3_DRIVER_MATRIX §6, 13b25624)
    systemctl stop nvidia-persistenced 2>/dev/null
    for m in nvidia_uvm nvidia_drm nvidia_modeset nvidia; do rmmod "$m" 2>/dev/null; done
    [ -n "${BDF:-}" ] && [ -w "/sys/bus/pci/devices/$BDF/reset" ] && echo 1 > "/sys/bus/pci/devices/$BDF/reset"
    sleep 2; modprobe nvidia; modprobe nvidia_uvm; nvidia-modprobe -c 0 -u 2>/dev/null; true
}
health() {  # <where> → 0: healthy (possibly after an FLR); 1: dead
    [ "$DRY" = 1 ] && return 0
    local n; n=$(bad_kernel_lines)
    if timeout 60 nvidia-smi -L >/dev/null 2>&1 && [ "$n" -le "$DMESG_BAD" ]; then DMESG_BAD=$n; return 0; fi
    emit "HEALTH host=$CUR_HOST at=$1 nvidia-smi=$(timeout 60 nvidia-smi -L >/dev/null 2>&1 && echo ok || echo FAILED) new_bad_kernel_lines=$(( n - DMESG_BAD )) -> FLR arch=$ARCH"
    flr >> "$DIR/rows/health.log" 2>&1
    DMESG_BAD=$(bad_kernel_lines)
    if timeout 60 nvidia-smi -L >/dev/null 2>&1; then emit "HEALTH host=$CUR_HOST at=$1 FLR recovered arch=$ARCH"; return 0; fi
    emit "HEALTH host=$CUR_HOST at=$1 DEAD after FLR arch=$ARCH"; return 1
}
pack() {
    [ "$DRY" = 1 ] && return 0
    tar -C "$DIR" -cJf "$DIR/summary_$TAG.tar.xz.part" summary gates swaps ARCH 2>/dev/null \
        && mv -f "$DIR/summary_$TAG.tar.xz.part" "$DIR/summary_$TAG.tar.xz"
}
finish() {  # <rc>
    emit "SWEEP_EXIT $(date -Is) rc=$1 rows_run=$ROWS_RUN"
    pack; rm -f "$DIR/RUNNING.pid"
    [ "$DRY" = 1 ] || echo "SWEEP_TARBALL $DIR/summary_$TAG.tar.xz"
    exit "$1"
}
abort() { emit "SWEEP_ABORTED $(date -Is) host=$CUR_HOST: $* arch=$ARCH"; finish 3; }

# ── state steps (decided on content; only a FAILED one is remembered) ─────────────────────────
loaded_driver() { timeout 60 nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>/dev/null | head -1; }
ensure_host() {  # <version> → 0: that host driver is loaded (open module)
    local h=$1 rc log got
    if [ "$DRY" = 1 ]; then
        echo "PLAN swap: unless $h is loaded — HOST_DRIVER=$h provision_host_driver.sh (XFree86/ then tesla/; CC = the kernel's compiler)"
        return 0
    fi
    got=$(loaded_driver)
    if [ "$got" = "$h" ] && grep -q "Open Kernel Module" /proc/driver/nvidia/version 2>/dev/null; then
        emit "SWAP host=$h already installed arch=$ARCH"; return 0
    fi
    if failed_before "swap:$h"; then
        emit "HOSTROW host=$h SWAP_FAILED (recorded rc=$(row_rc "swap:$h"); RETRY_FAILED=1 retries) arch=$ARCH"; return 1
    fi
    row_start "swap:$h"; log=$(rowlog "swap:$h")
    step "$T_SWAP" "$log" env HOST_DRIVER="$h" RUN_URL= bash "$REPO/scripts/bench/provision_host_driver.sh"; rc=$?
    got=$(loaded_driver)
    cp "$log" "$DIR/swaps/swap_$h.log" 2>/dev/null
    cp "/root/nvidia-installer-NVIDIA-Linux-x86_64-$h.log" "$DIR/swaps/" 2>/dev/null
    [ "$rc" = 0 ] && [ "$got" != "$h" ] && rc=7
    emit "SWAP host=$h rc=$rc got=${got:-none} $(grep -ao 'OPEN_MODULE=[a-z]*' "$log" | tail -1) $(grep -ao 'KERNEL_CC=[^ ]*' "$log" | tail -1) arch=$ARCH"
    row_end "swap:$h" "$rc"
    [ "$rc" = 0 ] || { emit "HOSTROW host=$h SWAP_FAILED arch=$ARCH"; return 1; }
    DMESG_BAD=$(bad_kernel_lines)
    return 0
}
stage_thin() {  # <guest> → 0: $DRIVERS/<guest>/STAGED (its modules, firmware and .run)
    local g=$1 rc log
    [ -f "$DRIVERS/$g/STAGED" ] && return 0
    failed_before "stage:$g" && return 1
    row_start "stage:$g"; log=$(rowlog "stage:$g")
    step "$T_STAGE" "$log" bash "$REPO/scripts/drivermatrix/stage_guest_driver.sh" "$g" "$DRIVERS"; rc=$?
    emit "STAGE $g rc=$rc $(tail -1 "$log" | cut -c1-200) arch=$ARCH"
    row_end "stage:$g" "$rc"
    [ -f "$DRIVERS/$g/STAGED" ]
}
stage_fat() {  # <guest> → 0: $BENCH/guest-<guest>.qcow2.STAGED
    local g=$1 rc log
    [ -f "$BENCH/guest-$g.qcow2.STAGED" ] && return 0
    stage_thin "$g" || return 1
    failed_before "fat:$g" && return 1
    row_start "fat:$g"; log=$(rowlog "fat:$g")
    step "$T_FAT" "$log" bash "$REPO/scripts/drivermatrix/stage_fat_guest.sh" "$g" "$DRIVERS"; rc=$?
    emit "FAT $g rc=$rc $(tail -1 "$log" | cut -c1-200) arch=$ARCH"
    row_end "fat:$g" "$rc"
    [ -f "$BENCH/guest-$g.qcow2.STAGED" ]
}
guests_for() {  # <host> → its guest set: the default guest; + every guest on the reference host;
                # + the MIXED pairs naming it (`*` = every host but the reference, whose set is GUESTS)
    local h=$1 p out=("$DEFAULT_GUEST")
    [ "$h" = "$REF_HOST" ] && out+=("${G_LIST[@]}")
    for p in ${M_LIST[@]+"${M_LIST[@]}"}; do
        { [ "${p%%:*}" = "$h" ] || { [ "${p%%:*}" = "*" ] && [ "$h" != "$REF_HOST" ]; }; } && out+=("${p#*:}")
    done
    printf '%s\n' "${out[@]}" | awk '!seen[$0]++'
}
canary_passed() { [ "$DRY" = 1 ] || grep -a "^CANARY host=$1 verdict=" "$LOG" 2>/dev/null | tail -1 | grep -q 'verdict=PASS'; }

# ── preconditions, identity, resume checks ────────────────────────────────────────────────────
if [ "$DRY" != 1 ]; then
    pre=""
    [ "$(id -u)" = 0 ] || pre="$pre not-root"
    [ -e /dev/kvm ] || pre="$pre no-/dev/kvm"
    for f in "$BENCH/guest.qcow2" "$BENCH/seed.iso" "$BENCH/guest_key" "$QEMU_SRC/VERSION"; do
        [ -e "$f" ] || pre="$pre missing:$f"
    done
    for c in cargo nvidia-smi timeout flock git; do command -v "$c" >/dev/null 2>&1 || pre="$pre no-$c"; done
    [ -z "$pre" ] || { echo "SWEEP_REFUSED preconditions:$pre (a READY box: provision_full.sh)"; exit 2; }
    mkdir -p "$DIR/summary" "$DIR/rows" "$DIR/gates" "$DIR/swaps"
    exec 8>"$SWEEP_ROOT/.lock"
    flock -n 8 || { echo "SWEEP_REFUSED: another sweep holds $SWEEP_ROOT/.lock (one sweep per box)"; exit 2; }
    echo $$ > "$DIR/RUNNING.pid"
    DMESG_BAD=$(bad_kernel_lines)   # the baseline: only NEW bad-register lines indict the GPU
fi
if [ "$DRY" = 1 ] && ! command -v nvidia-smi >/dev/null 2>&1; then
    ARCH=${SWEEP_DRY_ARCH:-unknown}; BDF=""; ARCH_LINE="SWEEP_ARCH arch=$ARCH src=dry-run (no nvidia-smi here)"
else
    derive_arch
fi
if [ -f "$LOG" ]; then
    rec_rev=$(sed -n 's/^SWEEP_START [^ ]* rev=\([^ ]*\) .*/\1/p' "$LOG" | head -1)
    rec_arch=$(sed -n 's/^SWEEP_START .* arch=\([^ ]*\).*/\1/p' "$LOG" | head -1)
    why=""
    [ -z "$rec_rev" ] || [ "$rec_rev" = "$REV" ] || why="rev=$rec_rev (this checkout is $REV)"
    [ -z "$rec_arch" ] || [ "$rec_arch" = "$ARCH" ] || why="$why arch=$rec_arch (this box is $ARCH)"
    if [ -n "$why" ]; then
        echo "SWEEP_REFUSED: $LOG was measured at $why — one TAG = one revision on one architecture"
        [ "$DRY" = 1 ] || exit 2
    fi
    echo "resuming $LOG: $(grep -c '^SWEEP_ROW_EXIT' "$LOG") row EXITs recorded"
fi
CUR_HOST=$(loaded_driver)
trap 'emit "SWEEP_KILLED $(date -Is) by a signal (host=$CUR_HOST)"; rm -f "$DIR/RUNNING.pid"; exit 143' TERM INT HUP
emit "SWEEP_START $(date -Is) rev=$REV tag=$TAG arch=$ARCH"
emit "$ARCH_LINE"
if [ "$DRY" != 1 ] && [ ! -f "$DIR/ARCH" ]; then
    printf '# The die this sweep ran on, derived by sweep.sh on the box (%s).\n%s\n' "${ARCH_LINE#SWEEP_ARCH }" "$ARCH" > "$DIR/ARCH"
fi
emit "SWEEP_PLAN hosts=${H_LIST[*]} ref_host=$REF_HOST guests=${G_LIST[*]} mixed=${M_LIST[*]:-none} default_guest=$DEFAULT_GUEST thin_re=$THIN_GUEST_RE budget=$BUDGET ladder_reps=$LADDER_REPS bare=${RUNGS[*]}"

# ── build ONCE per revision: every invocation asks cargo, whose fingerprints decide ──────────
if [ "$DRY" = 1 ]; then
    echo "PLAN build: cargo build --release --target x86_64-unknown-linux-musl -p kayfabe-rm-ladder; build_kf3.sh $QEMU_SRC $QEMU_BUILD"
else
    blog=$(rowlog build)
    step "$T_BUILD" "$blog" env -C "$REPO" cargo build --release --target x86_64-unknown-linux-musl -p kayfabe-rm-ladder --bin kayfabe-rm-ladder
    crc=$?; emit "CLIENT_RC=$crc"
    step "$T_BUILD" "$blog" bash "$REPO/scripts/bench/build_kf3.sh" "$QEMU_SRC" "$QEMU_BUILD"
    krc=$?; emit "KF3_BUILD_RC=$krc $(grep -a '^KF3_BUILT' "$blog" | tail -1)"
    { [ "$crc" = 0 ] && [ "$krc" = 0 ]; } || abort "the build failed (rows/build.log)"
fi
if [ "${RUN_TESTS:-0}" = 1 ] && row_begin tests "cargo test -q --no-fail-fast (every kf-* crate)"; then
    pk=(); for c in "$REPO"/crates/kf-*/; do c=${c%/}; pk+=(-p "${c##*/}"); done
    tlog=$(rowlog tests)
    step "$T_TESTS" "$tlog" env -C "$REPO" cargo test -q --no-fail-fast "${pk[@]}"; rc=$?
    emit "TESTS_RC=$rc $(awk '/^test result/ { s += $4; f += $6 } END { print "passed=" s + 0, "failed=" f + 0 }' "$tlog")"
    row_end tests "$rc"
fi

# ── the hosts ─────────────────────────────────────────────────────────────────────────────────
for H in "${H_LIST[@]}"; do
    hs=${H//./}
    mapfile -t GS < <(guests_for "$H")
    thin=(); for g in "${GS[@]}"; do [[ "$g" =~ $THIN_GUEST_RE ]] && thin+=("$g"); done
    rows=(); for r in "${RUNGS[@]}"; do rows+=("bare:$H:$r"); done
    rows+=("gates:$H")
    [ "${#thin[@]}" -gt 0 ] && rows+=("canary:$H")
    for g in ${thin[@]+"${thin[@]}"}; do rows+=("thin:$H:$g"); done
    for g in "${GS[@]}"; do rows+=("ladder:$H:$g"); done
    pending=0; for r in "${rows[@]}"; do row_done "$r" || pending=$((pending + 1)); done
    if [ "$pending" = 0 ]; then echo "host $H: all ${#rows[@]} rows recorded — skipped"; continue; fi
    [ "$DRY" = 1 ] && echo "PLAN host=$H: ${#rows[@]} rows ($pending pending); guests: ${GS[*]}; thin: ${thin[*]:-none}"
    emit "SWEEP_HOST_START $(date -Is) rev=$REV host=$H arch=$ARCH"
    ensure_host "$H" || continue
    CUR_HOST=$H

    for r in "${RUNGS[@]}"; do   # bare metal first: the same workload, no kayfabe
        if row_begin "bare:$H:$r" "cuda_ladder.sh host (bare metal, $r)"; then
            health "bare:$r" || abort "the GPU is dead (rows/health.log)"
            t=${TAG}_h${hs}_bare_${r}_r${REV}; log=$(rowlog "bare:$H:$r")
            step "$T_BARE" "$log" bash "$REPO/scripts/bench/cuda_ladder.sh" host "$t" 1 "$r"; rc=$?
            l=$(grep -a '^CL_ROW mode=host' "$BENCH/cl_${t}_host.out" 2>/dev/null | tail -1)
            if [ -n "$l" ]; then
                emit "BARE host=$H $r rc=$(sed -n 's/.* rc=\([0-9]*\) .*/\1/p' <<< "$l") $(sed -n 's/.*graded=\[\(.*\)\]$/\1/p' <<< "$l") verdict=$(sed -n 's/.*verdict=\([A-Z]*\).*/\1/p' <<< "$l") arch=$ARCH"
            else
                emit "BARE host=$H $r NOTRUN (step rc=$rc, no CL_ROW: rows/bare_${H}_$r.log) arch=$ARCH"; [ "$rc" = 0 ] && rc=4
            fi
            cp "$BENCH/cl_${t}_host.out" "$DIR/summary/" 2>/dev/null
            row_end "bare:$H:$r" "$rc"
        fi
    done

    if row_begin "gates:$H" "v3_gates.sh (9 gates, no guest)"; then
        health gates || abort "the GPU is dead (rows/health.log)"
        glog=$DIR/gates/v3_gates_h${hs}_$REV.log
        step "$T_GATES" "$(rowlog "gates:$H")" bash "$REPO/scripts/bench/v3_gates.sh" "$glog"; rc=$?
        s=$(grep -a '^V3_GATES_SUMMARY' "$glog" 2>/dev/null | tail -1)
        emit "GATES host=$H ${s:-NO_V3_GATES_SUMMARY rc=$rc} arch=$ARCH"
        row_end "gates:$H" "$rc"
    fi

    # ★ The canary: one arm of the first thin guest before 30 x budget is spent on this host.
    # `[measured 2026-09-26, hostwalk]` the first sub-580 host scored 30 TIMEOUTs in 90 minutes.
    if [ "${#thin[@]}" -gt 0 ] && row_begin "canary:$H" "guest_walk.sh ${thin[0]} ($CANARY_ARMS only)"; then
        if stage_thin "${thin[0]}"; then
            health canary || abort "the GPU is dead (rows/health.log)"
            # (its own tag prefix: the thin rows' `fast_<tag>_*` globs must not take its logs)
            t=${TAG}_h${hs}_canary_g${thin[0]//./}_r${REV}; log=$(rowlog "canary:$H")
            step "$T_CANARY" "$log" env GUEST_WALK_ARMS="$CANARY_ARMS" \
                bash "$REPO/scripts/drivermatrix/guest_walk.sh" "${thin[0]}" "$t" "$BUDGET" 0; rc=$?
            th=$(grep -a '^MATRIX_ROW ' "$log" | tail -1 | sed -n 's/.* thin=\([0-9?]*\/[0-9?]*\).*/\1/p')
            v=FAIL; [ -n "$th" ] && [ "${th%/*}" = "${th#*/}" ] && [ "${th#*/}" != 0 ] && v=PASS
            emit "CANARY host=$H verdict=$v thin=${th:-none} guest=${thin[0]} arch=$ARCH"
            # a failed canary is a row RETRY_FAILED=1 re-runs (and with it the thin rows it held back)
            [ "$v" = PASS ] || [ "$rc" != 0 ] || rc=1
            refusals "$BENCH/fast_${t}_*_qemu.log" | while IFS= read -r x; do emit "$x"; done
            cp "$BENCH/${t}_suite.out" "$DIR/summary/" 2>/dev/null
        else
            rc=2; emit "CANARY host=$H verdict=NOTRUN (guest ${thin[0]} unstaged: rows/stage_${thin[0]}.log) arch=$ARCH"
        fi
        row_end "canary:$H" "$rc"
    fi

    for g in ${thin[@]+"${thin[@]}"}; do
        if row_begin "thin:$H:$g" "guest_walk.sh $g (30 arms, ${BUDGET}s each; after stage_guest_driver.sh $g if unstaged)"; then
            if ! canary_passed "$H"; then
                emit "THIN host=$H guest=$g SKIPPED (the canary did not pass on this host) arch=$ARCH"; row_end "thin:$H:$g" 3; continue
            fi
            if ! stage_thin "$g"; then
                emit "THIN host=$H guest=$g UNSTAGED (rows/stage_$g.log) arch=$ARCH"; row_end "thin:$H:$g" 2; continue
            fi
            health "thin:$g" || abort "the GPU is dead (rows/health.log)"
            t=${TAG}_h${hs}_g${g//./}_r${REV}; log=$(rowlog "thin:$H:$g")
            step "$T_THIN" "$log" bash "$REPO/scripts/drivermatrix/guest_walk.sh" "$g" "$t" "$BUDGET" 0; rc=$?
            m=$(grep -a '^MATRIX_ROW ' "$log" | tail -1)
            if [ -n "$m" ]; then emit "$m arch=$ARCH"
            else
                emit "THIN host=$H guest=$g NO_MATRIX_ROW rc=$rc $(grep -a 'GUEST_WALK_REFUSED' "$log" | tail -1) arch=$ARCH"
                [ "$rc" = 0 ] && rc=4
            fi
            grep -a '^FAST_CELL_ARM ' "$BENCH/${t}_suite.out" 2>/dev/null | grep -v 'verdict=PASS' | head -12 \
                | while IFS= read -r x; do emit "$x"; done
            refusals "$BENCH/fast_${t}_*_qemu.log" | while IFS= read -r x; do emit "$x"; done
            cp "$BENCH/${t}_suite.out" "$DIR/summary/" 2>/dev/null
            row_end "thin:$H:$g" "$rc"
        fi
    done

    for g in "${GS[@]}"; do
        img=""; [ "$g" = "$BASE_GUEST" ] || img=$BENCH/guest-$g.qcow2
        if row_begin "ladder:$H:$g" "cuda_ladder.sh guest $g x$LADDER_REPS (${img:-the bench image}${img:+; staged by stage_guest_driver.sh + stage_fat_guest.sh if absent})"; then
            if [ -n "$img" ] && ! stage_fat "$g"; then
                emit "LADDER guest=$g UNSTAGED arch=$ARCH"; row_end "ladder:$H:$g" 2; continue
            fi
            health "ladder:$g" || abort "the GPU is dead (rows/health.log)"
            t=${TAG}_h${hs}_g${g//./}_r${REV}; log=$(rowlog "ladder:$H:$g")
            ev=(KF3_DEV_EXTRA="guest-driver=$g" KF_DEVICE=kf3); [ -n "$img" ] && ev+=(KF_GUEST_IMG="$img")
            step "$T_LADDER" "$log" env "${ev[@]}" bash "$REPO/scripts/bench/cuda_ladder.sh" guest "$t" "$LADDER_REPS"; rc=$?
            clout=$BENCH/cl_${t}_guest.out
            lp=$(grep -ac '^CL_ROW .*verdict=PASS' "$clout" 2>/dev/null); ln=$(grep -ac '^CL_ROW ' "$clout" 2>/dev/null)
            if [ "${ln:-0}" -gt 0 ]; then emit "LADDER host=$H guest=$g rev=$REV ${lp:-0}/$ln arch=$ARCH"
            else emit "LADDER host=$H guest=$g rev=$REV NOTRUN (step rc=$rc, no CL_ROW) arch=$ARCH"; [ "$rc" = 0 ] && rc=4; fi
            grep -a '^CL_ROW ' "$clout" 2>/dev/null | sed 's/ ledger=.*//' | while IFS= read -r x; do emit "$x"; done
            refusals "$BENCH/run_cl_${t}_*_qemu.log" | while IFS= read -r x; do emit "$x"; done
            cp "$clout" "$DIR/summary/" 2>/dev/null
            row_end "ladder:$H:$g" "$rc"
        fi
    done
    emit "SWEEP_HOST_EXIT $(date -Is) host=$H"
    pack
done

if [ "${RESTORE_REF:-1}" = 1 ]; then
    emit "SWEEP_RESTORE host=$REF_HOST"
    ensure_host "$REF_HOST" && CUR_HOST=$REF_HOST
fi
finish 0
