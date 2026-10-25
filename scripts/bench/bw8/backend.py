"""BW8 adapter around the unchanged tested single-lease lifecycle and BW6 protocol."""

import datetime as dt
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from capture import verify_pulled
from execution import actual_rate


def source_lock(files):
    for name, expected in files.items():
        if hashlib.sha256(Path(name).read_bytes()).hexdigest() != expected:
            raise ValueError("execution source receipt changed")
    return True


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def backend_type(provider):
    class Backend(provider.Backend):
        def __init__(self, offer, plan, credit, config):
            self.config = config
            self.workspace = Path(config["workspace"])
            self.relative_watch = None
            super().__init__(
                offer,
                plan,
                credit,
                evidence=Path(config["evidence"]),
                report=Path(config["report"]),
                here=Path(config["operator"]),
            )

        def create(self):
            if self.watch.poll() is not None:
                raise ValueError("precreate watchdog absent")
            self.created = provider.utc()
            self.label = "codex-bw8-" + self.created.replace(":", "").split(".")[0]
            attempt = self.evidence / "create_attempt.json"
            with attempt.open("x") as stream:
                json.dump(
                    {"utc": self.created, "label": self.label, "offer_id": self.offer["id"]}, stream
                )
                stream.flush()
                os.fsync(stream.fileno())
            reply = provider.api(
                [
                    "create",
                    "instance",
                    str(self.offer["id"]),
                    "--image",
                    self.config["provider_image"],
                    "--disk",
                    "180",
                    "--cancel-unavail",
                    "--ssh",
                    "--direct",
                    "--label",
                    self.label,
                ]
            )
            provider.save(
                self.evidence / "create_safe.json",
                {k: reply.get(k) for k in ("success", "new_contract")},
            )
            if reply.get("success") is not True:
                raise ValueError("single create refused; no retry")
            return reply.get("new_contract")

        def report(self, receipt):
            ledger = self.evidence / "owned.json"
            old = json.loads(ledger.read_text())
            row = old[0] if old else {}
            if receipt.get("id"):
                row.update(receipt)
                row.update(
                    created_utc=self.created,
                    label=self.label,
                    machine_id=self.offer["machine_id"],
                    host_id=self.offer["host_id"],
                    quote_dph=self.offer["dph_total"],
                    credit_before=self.credit,
                )
                watch = receipt["watch"]
                row.update(
                    auto_destroy_verified=True,
                    auto_destroy_pid=watch["pid"],
                    auto_destroy_pid_starttime=watch["starttime"],
                    auto_destroy_pid_starttime_verified=True,
                    round_deadline_utc=self.plan["deadline_utc"],
                    budget_cutoff_minutes=self.plan["seconds"] / 60,
                    auto_destroy_minutes=self.plan["seconds"] / 60,
                    prior_lane_runtime_usd=0.0,
                    candidate_die="GB202",
                    experiment_mode="OBSERVATION",
                    attestation=False,
                    lease_profile="default",
                    destroyed=bool(receipt.get("api_absent")),
                )
                provider.save(ledger, [row])
                (self.workspace / "own-id").write_text(str(row["id"]) + "\n")
            projected = {
                k: receipt[k]
                for k in (
                    "id",
                    "deadline_utc",
                    "capture_verified",
                    "capture_sha256",
                    "api_absent",
                    "watch_cancelled_after_absence",
                )
                if k in receipt
            }
            projected.update(
                utc=provider.utc(),
                watchdog_pid=receipt["watch"]["pid"],
                requested_180gb_quote=self.offer["dph_total"],
                requested_disk_actual_rate=row.get("observed_dph_total", "UNRUN"),
            )
            with self.report_path.open("a") as stream:
                stream.write("\nMEASURED BW8 lifecycle: `" + json.dumps(projected) + "`.\n")
                stream.flush()
                os.fsync(stream.fileno())

        def ssh(self, command):
            alias = (self.workspace / "alias").read_text().strip()
            return self.command(
                ["ssh", "-F", str(self.workspace / "ssh.conf"), alias, command], text=True
            ).stdout

        def ready(self, iid):
            ledger = self.evidence / "owned.json"
            remaining = (
                dt.datetime.fromisoformat(self.plan["deadline_utc"])
                - dt.datetime.now(dt.timezone.utc)
            ).total_seconds()
            log = (self.workspace / "relative_watch.log").open("x")
            self.relative_watch = subprocess.Popen(
                [sys.executable, str(self.workspace / "budget_watch.py"), str(iid), str(remaining)],
                stdin=subprocess.DEVNULL,
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            log.close()
            marker = self.workspace / f"timer_ready_{iid}_{self.relative_watch.pid}.json"
            for _ in range(100):
                if self.relative_watch.poll() is not None:
                    raise ValueError("relative timer failed to arm")
                if marker.exists():
                    break
                time.sleep(0.1)
            if not marker.exists():
                raise ValueError("relative timer ready marker missing")
            proof = json.loads(marker.read_text())
            if (proof["pid"], proof["id"], proof["seconds"]) != (
                self.relative_watch.pid,
                iid,
                remaining,
            ) or os.getpgid(self.relative_watch.pid) != self.relative_watch.pid:
                raise ValueError("relative timer ready identity mismatch")
            row = json.loads(ledger.read_text())[0]
            start = int(
                Path("/proc", str(self.relative_watch.pid), "stat")
                .read_text()
                .rpartition(")")[2]
                .split()[19]
            )
            row.update(
                budget_watch_verified=True,
                budget_watch_pid=self.relative_watch.pid,
                budget_watch_pid_starttime=start,
            )
            provider.save(ledger, [row])
            self.command(["vastai", "attach", "ssh", str(iid), self.config["public_key"]])
            result = subprocess.run(
                [sys.executable, self.config["ready_program"], self.config["runtime_config"]],
                capture_output=True,
                text=True,
                timeout=min(1260, remaining),
            )
            (self.evidence / "readiness.log").write_text(result.stdout + result.stderr)
            if result.returncode:
                raise ValueError("unchanged settled identity/readiness refused; no retry")
            row = json.loads(ledger.read_text())[0]
            status = next(x for x in provider.api(["show", "instances"]) if x["id"] == iid)
            if (
                status.get("machine_id") != self.offer["machine_id"]
                or status.get("host_id") != self.offer["host_id"]
            ):
                raise ValueError("actual provider machine/host changed")
            rate = actual_rate(status["dph_total"], self.plan)
            row["observed_dph_total"] = rate
            provider.save(ledger, [row])
            self.report(
                dict(
                    id=iid,
                    watch=json.loads((self.evidence / "watch_ready.json").read_text()),
                    deadline_utc=self.plan["deadline_utc"],
                )
            )

        def run(self, iid):
            ledger = self.evidence / "owned.json"
            row = json.loads(ledger.read_text())[0]
            start = dt.datetime.fromisoformat(row["created_utc"])
            end = dt.datetime.fromisoformat(self.plan["deadline_utc"])
            sys.path[:0] = [self.config["kit"], self.config["bw"]]
            import b1n_authority as BA
            import saved_xid32_observation as SX

            authority = BA.build(
                lease_id=iid, die="GB202", start=start, end=end, table=SX.DIE_PAIRS
            )
            (self.workspace / "observation_authority.json").write_text(
                json.dumps(authority, indent=2) + "\n"
            )
            for name in ("upload.py", "parallel_eight.py"):
                result = subprocess.run(
                    [sys.executable, str(self.workspace / name)],
                    capture_output=True,
                    text=True,
                    timeout=min(
                        1260, max(1, (end - dt.datetime.now(dt.timezone.utc)).total_seconds() - 300)
                    ),
                )
                (self.evidence / (name + ".log")).write_text(result.stdout + result.stderr)
                if result.returncode:
                    raise ValueError(
                        "bounded upload/controller launch refused; private errors kept"
                    )
                if name == "upload.py":
                    row["work_started"] = True
                    provider.save(ledger, [row])
            counter = 0
            while dt.datetime.now(dt.timezone.utc) < end - dt.timedelta(seconds=300):
                if self.watch.poll() is not None:
                    raise ValueError("precreate watchdog ended the lease")
                result = self.command(
                    [
                        sys.executable,
                        str(self.workspace / "remote.py"),
                        str(self.workspace / "milestone_probe.py"),
                    ],
                    text=True,
                )
                state = json.loads(result.stdout)
                provider.save(self.evidence / f"poll_{counter:04d}.json", state)
                counter += 1
                safe = {
                    k: state.get(k)
                    for k in ("utc", "controller_alive", "fatal", "native_ready", "native_ack")
                }
                print(json.dumps(safe), flush=True)
                if state.get("native_ready") and not state.get("native_ack"):
                    result = self.command(
                        [sys.executable, str(self.workspace / "ack_native.py")], text=True
                    )
                    (self.evidence / "native_ack.log").write_text(result.stdout + result.stderr)
                if not state.get("controller_alive"):
                    return
                time.sleep(10)
            raise ValueError("fixed capture/teardown margin reached")

        def capture(self, iid):
            row = json.loads((self.evidence / "owned.json").read_text())[0]
            if not row.get("work_started"):
                return super().capture(iid)
            packed = self.ssh("bash " + self.config["remote_kit"] + "/pack_box.sh")
            (self.evidence / "pack_box.log").write_text(packed)
            log = (self.evidence / "capture_pull.log").open("x")
            child = subprocess.Popen(
                [
                    sys.executable,
                    str(self.workspace / "capture_pull.py"),
                    str(self.workspace),
                    str(self.evidence),
                    "ALL",
                ],
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            log.close()
            remaining = (
                dt.datetime.fromisoformat(self.plan["deadline_utc"])
                - dt.datetime.now(dt.timezone.utc)
            ).total_seconds()
            try:
                rc = child.wait(timeout=max(1, min(300, remaining - 60)))
            except BaseException:
                if child.poll() is None:
                    os.killpg(child.pid, 15)
                    child.wait(timeout=10)
                raise
            if rc:
                raise ValueError("inherited private capture pull failed")
            receipts = [
                json.loads(p.read_text()) for p in sorted(self.evidence.glob("capture_*.json"))
            ]
            verify_pulled(receipts, Path(self.config["capture_root"]) / str(iid))
            receipt = dict(
                verified_utc=provider.utc(),
                receipts=receipts,
                scope="original private box and any packed arm capture",
            )
            provider.save(self.evidence / "verified_capture.json", receipt)
            data = json.dumps(receipt, sort_keys=True).encode()
            return data, hashlib.sha256(data).hexdigest()

        def cancel(self, watch):
            super().cancel(watch)
            if self.relative_watch is not None:
                rows = json.loads((self.evidence / "owned.json").read_text())
                expected = rows[0]["budget_watch_pid_starttime"]
                pid = self.relative_watch.pid
                try:
                    actual = int(
                        Path("/proc", str(pid), "stat").read_text().rpartition(")")[2].split()[19]
                    )
                    if actual != expected or os.getpgid(pid) != pid:
                        raise ValueError("relative timer identity changed")
                    os.killpg(pid, 15)
                    self.relative_watch.wait(timeout=10)
                except (FileNotFoundError, ProcessLookupError):
                    pass

    return Backend
