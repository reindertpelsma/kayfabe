#!/usr/bin/env python3
"""A sandbox for scripts/bench/box/specviewperf_stage.sh without a box (GPU-free, stdlib only).

It lays out a fake checkout (the REAL stage script, lane.py, lanes.json, tarfilter.py and the real guest
scripts, which are only copied to the fake guest) next to fakes of everything that needs a box: win_vm.sh
(a bash script over a scenario directory), fetch_bench_data.sh, nvidia-smi, gcc (the host cup8 reference),
qemu-img, free, df, dmesg, id, and a clock that moves only when the stage sleeps (so waits are instant and
the time guard can be driven). Used by test_vast_spec_stage.py and spec_stage_mutants.py.

The fake guest answers the guest script's actions from the files under <root>/scn: `viewset.<W>` (a mode:
pass, fail_subtest, reset, refused, gather, gather_once, no_tdr, hang, dead, silent, scoreleak),
`wall.<W>` (seconds the run takes on the fake clock), `polls.<W>`, `boot_fail.<kind><n>` (refused, bar1,
nossh), `tar_rc`, `verify`, `cli_absent`, `scp_fail`, `fetch`, `cup8_fail_after`, `xid_new`, `block.<W>`.
"""

import hashlib
import io
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

HERE = Path(__file__).resolve().parent
BENCH = HERE.parent / "bench"
IDS = [
    "medical-04", "energy-04", "maya-07", "catia-07", "solidworks-08",
    "creo-04", "unreal_engine-01", "blender-01", "enscape-01",
]
REAL_DATE = shutil.which("date") or "/usr/bin/date"
VM_UUID = "0a1b2c3d-0000-4000-8000-0123456789ab"  # the fake VM's own identity (a synthetic value)

