#!/usr/bin/env python3
"""One kf3 boot of the sealed non-proprietary win11_base image on the pc, with or without the kf3 display plane, polled for a fixed
time (STATUS: RESEARCH, 2026-11-25; adapted from traces/pc_displayexp_20261124/displayexp.py; evidence tool of traces/pc_rc1_20261125/, run by hand as root on the pc; not a gate).
No GPU bind: the kf3 device reaches the host GPU through unprivileged RM. Reuses the Vm helper of
traces/vfio_s3_control_20261124/s3vfio.py. The guest's and the host's output is data, only parsed.
usage: displayexp.py <arm Wdef|Wfill|Woff|Woffsup> <qemu> <vmroot> <prep.qcow2> <broker> <gop-efi> <out> <poll_s>"""
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "vfio_s3_control_20261124"))
import s3vfio as S  # noqa: E402
PI = S.PI

POLL_PS = r'''
$o = Get-CimInstance Win32_OperatingSystem
"UP " + $o.LastBootUpTime.ToString("o") + " NOW " + (Get-Date).ToString("o")
$ev = Get-WinEvent -FilterHashtable @{LogName="System"; StartTime=$o.LastBootUpTime.AddSeconds(-5)} -EA 0
foreach ($i in 153,141,41,1074,6008,1001,4101) { "EV $i " + @($ev | ? { $_.Id -eq $i -and $_.ProviderName -eq "nvlddmkm" -or ($_.Id -eq $i -and $i -ne 153) }).Count }
"EV 153KernelBoot " + @($ev | ? { $_.Id -eq 153 -and $_.ProviderName -ne "nvlddmkm" }).Count
"EV nvlddmkm_any " + @($ev | ? { $_.ProviderName -eq "nvlddmkm" }).Count
$ev | ? { $_.Id -in 153,141,41,1074,6008,1001,4101,14 } | sort TimeCreated | select -Last 12 | % { $m = ($_.Message + "") -replace "\s+"," "; "EVL $($_.TimeCreated.ToString("HH:mm:ss")) $($_.Id) $($_.ProviderName) " + $m.Substring(0, [Math]::Min(110, $m.Length)) }
'''
PNP_PS = r'''
$g = Get-CimInstance Win32_VideoController | ? { $_.Name -like "*NVIDIA*" }
"ADAPTER " + $g.Name + " code=" + $g.ConfigManagerErrorCode + " status=" + $g.Status
Get-PnpDevice -Class Display -EA 0 | % { "PNP " + $_.FriendlyName + " status=" + $_.Status + " problem=" + (Get-PnpDeviceProperty -InstanceId $_.InstanceId -KeyName DEVPKEY_Device_ProblemCode -EA 0).Data }
"NVSMI " + ((& nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>&1) -join " | ")
'''


class DVm(S.Vm):
    def __init__(self, *a, arm, broker, gop, **kw):
        super().__init__(*a, **kw)
        self.arm, self.broker_bin, self.gop = arm, broker, gop

    def argv(self):
        kw = {}
        display = "off"
        if self.arm in ("Wdef", "Wfill"):
            display = "broker"
            kw = {"broker_sock": str(self.work / "broker.sock"), "gop_efi": str(self.gop), "uid": 0}
        a = PI.build_argv(self.state, PI.PLAIN, self.disk, self.work, qemu=self.qemu, display=display, net="off", **kw)
        i = a.index("-cpu")
        a[i + 1] = self.cpu
        extra = {"Wfill": ",unmapped-reads-zero=on", "Woffsup": ",windows-support=off"}.get(self.arm, "")
        if extra:
            i = next(n for n, v in enumerate(a) if v.startswith("kf3-gpu"))
            a[i] += extra
        a += ["-qmp", "unix:%s,server=on,wait=off" % (self.work / "qmp-ev.sock"), "-pidfile", str(self.work / "qemu.pid")]
        assert any(x.startswith("kf3-gpu") for x in a), "kf3 device missing"
        assert (self.arm in ("Woff", "Woffsup")) == (not any("display=on" in x for x in a)), "display flag does not match the arm"
        return a


def start_broker(vm):
    sock = vm.work / "broker.sock"
    fifo = vm.work / "broker.fifo"
    os.mkfifo(fifo)
    fd = os.open(fifo, os.O_RDWR)
    log = open(vm.work / "broker.log", "w")
    bp = subprocess.Popen([vm.broker_bin, "--socket", str(sock), "--backend", "test", "--persist", "--verbose", "--size", "1920x1080"],
                          stdin=fd, stdout=log, stderr=subprocess.STDOUT)
    for _ in range(50):
        if sock.exists() or bp.poll() is not None:
            break
        time.sleep(0.2)
    if not sock.exists():
        raise RuntimeError("display broker did not become ready")
    return bp, fd


