"""Executable SW4 adapter; all provider output is captured and projected explicitly."""

import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

import lifecycle

HERE = Path(__file__).resolve().parent
EVIDENCE = Path("/data/sw4_private_capture/lease1")
REPORT = Path("/root/codex_overnight_20261018/reports/SW4.md")


def utc():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def save(path, row):
    temporary = path.with_suffix(".tmp")
    with temporary.open("w") as stream:
        stream.write(json.dumps(row, indent=2) + "\n")
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


def api(args):
    result = subprocess.run(
        ["vastai", *args, "--raw"], capture_output=True, text=True, timeout=45, check=True
    )
    return json.loads(result.stdout)


def remote_script():
    return (HERE / "run_box.sh").read_text()


class Backend:
    def __init__(self, offer, plan, credit, *, evidence=EVIDENCE, report=REPORT, here=HERE):
        self.offer, self.plan, self.credit = offer, plan, credit
        self.here, self.report_path = here, report
        self.evidence = evidence
        self.evidence.mkdir(exist_ok=False)
        save(self.evidence / "owned.json", [])
        (self.here / "current").write_text(str(self.evidence))
        self.watch = None
        self.remote_pid = None

    def command(self, args, **kwargs):
        result = subprocess.run(args, capture_output=True, timeout=120, **kwargs)
        if result.returncode:
            save(
                self.evidence / "operation_failure.json",
                {
                    "utc": utc(),
                    "tool": args[0],
                    "returncode": result.returncode,
                    "stdout": (
                        result.stdout.decode(errors="replace")
                        if isinstance(result.stdout, bytes)
                        else result.stdout
                    ),
                    "stderr": (
                        result.stderr.decode(errors="replace")
                        if isinstance(result.stderr, bytes)
                        else result.stderr
                    ),
                },
            )
            raise ValueError("operation refused; original receipt in operation_failure.json")
        return result

    def ssh(self, command):
        return self.command(
            ["ssh", "-F", str(self.evidence / "ssh.conf"), "sw4-own", command], text=True
        ).stdout

    def arm(self, plan):
        save(self.evidence / "plan.json", plan)
        watch_output = (self.evidence / "watch.log").open("x")
        self.watch = subprocess.Popen(
            [
                sys.executable,
                str(self.here / "precreate_watchdog.py"),
                "--plan",
                str(self.evidence / "plan.json"),
                "--ledger",
                str(self.evidence / "owned.json"),
                "--ready",
                str(self.evidence / "watch_ready.json"),
                "--result",
                str(self.evidence / "watch_result.json"),
            ],
            stdin=subprocess.DEVNULL,
            stdout=watch_output,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        watch_output.close()
        for _ in range(100):
            if self.watch.poll() is not None:
                raise ValueError("watchdog died before create")
            if (self.evidence / "watch_ready.json").exists():
                ready = json.loads((self.evidence / "watch_ready.json").read_text())
                if ready["pid"] != self.watch.pid:
                    raise ValueError("watchdog PID mismatch")
                return ready
            time.sleep(0.1)
        raise ValueError("watchdog failed to arm")

    def create(self):
        if self.watch.poll() is not None:
            raise ValueError("watchdog absent before create")
        self.created = utc()
        self.label = "codex-sw4-" + self.created.replace(":", "").split(".")[0]
        reply = api(
            [
                "create",
                "instance",
                str(self.offer["id"]),
                "--image",
                "docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16",
                "--disk",
                "180",
                "--cancel-unavail",
                "--ssh",
                "--direct",
                "--label",
                self.label,
            ]
        )
        save(
            self.evidence / "create_safe.json",
            {key: reply.get(key) for key in ("success", "new_contract")},
        )
        if reply.get("success") is not True:
            raise ValueError("provider create refused; no retry")
        return reply.get("new_contract")

    def report(self, row):
        old = json.loads((self.evidence / "owned.json").read_text())
        record = old[0] if old else {}
        if row.get("id"):
            record.update(
                {
                    "created_utc": self.created,
                    "label": self.label,
                    "machine_id": self.offer["machine_id"],
                    "host_id": self.offer["host_id"],
                    "quote_dph": self.offer["dph_total"],
                    "credit_before": self.credit,
                    "watch_pid": row["watch"]["pid"],
                    "watch_starttime": row["watch"]["starttime"],
                    "destroyed": False,
                }
            )
            record.update(row)
            if row.get("api_absent"):
                record.update(destroyed=True, absence_verified_utc=utc())
            save(self.evidence / "owned.json", [record])
        with self.report_path.open("a") as stream:
            stream.write(
                "\nMEASURED (SW4 lifecycle receipt, "
                + utc()
                + "): `"
                + json.dumps(row, sort_keys=True)
                + "`.\n"
            )

    def ready(self, iid):
        result = subprocess.run(
            [sys.executable, str(self.here / "ready.py")],
            capture_output=True,
            text=True,
            timeout=1260,
        )
        (self.evidence / "readiness.log").write_text(result.stdout + result.stderr)
        if result.returncode:
            raise ValueError("settled SSH/provider guard refused; see readiness.log")

    def run(self, iid):
        preflight = self.ssh(
            "python3 - <<'PY'\n"
            "import os,shutil,json\n"
            "r={'free_bytes':shutil.disk_usage('/').free,'cpus':os.cpu_count(),"
            "'kvm':os.path.exists('/dev/kvm')}\n"
            "print(json.dumps(r))\n"
            "assert r['free_bytes']>=100*1024**3 and r['cpus']>=8 and r['kvm']\n"
            "PY"
        )
        (self.evidence / "resource_preflight.json").write_text(preflight)
        self.ssh("mkdir -p /root/sw4")
        files = [
            "run_box.sh",
            "instruments.tar",
            "guard_hooks.sh",
            "die_binding.py",
            "prepare_binding.py",
            "final_xids.py",
            "collect_capture.py",
            "derived.tar",
            "network_guard.py",
        ]
        record = json.loads((self.evidence / "owned.json").read_text())[0]
        save(
            self.here / "sshproof.json",
            {"ssh_verified": True, "identity": record["console_pin_proof"]},
        )
        files.append("sshproof.json")
        self.command(
            [
                "scp",
                "-F",
                str(self.evidence / "ssh.conf"),
                *[str(self.here / name) for name in files],
                "sw4-own:/root/sw4/",
            ]
        )
        sums = self.ssh("sha256sum " + " ".join("/root/sw4/" + name for name in files))
        actual = {Path(line.split()[1]).name: line.split()[0] for line in sums.splitlines()}
        expected = {
            name: hashlib.sha256((self.here / name).read_bytes()).hexdigest() for name in files
        }
        if actual != expected:
            raise ValueError("uploaded kit source hash mismatch")
        save(self.evidence / "uploaded_source_hashes.json", actual)
        self.ssh("tar -xf /root/sw4/derived.tar -C /root/sw4")
        started = self.ssh(
            "nohup setsid bash /root/sw4/run_box.sh </dev/null "
            ">/root/sw4/launch.log 2>&1 &\nprintf '%s\\n' \"$!\""
        )
        self.remote_pid = int(started.strip())
        record.update(work_started=True, controller_pid=self.remote_pid)
        save(self.evidence / "owned.json", [record])
        while dt.datetime.now(dt.timezone.utc) < dt.datetime.fromisoformat(
            self.plan["deadline_utc"]
        ):
            output = self.ssh(
                "if test -f /root/sw4/evidence/controller.rc; then "
                "printf 'RC='; cat /root/sw4/evidence/controller.rc; "
                "else kill -0 " + str(self.remote_pid) + " && echo RUNNING; fi\n"
                "tail -n 5 /root/sw4/evidence/controller.log"
            )
            (self.evidence / "latest_poll.txt").write_text(output)
            phases = [line for line in output.splitlines() if line.startswith("SW4_")]
            for line in phases:
                print(line, flush=True)
            if output.startswith("RC="):
                if output.splitlines()[0] != "RC=0":
                    raise ValueError("remote controller stopped; see original capture")
                return
            if self.watch.poll() is not None:
                raise ValueError("watchdog stopped the lease")
            time.sleep(20)
        raise ValueError("fixed deadline reached")

    def capture(self, iid):
        record = json.loads((self.evidence / "owned.json").read_text())[0]
        if record.get("work_started"):
            result = subprocess.run(
                [sys.executable, str(self.here / "pull.py")],
                capture_output=True,
                text=True,
                timeout=240,
            )
            if result.returncode:
                (self.evidence / "pull_failure.log").write_text(result.stdout + result.stderr)
                raise ValueError("private capture pull refused")
            receipt = json.loads((self.evidence / "capture_receipt.json").read_text())
            return (self.evidence / "capture.tgz").read_bytes(), receipt["sha256"]
        # Before any remote work, capture provider console privately; never print it.
        console = self.command(["vastai", "logs", str(iid), "--tail", "5000"])
        payload = console.stdout
        (self.evidence / "provider_console.txt").write_bytes(payload)
        digest = hashlib.sha256(payload).hexdigest()
        save(
            self.evidence / "capture_receipt.json",
            {"sha256": digest, "verified_utc": utc(), "kind": "prework provider console"},
        )
        return payload, digest

    def destroy(self, iid):
        self.command(["vastai", "destroy", "instance", str(iid), "-y"])

    def absent(self, iid):
        for _ in range(10):
            if not any(row["id"] == iid for row in api(["show", "instances"])):
                return True
            time.sleep(3)
        return False

    def cancel(self, watch):
        pid = watch["pid"]
        try:
            start = int(Path("/proc", str(pid), "stat").read_text().rpartition(")")[2].split()[19])
            if start != watch["starttime"] or os.getpgid(pid) != pid:
                raise ValueError("watchdog process identity changed")
            os.killpg(pid, signal.SIGTERM)
            self.watch.wait(timeout=10)
        except (FileNotFoundError, ProcessLookupError):
            pass


def rehearse(**options):
    from fake_lifecycle import rehearse_adapter

    return rehearse_adapter(**options)