FAKE_WIN_VM = r'''#!/bin/bash
R=${FAKE_ROOT:?}
VMD=${WINVM_ROOT:?}/kfwin
S=$R/scn
mkdir -p "$VMD/logs" "$VMD/run"
rd() { cat "$S/$1" 2> /dev/null; }
alive() { local p; p=$(cat "$VMD/run/qemu.pid" 2> /dev/null); [ -n "$p" ] && [ -d "/proc/$p" ] && [ "$(awk '{print $3}' "/proc/$p/stat")" != Z ]; }
killq() {
    local p i
    p=$(cat "$VMD/run/qemu.pid" 2> /dev/null)
    [ -n "$p" ] || return 0
    kill -9 "$p" 2> /dev/null
    for i in $(seq 1 40); do [ -d "/proc/$p" ] && [ "$(awk '{print $3}' "/proc/$p/stat" 2> /dev/null)" != Z ] || return 0; /bin/sleep 0.05; done
}
clock_add() { echo $(( $(cat "$R/clock_offset" 2> /dev/null || echo 0) + $1 )) > "$R/clock_offset"; }
tdrline() { printf 'GATE_TDR since=a display4101=0 nvlddmkm153=%d kernelpower41=0 eventlog6008=0 bugcheck1001=0 livekernelevent_other=0 livekernelevent193=0 livekernelevent_stale=0 strict=0 total=%d' "$1" "$1"; }
gen_cli() { # W MODE: the three status files of a run, between the markers spec_guest.ps1 -Action collect prints
    local w=$1 mode=$2 st wl rc total=0 s cli="" end="" tdr="" name t
    local pre="[2026-11-07|10:00:00] [workload: $w / subtest:"
    case $mode in
        pass | reset | fail_subtest | no_tdr | scoreleak)
            cli="[2026-11-07|10:00:00] [Workload:$w] Starting"
            for s in 01_A 02_B 03_C; do
                cli+=$'\n'"$pre $s] Started"
                st=Completed
                [ "$mode" = fail_subtest ] && [ "$s" = 02_B ] && st="Failed -- check logs for errors"
                cli+=$'\n'"$pre $s] $st"
            done
            wl=Completed
            [ "$mode" != fail_subtest ] || wl="Failed -- check logs for errors"
            cli+=$'\n'"[2026-11-07|10:00:00] [Workload: $w] $wl"
            if [ "$mode" = scoreleak ]; then
                cli+=$'\n'"Composite score: 123.45 fps"$'\n'"Result C:\\spec\\results\\$w\\result_2026"$'\n'"Run complete -- results and logs available in:"
            fi
            rc=0
            [ "$mode" != fail_subtest ] || rc=1
            end="SPECRUN_END workload=$w rc=$rc start=a end=b secs=60"
            [ "$mode" != reset ] || total=7
            tdr="$(tdrline "$total")"$'\n'"GATE_STATE_DONE mode=tdr"
            if [ "$mode" = no_tdr ]; then end=""; tdr=""; fi ;;
        refused)
            cli="Error: Requirement: raytracing"$'\n'"  Requirement: sufficientVideoMemory"$'\n'"[Workload: $w] Failed -- check logs for errors"
            end="SPECRUN_END workload=$w rc=1 start=a end=b secs=2"
            tdr=$(tdrline 0) ;;
        gather)
            cli="Error: Unable to gather system details: gwpg_sysinfo.exe -s -d"
            end="SPECRUN_END workload=$w rc=1 start=a end=b secs=2"
            tdr=$(tdrline 0) ;;
        hang)
            cli="[Workload:$w] Starting"$'\n'"$pre 01_A] Started" ;;
    esac
    for name in cli.log END.txt tdr.txt; do
        echo "SPEC_COLLECT_BEGIN $name"
        case $name in cli.log) t=$cli ;; END.txt) t=$end ;; *) t=$tdr ;; esac
        [ -z "$t" ] || printf '%s\n' "$t"
        echo "SPEC_COLLECT_END $name"
    done
}
echo "win_vm $*" >> "$R/calls.log"
case "${1:-}" in
run)
    shift
    tag="" nokf3=0 overlay="" args=("$@")
    while [ $# -gt 0 ]; do
        case $1 in
            --tag) tag=$2; shift 2 ;;
            --no-kf3) nokf3=1; shift ;;
            --overlay) overlay=$2; shift 2 ;;
            --arm | --guest-driver | --kf3-extra | --env-file) shift 2 ;;
            *) shift ;;
        esac
    done
    echo "RUN tag=$tag nokf3=$nokf3 overlay=$overlay ram=${WINVM_RAM_MB:-unset} :: ${args[*]}" >> "$R/runs.log"
    kind=$(sed 's/.*_\(copy\|run\)\([0-9]*\)$/\1\2/' <<< "$tag")
    echo "$kind" > "$R/current_kind"
    rm -f "$R/nossh"
    f=$(rd "boot_fail.$kind")
    case "$f" in
        refused) echo "[t] REFUSED: the GPU lock was not free in 120s" > "$VMD/logs/${tag}_run.log"; exit 0 ;;
        bar1)
            echo "[t] kf3: realize refused: bar1-size 134217728 is not usable" > "$VMD/logs/${tag}_b1_qemu.log"
            echo "[t] WINVM_RUN_END tag=$tag boots=1 reason=" > "$VMD/logs/${tag}_run.log"
            exit 0 ;;
    esac
    /bin/sleep 3600 > /dev/null 2>&1 &
    echo $! > "$VMD/run/qemu.pid"
    echo "[t] QEMU_START tag=$tag boot=1 pid=$! log=x" > "$VMD/logs/${tag}_run.log"
    [ "$f" != nossh ] || touch "$R/nossh"
    exit 0 ;;
stop)
    killq
    [ -z "$(rd xid_new)" ] || echo 2 > "$R/xid_count"
    exit 0 ;;
scp-to)
    [ ! -e "$S/scp_fail" ] || exit 1
    echo "scp $2 -> $3" >> "$R/scp.log"
    exit 0 ;;
ssh)
    shift
    cmd="$*"
    alive || exit 255
    [ ! -e "$R/nossh" ] || exit 255
    case "$cmd" in
        *KF_SSH_UP*) echo KF_SSH_UP ;;
        *'tar -xf -'*)
            cat > "$R/guest_stream.tar"
            rc=$(rd tar_rc)
            exit "${rc:-0}" ;;
        *'New-Item'*) : ;;
        *'-Mode desktop'*) echo "GATE_DESKTOP user='KFWIN\\kf' dwm=2 explorer=1" ;;
        *'-Action prep'*) echo "SPEC_PREP exclusion=ok free_gb=250.0" ;;
        *'-Action verify'*)
            case "$(rd verify)" in
                empty) : ;;
                *) python3 "$R/count_tar.py" "$R/guest_stream.tar" "$(rd verify)" ;;
            esac
            echo SPEC_VERIFY_DONE ;;
        *'-Action post'*)
            if [ -e "$S/cli_absent" ]; then echo "SPEC_POST cli=absent"; else printf 'SPEC_POST cli=present\nSPEC_POST icacls_rc=0\nSPEC_POST vcredist_rc=0\n'; fi ;;
        *'-Action launch'*)
            w=$(sed 's/.*-Workload \([a-z_0-9-]*\).*/\1/' <<< "$cmd")
            echo "$w" > "$R/active"
            n=$(cat "$R/launches.$w" 2> /dev/null || echo 0); echo $((n + 1)) > "$R/launches.$w"
            mode=$(rd "viewset.$w"); mode=${mode:-pass}
            [ "$mode" != gather_once ] || { if [ "$n" = 0 ]; then mode=gather; else mode=pass; fi; }
            echo "$mode" > "$R/mode.$w"
            echo "launch $w mode=$mode" >> "$R/calls.log"
            if [ "$mode" = dead ]; then killq; exit 255; fi
            if [ "$mode" = launch_fail ]; then echo "SPEC_LAUNCH fail no-interactive-user"; exit 0; fi
            if [ "$mode" = launch_silent ]; then exit 255; fi
            echo "SPEC_LAUNCH ok workload=$w" ;;
        *'-Action status'*)
            w=$(sed 's/.*-Workload \([a-z_0-9-]*\).*/\1/' <<< "$cmd")
            mode=$(cat "$R/mode.$w" 2> /dev/null)
            if [ -e "$S/block.$w" ]; then touch "$R/blocked"; /bin/sleep 30; fi
            [ "$mode" != silent ] || exit 255
            [ "$mode" != hang ] || { echo "SPEC_STATE end=0 running=1"; exit 0; }
            polls=$(rd "polls.$w"); polls=${polls:-0}
            n=$(cat "$R/polls.$w.n" 2> /dev/null || echo 0)
            if [ "$n" -lt "$polls" ]; then echo $((n + 1)) > "$R/polls.$w.n"; echo "SPEC_STATE end=0 running=1"; exit 0; fi
            if [ ! -e "$R/walled.$w" ]; then touch "$R/walled.$w"; clock_add "$(rd "wall.$w" || echo 60)"; fi
            echo "SPEC_STATE end=1 running=0" ;;
        *'-Action collect'*)
            w=$(sed 's/.*-Workload \([a-z_0-9-]*\).*/\1/' <<< "$cmd")
            mode=$(cat "$R/mode.$w" 2> /dev/null); mode=${mode:-pass}
            gen_cli "$w" "$mode" ;;
        *'-Action kill'*) echo "kill $(cat "$R/active" 2> /dev/null)" >> "$R/calls.log"; echo "SPEC_KILL done"; rm -f "$R/walled.$(cat "$R/active" 2> /dev/null)" ;;
        *) echo "unexpected ssh: $cmd" >> "$R/calls.log"; exit 1 ;;
    esac
    exit 0 ;;
*) echo "unexpected win_vm: $*" >> "$R/calls.log"; exit 1 ;;
esac
'''

