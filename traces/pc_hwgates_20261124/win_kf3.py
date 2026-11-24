#!/usr/bin/env python3
"""One kf3 boot of the sealed non-proprietary win11_base image on the pc (no GPU bind: the kf3 device reaches the host GPU through
unprivileged RM). Reuses the Vm helper of traces/vfio_s3_control_20261124/s3vfio.py, keeps the kf3 device in the argv, and records
the claims of PREREGISTRATION.md C4: the guest comes up, the adapter reads ConfigManagerErrorCode 0, nvidia-smi names the GPU, no
Display 4101 / nvlddmkm 153 / Kernel-Power 41 in a short idle. usage: win_kf3.py <qemu> <vmroot> <prep.qcow2> <out> <idle_s>"""
import json
import os
import re
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "vfio_s3_control_20261124"))
import s3vfio as S  # noqa: E402
PI = S.PI


class KVm(S.Vm):
    def argv(self):
        a = PI.build_argv(self.state, PI.PLAIN, self.disk, self.work, qemu=self.qemu, display="off", net="off")
        i = a.index("-cpu")
        a[i + 1] = self.cpu
        extra = os.environ.get("KF_DEV_EXTRA", "")
        if extra:
            i = next(n for n, v in enumerate(a) if v.startswith("kf3-gpu"))
            a[i] += "," + extra
        a += ["-pidfile", str(self.work / "qemu.pid")]
        assert any(x.startswith("kf3-gpu") for x in a), "kf3 device missing"
        return a


def main():
    qemu, vmroot, prep, out, idle = sys.argv[1], sys.argv[2], sys.argv[3], Path(sys.argv[4]), int(sys.argv[5])
    out.mkdir(parents=True, exist_ok=True)
    S.SP.OUT = out
    S.OUT = out
    base = Path(vmroot).parent / "win11_base" / "sealed" / "20261117T234916Z"
    state = json.loads((base / "state.json").read_text())
    name = "hwgates_kf3_%d" % int(time.time())
    vm = KVm(state, vmroot, name, Path(prep), qemu, S.ARM_CPU["B"], gpu=False)
    vm.setup()
    dm0 = S.sh("dmesg | wc -l")[1].strip()
    argv = vm.start()
    (out / "argv_scrubbed.txt").write_text(S.scrub(" ".join(argv)) + "\n")
    S.active("BOOT %s qemu=%s (kf3, no bind)" % (name, qemu))
    t0 = time.time()
    up = vm.wait_up(900)
    S.say("guest up=%s after %ds" % (up, time.time() - t0))
    res = {"up": up}
    if up:
        S.say("QEMU alive right after ssh answered: rc=%s" % vm.proc.poll())
        since = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(t0 - 5))
        ps = ('$g=Get-CimInstance Win32_VideoController | ? { $_.Name -like "*NVIDIA*" }; '
              '"ADAPTER " + $g.Name + " code=" + $g.ConfigManagerErrorCode + " status=" + $g.Status + " driver=" + $g.DriverVersion; '
              '"NVSMI " + ((& nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>&1) -join " | "); '
              '"NVSMI_RC " + $LASTEXITCODE')
        rc, o = vm.guest(ps, 120)
        (out / "guest_gpu.txt").write_text(S.scrub(o))
        S.say("GUEST_GPU " + S.scrub(o).replace("\n", " ; "))
        res["gpu_text"] = S.scrub(o)
        # the guest is polled every 5 s: when it stops answering is part of the evidence
        probe = []
        for _ in range(int(os.environ.get("KF_POLLS", "16"))):
            rc, o = vm.guest('"KF_UP " + (Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToString("o")', 15)
            probe.append("%ds rc=%s qemu_rc=%s %s" % (time.time() - t0, rc, vm.proc.poll(), o.strip()[:60]))
            if rc == 0:
                _, ev = vm.guest('Get-WinEvent -LogName System -MaxEvents 12 -EA 0 | % { $m = ($_.Message + "") -replace "\\s+"," "; '
                                 '"$($_.TimeCreated.ToString("HH:mm:ss")) $($_.Id) $($_.ProviderName) " + $m.Substring(0, [Math]::Min(110, $m.Length)) }', 40)
                probe.append(S.scrub(ev).rstrip())
            if vm.proc.poll() is not None:
                break
            time.sleep(5)
        (out / "poll.txt").write_text("\n".join(probe) + "\n")
        S.say("POLL last: " + probe[-1])
        if vm.proc.poll() is None:
            time.sleep(idle)
        res["events"] = S.guest_events(vm, since)
        (out / "guest_events.txt").write_text("\n".join(res["events"]) + "\n")
        S.say("EVENTS %d: %s" % (len(res["events"]), res["events"][:6]))
        rc, o = vm.guest(ps, 120)
        (out / "guest_gpu_after_idle.txt").write_text(S.scrub(o))
        res["gpu_after_idle"] = S.scrub(o)
    res["qemu_returncode_before_stop"] = vm.proc.poll()
    res["stop"] = vm.stop(clean=up)
    S.say("STOP " + res["stop"])
    d = S.sh("dmesg | tail -n +%d" % (int(dm0 or 0) + 1))[1].splitlines()
    keep = [ln for ln in d if re.search(r"NVRM|Xid|kvm|BUG:|Oops|hung|nvidia", ln, re.I)]
    (out / "dmesg_filtered.txt").write_text(S.scrub("\n".join(keep[-400:])) + "\n")
    qlog = (vm.work / "qemu.log").read_text(errors="replace")
    (out / "qemu_log_tail.txt").write_text(S.scrub(qlog[-30000:]))
    refs = [ln for ln in qlog.splitlines() if re.search(r"refus|REFUSED|denied|not permitted|NotPermitted|allowlist|RPC-REFUSED", ln, re.I)]
    (out / "qemu_refusal_lines.txt").write_text(S.scrub("\n".join(refs)) + "\n")
    res["refusal_lines"] = len(refs)
    (out / "result.json").write_text(json.dumps(res, indent=1))
    print("WIN_KF3_RESULT", json.dumps({k: v for k, v in res.items() if k not in ("events", "gpu_text", "gpu_after_idle")}))
    S.active("END BOOT %s up=%s" % (name, up))


main()
