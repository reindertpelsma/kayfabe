"""Launch the prepared protocol only after the owned lease and both fixed timers are ready."""

from pathlib import Path
import datetime as dt
import json
import os
import subprocess
import sys

D = Path(__file__).resolve().parent
R = Path("/workspace/bw5-wt")
T = R / "traces/blackwell_ab_20261023/ROUND_B1q_bw5/attempt1"
K = T / "run_kit"
rows = json.loads((T / "owned_instances.json").read_text())
iid = int((D / "own-id").read_text())
row = next(r for r in rows if r["id"] == iid)
assert row.get("ssh_ready_utc") and not row["destroyed"]
assert ("OWN_STRETCH_READY " + str(iid)) in (D / "start_exception_stretch.log").read_text()
assert (D / "alias").read_text().strip() == "bw-fields-" + str(iid)
assert row["auto_destroy_minutes"] <= 150 and row["budget_cutoff_minutes"] <= 150  # B1f profile
assert "profile" not in row and row["lease_profile"] == "default"
for key in ["auto_destroy_pid", "budget_watch_pid"]:
    pid = row[key]
    assert os.getpgid(pid) == pid
    got = int(Path("/proc", str(pid), "stat").read_text().rpartition(")")[2].split()[19])
    assert got == row[key + "_starttime"]
start = dt.datetime.fromisoformat(row["created_utc"])
end = dt.datetime.fromisoformat(row["round_deadline_utc"])
assert dt.datetime.now(dt.timezone.utc) < end - dt.timedelta(minutes=25)
die = row["candidate_die"]
donor = die if die in ["GB202", "GB206"] else "GB202"
# BW4 B1n: the authority comes from the guard's pinned per-die table.
sys.path.insert(0, str(K))
sys.path.insert(0, "/workspace/bw5-wt/scripts/bench/blackwell")
import b1n_authority as BA  # noqa: E402
import saved_xid32_observation as SX  # noqa: E402

authority = BA.build(lease_id=iid, die=die, start=start, end=end, table=SX.DIE_PAIRS)
for path in [D / "observation_authority.json", T / "observation_authority.json"]:
    path.write_text(json.dumps(authority, indent=2) + "\n")
j = json.loads((K / "manifest.json").read_text())
j.update(
    actual_lease=iid,
    expected_die=die,
    expected_donor=donor,
    provider_exception_used=row.get("one_time_provider_exception", False),
    deadline_utc=end.isoformat(),
    attempt_state="launching_authenticated_lease",
    ssh_identity_guard_refusal=False,
    owner_authority="controller17:00 one lease OBSERVATION; no PASS/attestation",
    host_verification="UNVERIFIED-HOST",
    exact_channels=authority["exact_channels"],
    die_pairs={k: list(v) for k, v in SX.DIE_PAIRS.items()},
    owner_decisions="2026-10-24 BW4: per-die pairs; collection boot after fatal",
    fix_source="007ba1eb03db9e21d351e465e95b46cbc3134a95",
)
(K / "manifest.json").write_text(json.dumps(j, indent=2) + "\n")
assert not row.get("work_started") and not (D / "parallel_eight_plan.json").exists()
row["work_started"] = True
row["work_started_utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
tmp = T / "owned_instances.tmp"
tmp.write_text(json.dumps(rows, indent=2) + "\n")
tmp.replace(T / "owned_instances.json")
print("UPLOAD_STAGE lease", iid, "die", die, "donor", donor, flush=True)
subprocess.run([sys.executable, str(D / "upload.py")], check=True)
print("MEDIA_PIN_AND_LAUNCH_STAGE", flush=True)
subprocess.run([sys.executable, str(D / "parallel_eight.py")], check=True)
print("CONTROLLER_STARTED lease", iid, flush=True)