COUNT_TAR = r'''
import re, sys, tarfile
mode = sys.argv[2] if len(sys.argv) > 2 else ""
stats = {}
with tarfile.open(sys.argv[1], "r:") as t:
    for m in t:
        if not m.isfile():
            continue
        for part in m.name.split("/"):
            if re.match(r"^[a-z][a-z_]*-[0-9]{2}-[0-9]+\.[0-9]+\.[0-9]+$", part):
                f, b = stats.get(part, (0, 0))
                stats[part] = (f + 1, b + m.size)
                break
for d, (f, b) in sorted(stats.items()):
    if mode == "corrupt":
        b += 1
    print("VIEWSET %s files=%d bytes=%d" % (d, f, b))
'''

FAKE_FETCH = r'''#!/bin/bash
R=${FAKE_ROOT:?}
echo "fetch $*" >> "$R/fetch.log"
out=""
while [ $# -gt 0 ]; do case $1 in --out) out=$2; shift 2 ;; *) shift ;; esac; done
case "$(cat "$R/scn/fetch" 2> /dev/null)" in
    sha) echo "fetch_bench_data: sha256 mismatch"; exit 3 ;;
    notfound) echo "fetch_bench_data: Worker does not know object (404)"; exit 4 ;;
    net) echo "fetch_bench_data: giving up after 3 attempt(s)"; exit 1 ;;
    timeout) exit 124 ;;
    other) echo "fetch_bench_data: something else"; exit 7 ;;
    usage) echo "fetch_bench_data: KF_BROKER_URL missing"; exit 2 ;;
esac
mkdir -p "$(dirname "$out")"
cp "$R/archive.tar.zst" "$out"
echo "fetch_bench_data: -> $out (sha256 ok)"
'''