def main():
    arm, qemu, vmroot, prep, broker, gop, out, poll_s = sys.argv[1:9]
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    poll_s = int(poll_s)
    S.SP.OUT = out
    S.OUT = out
    base = Path(vmroot).parent / "win11_base" / "sealed" / "20261117T234916Z"
    state = json.loads((base / "state.json").read_text())
    name = "dispexp_%s_%d" % (arm, int(time.time()))
    vm = DVm(state, vmroot, name, Path(prep), qemu, S.ARM_CPU["B"], gpu=False, arm=arm, broker=broker, gop=gop)
    vm.setup()
    bp = fd = rec = None
    up0 = float(open("/proc/uptime").read().split()[0])   # dmesg is cut by its monotonic stamp: the ring buffer can wrap, so a line offset is not safe
    try:
        if arm in ("Wdef", "Wfill"):
            bp, fd = start_broker(vm)
        argv = vm.start()
        (out / "argv_scrubbed.txt").write_text(S.scrub(" ".join(argv)) + "\n")
        S.active("BOOT %s arm=%s qemu=%s (kf3, no bind, display experiment)" % (name, arm, qemu))
        t0 = time.time()
        rec = subprocess.Popen([sys.executable, str(HERE / "qmp_events.py"), str(vm.work / "qmp-ev.sock"), str(out / "qmp_events.jsonl")])
        poll, last_ev, up_at, died_at = [], "", None, None
        n = 0
        while time.time() - t0 < poll_s:
            if vm.proc.poll() is not None:
                died_at = time.time() - t0
                break
            ts = int(time.time() - t0)
            rc, o = vm.guest(POLL_PS, 30)
            o = S.scrub(o)
            if rc == 0 and "UP " in o:
                if up_at is None:
                    up_at = ts
                cnt = " ".join(l[3:].replace(" ", "=") for l in o.splitlines() if l.startswith("EV "))
                up = re.search(r"UP (\S+)", o).group(1)
                line = "%ds rc=0 qemu_rc=None boot=%s ev(id=count) %s" % (ts, up, cnt)
                last_ev = o
                if n % 6 == 0:
                    rc2, p = vm.guest(PNP_PS, 30)
                    line += " | " + S.scrub(p).strip().replace("\n", " ; ")
            else:
                line = "%ds rc=%s qemu_rc=%s (no answer)" % (ts, rc, vm.proc.poll())
            n += 1
            poll.append(line)
            S.say(line[:200])
            time.sleep(max(0, 10 - (time.time() - t0 - ts)))
        if died_at is None and vm.proc.poll() is not None:
            died_at = time.time() - t0
        (out / "poll.txt").write_text("\n".join(poll) + "\n")
        (out / "events_last_successful_poll.txt").write_text(last_ev)
        res = {"arm": arm, "up_at_s": up_at, "qemu_exit_at_s": died_at, "qemu_rc": vm.proc.poll(),
               "alive_at_end": vm.proc.poll() is None, "poll_s": poll_s}
        if vm.proc.poll() is None:
            _, p = vm.guest(PNP_PS, 30)
            (out / "pnp_at_end.txt").write_text(S.scrub(p))
        try:
            vm.qmp("quit", tries=2)
        except Exception:
            pass
        try:
            vm.proc.wait(30)
        except subprocess.TimeoutExpired:
            vm.proc.kill()
            vm.proc.wait(30)
    finally:
        for p in (rec, bp):
            if p is not None and p.poll() is None:
                p.terminate()
        if fd is not None:
            os.close(fd)
    time.sleep(1)
    up1 = float(open("/proc/uptime").read().split()[0])
    res["host_uptime_window_s"] = [round(up0, 1), round(up1, 1)]
    d = [ln for ln in S.sh("dmesg")[1].splitlines() if re.match(r"\[\s*(\d+\.\d+)\]", ln) and up0 - 2 <= float(re.match(r"\[\s*(\d+\.\d+)\]", ln).group(1)) <= up1 + 2]
    keep = [ln for ln in d if re.search(r"NVRM|Xid|BUG:|Oops|hung|nvidia", ln, re.I)]
    (out / "dmesg_filtered.txt").write_text(S.scrub("\n".join(keep[-400:])) + "\n")
    res["host_mmuWalkMap_lines"] = sum(1 for ln in d if "mmuWalkMap" in ln)
    res["host_no_memory_lines"] = sum(1 for ln in d if "NV_ERR_NO_MEMORY" in ln)
    qlog = (vm.work / "qemu.log").read_text(errors="replace")
    (out / "qemu_log_tail.txt").write_text(S.scrub(qlog[-30000:]))
    qf = [ln for ln in qlog.splitlines() if re.search(r"NV_ERR_NO_MEMORY|status=0x51|mmuWalk|VA_ALREADY|HELD|refus|REFUSED|RPC-REFUSED|through our reservation|unmapped-reads-zero", ln)]
    res["log_counts"] = {k: len(re.findall(k, qlog)) for k in ("through our reservation", "HELD BY HOST", "HELD-BY-HOST", "HELD-BY-OURSELVES",
                         "WINDOWS GUEST REFUSED", "kf3: unmapped-reads-zero=auto", "kf3: unmapped-reads-zero=on", "kf3: unmapped-reads-zero=off")}
    res["unmapped_reads_zero_lines"] = sorted(set(ln.strip()[:120] for ln in qlog.splitlines() if "kf3: unmapped-reads-zero=" in ln))
    (out / "qemu_f1_refusal_lines.txt").write_text(S.scrub("\n".join(qf)) + "\n")
    res["qemu_log_0x51_lines"] = sum(1 for ln in qf if "status=0x51" in ln or "NO_MEMORY" in ln)
    res["qemu_log_refusal_lines"] = len(qf)
    evs = []
    try:
        evs = [json.loads(x) for x in (out / "qmp_events.jsonl").read_text().splitlines() if x.strip()]
    except Exception:
        pass
    res["qmp_events"] = [e for e in evs if "event" in e]
    (out / "result.json").write_text(json.dumps(res, indent=1))
    print("DISPEXP_RESULT", json.dumps({k: v for k, v in res.items() if k != "qmp_events"}), [e["event"] for e in res["qmp_events"]])
    S.active("END BOOT %s arm=%s alive_at_end=%s" % (name, arm, res["alive_at_end"]))


main()
