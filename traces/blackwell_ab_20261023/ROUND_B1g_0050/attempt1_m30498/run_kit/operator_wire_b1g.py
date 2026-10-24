"""Wire one B1g attempt workspace from the B1f profile workspace by substitution only.

Usage: wire_b1g.py <attempt 1..3> <prior_runtime_usd> <extra owned ids, comma-separated>
         <known_bad_source.json>
Round B1g (controller 00:50): up to 3 sequential attempts, each on a different machine; the
round stop 2.00 / cap 2.50 is TOTAL, so attempt n plans with stop/cap minus prior runtime.
"""

from pathlib import Path
import json
import shutil
import sys

attempt = int(sys.argv[1])
prior = float(sys.argv[2])
extra_ids = [int(x) for x in sys.argv[3].split(",") if x]
bad_source = Path(sys.argv[4])
tag = sys.argv[5] if len(sys.argv) > 5 else ""
assert tag in ("", "_m30498", "_m99805", "_next")
assert attempt in (1, 2, 3) and 0 <= prior < 1.5
SRC = Path("/workspace/bw3-round-b1f-run")
W = Path("/workspace/bw-native-edge-wt")
TB = W / "traces/blackwell_ab_20261023"
tname = f"ROUND_B1g_0050/attempt{attempt}{tag}"
D = Path(f"/workspace/bw3-round-b1g-a{attempt}{tag}-run")
T = TB / tname
K = T / "run_kit"
assert not D.exists() and not T.exists()
D.mkdir()
K.mkdir(parents=True)
stop, cap = round(2.0 - prior, 6), round(2.5 - prior, 6)


def sub(text):
    return text.replace(str(SRC), str(D)).replace("ROUND_B1f_0115", tname)


for p in (TB / "ROUND_B1f_0115/run_kit").iterdir():
    if p.is_file() and not p.name.startswith("operator_"):
        (K / p.name).write_text(sub(p.read_text()))
keep = (".py", ".json", ".txt")
skip_prefix = ("rent_", "ready_", "destroy_", "provider_failure_", "timer_ready_", "precreate_watch",
               "round_plan", "own-id", "single_market", "start_exception", "budget_watch_",
               "console_pin_", "rehearsal", "rules_lock", "preflight", "lease.json", "alias",
               "known_hosts", "ssh.conf", "deviation.json", "initial_inventory", "launch.log",
               "bundle", "upload_receipt", "media_cache", "probe_hashes", "parallel_eight_plan",
               "milestones", "diag_", "cpu_inventory", "observation_authority", "stretch")
for p in SRC.iterdir():
    if p.is_file() and p.suffix in keep and not p.name.startswith(skip_prefix):
        (D / p.name).write_text(sub(p.read_text()))
shutil.copy2(bad_source, D / "known_bad_machines.json")
(D / "media").symlink_to(Path("/workspace/bw3-round-b1-run/media"), target_is_directory=True)
(D / "stretch.json").write_text(json.dumps({"host_verification": "UNVERIFIED-HOST",
                                            "attempts_used": 0, "provider_retries_used": 0}) + "\n")
(T / "owned_instances.json").write_text("[]\n")
ops = (D / "ops.py").read_text()
a = "        57429076,\n"
assert ops.count(a) == 1
ops = ops.replace(a, a + "".join(f"        {i},\n" for i in extra_ids))
a = "stop_usd=2.0, cap_usd=2.5"
assert ops.count(a) == 3
ops = ops.replace(a, f"stop_usd={stop}, cap_usd={cap}")
a = "    spent = runtime_spent = 0\n" if "    spent = runtime_spent = 0\n" in ops else \
    "    spent = runtime_spent = 0.0\n"
assert ops.count(a) == 1
ops = ops.replace(a, f"    spent = runtime_spent = {prior}\n")
for old, new in [("single_market_B1f_1.json", f"single_market_B1g_{attempt}{tag}.json"),
                 ('"codex-bw3-B1f1-OBSERVATION-20261023"',
                  f'"codex-bw3-B1g{attempt}-OBSERVATION-20261023"'),
                 ("B1f attempt1; native", f"B1g attempt{attempt} (total stop/cap); native")]:
    assert ops.count(old) == 1, old
    ops = ops.replace(old, new)
shutil.copy2(Path("/workspace/bw3-round-b1g-common/pin_evidence.py"), D / "pin_evidence.py")
a = "import console_pin as C\n"
assert ops.count(a) == 1
ops = ops.replace(a, a + "import pin_evidence as PE  # B1g: export console host-key lines\n")
a = """        return subprocess.run(
            ["vastai", "logs", str(iid), "--tail", "5000"],
            capture_output=True,
            text=True,
            timeout=timeout,
        ).stdout
"""
assert ops.count(a) == 1
ops = ops.replace(a, """        text = subprocess.run(
            ["vastai", "logs", str(iid), "--tail", "5000"],
            capture_output=True,
            text=True,
            timeout=timeout,
        ).stdout
        PE.capture(D, iid, text)
        return text
""")
(D / "ops.py").write_text(ops)
auth = json.loads((SRC / "owner_authority.json").read_text())
auth.update(controller="00:50 B1g: up to 3 sequential attempts, different machines, total caps",
            attempt=attempt, plan_stop_usd=stop, plan_cap_usd=cap, prior_round_runtime_usd=prior)
for path in (D / "owner_authority.json", T / "owner_authority.json"):
    path.write_text(json.dumps(auth, indent=2) + "\n")
print("WIRED", attempt, D, tname, "stop", stop, "cap", cap, "extra_ids", extra_ids)