FAKES = {
    "date": '#!/bin/bash\nif [ "$#" = 1 ] && [ "$1" = "+%s" ]; then echo $(( $(' + REAL_DATE + ' +%s) + $(cat "$FAKE_ROOT/clock_offset" 2>/dev/null || echo 0) )); else exec ' + REAL_DATE + ' "$@"; fi\n',
    "sleep": '#!/bin/bash\necho "$1" >> "$FAKE_ROOT/sleeps.log"\necho $(( $(cat "$FAKE_ROOT/clock_offset" 2>/dev/null || echo 0) + ${1%.*} )) > "$FAKE_ROOT/clock_offset"\n',
    "pgrep": '#!/bin/bash\n[ "${1:-}" = "-x" ] && exit 1\nexec ' + (shutil.which("pgrep") or "/usr/bin/pgrep") + ' "$@"\n',
    "id": '#!/bin/bash\n[ "${1:-}" = "-u" ] && echo 0 || exec /usr/bin/id "$@"\n',
    "free": '#!/bin/bash\nprintf "              total\\nMem:             %s\\n" "$(cat "$FAKE_ROOT/scn/ram" 2>/dev/null || echo 64)"\n',
    "df": '#!/bin/bash\nprintf "Avail\\n  %sG\\n" "$(cat "$FAKE_ROOT/scn/disk" 2>/dev/null || echo 500)"\n',
    "dmesg": '#!/bin/bash\nn=$(cat "$FAKE_ROOT/xid_count" 2>/dev/null || echo 0)\nfor i in $(seq 1 "$n"); do echo "[1.0] NVRM: Xid (PCI:0000:01:00): 31, fake"; done\necho "[1.0] unrelated line"\n',
    "nvidia-smi": '#!/bin/bash\ncase "$*" in *"-L"*) echo "GPU 0: NVIDIA GeForce RTX 4090 (UUID: fake)";; *) echo "NVIDIA GeForce RTX 4090, 580.159.04, 24564 MiB, P8";; esac\n',
    "gcc": '#!/bin/bash\nout=""\nwhile [ $# -gt 0 ]; do case $1 in -o) out=$2; shift 2;; *) shift;; esac; done\n'
           'cat > "$out" <<\'EOS\'\n#!/bin/bash\nn=$(cat "$FAKE_ROOT/cup8_calls" 2>/dev/null || echo 0); echo $((n+1)) > "$FAKE_ROOT/cup8_calls"\n'
           'if [ "$n" != 0 ] && [ -e "$FAKE_ROOT/scn/cup8_fail_after" ]; then echo "CUP8 VERDICT: FAIL"; exit 1; fi\necho "CUP8 VERDICT: PASS"\nEOS\nchmod +x "$out"\n',
    "qemu-img": '#!/bin/bash\nback=""; last=""\nwhile [ $# -gt 0 ]; do case $1 in -b) back=$2; shift 2;; *) last=$1; shift;; esac; done\n'
                '[ -f "$back" ] || { echo "qemu-img: no backing file"; exit 1; }\necho "qemu-img create backing=$back" >> "$FAKE_ROOT/calls.log"\ntouch "$last"\n',
}


def make_archive(root, members=None, bad=None):
    """The fake private copy: a zstd-compressed tar with the nine viewset directories. `bad` adds one member that
    a tree of ours never holds ("symlink", "absolute", "dotdot")."""
    base = "Program Files/SPECviewperf 15"
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w", format=tarfile.GNU_FORMAT) as t:
        def add_dir(name):
            info = tarfile.TarInfo(name)
            info.type = tarfile.DIRTYPE
            info.mode = 0o755
            t.addfile(info)

        def add_file(name, data):
            info = tarfile.TarInfo(name)
            info.size = len(data)
            t.addfile(info, io.BytesIO(data))

        add_dir("Program Files")
        add_dir(base)
        add_file(base + "/SPECviewperf-CLI.exe", b"MZ" + b"cli" * 5)
        add_file(base + "/vcredist_x64.exe", b"MZvc")
        for k, vid in enumerate(members or IDS):
            d = "%s/viewsets/%s-1.0.%d" % (base, vid, k + 1)
            add_dir(d)
            add_file(d + "/data1.bin", bytes([k]) * (100 + k))
            add_file(d + "/sub/data2.bin", bytes([k + 1]) * (50 + 7 * k))
        if bad == "symlink":
            info = tarfile.TarInfo(base + "/link")
            info.type = tarfile.SYMTYPE
            info.linkname = "/etc/passwd"
            t.addfile(info)
        elif bad == "absolute":
            add_file("/etc/evil", b"x")
        elif bad == "dotdot":
            add_file(base + "/../evil", b"x")
    raw = buf.getvalue()
    out = Path(root) / "archive.tar.zst"
    out.write_bytes(subprocess.run(["zstd", "-q", "-c", "-3"], input=raw, capture_output=True, check=True).stdout)
    return hashlib.sha256(out.read_bytes()).hexdigest()


class Sandbox:
    """root/ holds the fake checkout (src/), the fake box state (winvm/, prov/, store/, bench/) and the fakes."""

    def __init__(self, root, tag="ci0123abcd", pin=True, gate_pass=True, bad=None):
        self.root = Path(root)
        self.tag = tag
        self.src = self.root / "src"
        self.prov = self.root / "prov"
        self.scn = self.root / "scn"
        self.fakebin = self.root / "fakebin"
        for d in (self.src, self.prov, self.scn, self.fakebin, self.root / "store", self.root / "bench", self.root / "home"):
            d.mkdir(parents=True, exist_ok=True)
        box = self.src / "scripts/bench/box"
        specd = self.src / "scripts/bench/specviewperf"
        win = self.src / "scripts/bench/windows"
        for d in (box, specd, win / "spec"):
            d.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(BENCH / "box/specviewperf_stage.sh", box / "specviewperf_stage.sh")
        for name in ("lane.py", "lanes.json", "tarfilter.py"):
            shutil.copyfile(BENCH / "specviewperf" / name, specd / name)
        for name in ("spec_guest.ps1", "spec_run.ps1"):
            shutil.copyfile(BENCH / "windows/spec" / name, win / "spec" / name)
        (win / "gate_state.ps1").write_text("# fake\n")
        (win / "kf3_flags.sample.env").write_text("KF3_FAKE=1\n")
        (win / "win_vm.sh").write_text(FAKE_WIN_VM)
        (box / "fetch_bench_data.sh").write_text(FAKE_FETCH)
        (self.src / "scripts/bench/cup8.c").write_text("/* fake */\n")
        (self.src / "scripts/bench/cuda_min").mkdir(exist_ok=True)
        for name, text in FAKES.items():
            f = self.fakebin / name
            f.write_text(text)
            f.chmod(0o755)
        (self.root / "count_tar.py").write_text(COUNT_TAR)
        self.sha = make_archive(self.root, bad=bad)
        if pin:
            (box / "specviewperf.pin").write_text("# test pin\nspecviewperf-15.0.0.tar.zst sha256=%s\n" % self.sha)
        vm = self.root / "winvm/kfwin"
        for d in ("disk", "run", "logs"):
            (vm / d).mkdir(parents=True, exist_ok=True)
        (vm / "installed").write_text("fake\n")
        (vm / "identity.env").write_text("VM_UUID=%s\n" % VM_UUID)
        (vm / "disk" / ("gate_" + tag + ".qcow2")).write_text("fake gate overlay\n")
        if gate_pass:
            (self.prov / (tag + "_windows.log")).write_text("WINDOWS_GATE_PASS rev=deadbeef gpu=x\nWINDOWS_GATE_EXIT rc=0 now\n")
        qemu = self.root / "bench/qemu"
        qemu.write_text("#!/bin/sh\n")
        qemu.chmod(0o755)
        (self.root / "flags.env").write_text("KF3_FAKE=1\n")
        self.vm = vm

    def set(self, name, value="1"):
        (self.scn / name).write_text(str(value))

    def env(self, lane="lanea", budget=40000, **extra):
        env = {
            "PATH": str(self.fakebin) + ":" + os.environ["PATH"],
            "HOME": str(self.root / "home"),
            "FAKE_ROOT": str(self.root),
            "WINVM_ROOT": str(self.root / "winvm"),
            "BENCH": str(self.root / "bench"),
            "WINVM_QEMU": str(self.root / "bench/qemu"),
            "KF_SPEC_STORE": str(self.root / "store"),
            "KF_SPEC_KVM_DEV": "/dev/null",
            "KF_SPEC_LANE": lane,
            "KF_SPEC_BUDGET_S": str(budget),
            "KF_WINGATE_FLAGS": str(self.root / "flags.env"),
        }
        env.update({k: str(v) for k, v in extra.items() if v is not None})
        return {k: v for k, v in env.items() if v is not None}

    def command(self, wrapper=False):
        if wrapper:
            return ["bash", str(HERE / "vast_stage_specviewperf.sh"), self.tag, str(self.prov)]
        return ["bash", str(self.src / "scripts/bench/box/specviewperf_stage.sh"), self.tag, str(self.prov)]

    def run(self, lane="lanea", budget=40000, wrapper=False, timeout=240, **extra):
        env = self.env(lane, budget, **extra)
        if wrapper:
            env["KF_SPECVIEWPERF_LANE"] = str(self.src / "scripts/bench/box/specviewperf_stage.sh")
            env["KF_CI_PRIVATE_DIR"] = str(self.root / "private")
        p = subprocess.run(self.command(wrapper), env=env, capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL)
        self.cleanup()
        return p

    def cleanup(self):
        pid = (self.vm / "run/qemu.pid")
        if pid.exists():
            try:
                os.kill(int(pid.read_text().strip()), 9)
            except (OSError, ValueError):
                pass

    def read(self, name):
        p = self.root / name
        return p.read_text() if p.exists() else ""

    def clock(self):
        return int(self.read("clock_offset") or 0)

    def runs(self):
        return [l for l in self.read("runs.log").splitlines() if l]

    def calls(self):
        return self.read("calls.log")


def rows_of(text):
    out = {}
    order = []
    for line in text.splitlines():
        if line.startswith("SPEC_ROW "):
            kv = dict(p.split("=", 1) for p in line.split()[1:] if "=" in p)
            out[kv["name"]] = kv
            order.append(kv["name"])
    return out, order


def verdict_of(text):
    lines = [l for l in text.splitlines() if l.startswith("SPECVIEWPERF_STAGE_") and not l.startswith("SPECVIEWPERF_STAGE_EXIT")]
    return lines[-1] if lines else ""
