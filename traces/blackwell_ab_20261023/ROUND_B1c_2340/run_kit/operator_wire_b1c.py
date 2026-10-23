"""Wire one B1c attempt workspace from the B1b operator by path/budget substitution only.

Usage: wire_b1c.py <attempt: 1|2> <prior_runtime_usd> <comma-separated extra owned ids>
Attempt 2 is the controller-granted single provider retry (23:40 grant) on a different
machine; its plan stop/cap are the round's 2.00/2.50 minus attempt 1's runtime estimate,
and the readiness check carries attempt 1's runtime as prior_lane_runtime_usd.
"""

from pathlib import Path
import json
import shutil
import sys

attempt = int(sys.argv[1])
prior = float(sys.argv[2])
extra_ids = [int(x) for x in sys.argv[3].split(",") if x]
assert attempt in (1, 2) and 0 <= prior < 1.0
SRC = Path("/workspace/bw3-round-b1b-run")
W = Path("/workspace/bw-native-edge-wt")
TB = W / "traces/blackwell_ab_20261023"
tname = "ROUND_B1c_2340" if attempt == 1 else "ROUND_B1c_2340_retry"
D = Path("/workspace/bw3-round-b1c-run" if attempt == 1 else "/workspace/bw3-round-b1c-retry-run")
T = TB / tname
K = T / "run_kit"
assert not D.exists() and not T.exists()
D.mkdir()
K.mkdir(parents=True)
stop, cap = round(2.0 - prior, 6), round(2.5 - prior, 6)


def sub(text):
    return text.replace("/workspace/bw3-round-b1b-run", str(D)).replace("ROUND_B1b_1815", tname)


for p in (TB / "ROUND_B1b_1815/run_kit").iterdir():
    if p.is_file():
        (K / p.name).write_text(sub(p.read_text()))
skip = {"rules_lock.json", "rehearsal_pass.json", "rehearsal.txt", "wire.py", "wire_b1c.py",
        "mutations.py", "mutations.json", "fixture_mutations.json", "preflight.txt"}
for p in SRC.iterdir():
    if p.is_file() and p.suffix in (".py", ".json", ".txt") and p.name not in skip:
        if p.name.startswith(("rent_", "ready_", "destroy_", "provider_failure_", "timer_ready_",
                              "precreate_watch", "round_plan", "own-id", "single_market",
                              "start_exception", "budget_watch_")):
            continue
        (D / p.name).write_text(sub(p.read_text()))
(D / "media").symlink_to(Path("/workspace/bw3-round-b1-run/media"), target_is_directory=True)
(D / "stretch.json").write_text(json.dumps({"host_verification": "UNVERIFIED-HOST",
                                            "attempts_used": 0, "provider_retries_used": 0}) + "\n")
(T / "owned_instances.json").write_text("[]\n")
ops = (D / "ops.py").read_text()
ids = "".join(f"        {i},\n" for i in extra_ids)
a = "        57388945,\n"
assert ops.count(a) == 1
ops = ops.replace(a, a + ids)
a = "stop_usd=2.0, cap_usd=2.5"
assert ops.count(a) == 3
ops = ops.replace(a, f"stop_usd={stop}, cap_usd={cap}")
a = "    spent = runtime_spent = 0.0\n"
assert ops.count(a) == 1
ops = ops.replace(a, f"    spent = runtime_spent = {prior}\n")
for old, new in [("single_market_B1b.json", f"single_market_B1c_{attempt}.json"),
                 ('"codex-bw3-B1b-OBSERVATION-20261023"',
                  f'"codex-bw3-B1c{attempt}-OBSERVATION-20261023"'),
                 ("B1b native benchmark verdicts are findings",
                  f"B1c attempt{attempt} (23:40 grant); native benchmark verdicts are findings")]:
    assert ops.count(old) == 1, old
    ops = ops.replace(old, new)
(D / "ops.py").write_text(ops)
auth = json.loads((SRC / "owner_authority.json").read_text())
auth.update(controller="23:40 B1c one-lease grant", attempt=attempt,
            provider_retry_allowed="one, different machine, same stop2.00/cap2.50",
            plan_stop_usd=stop, plan_cap_usd=cap, prior_round_runtime_usd=prior,
            deverified_admitted="owner ruling: unverified hosts allowed for sweeps")
for path in (D / "owner_authority.json", T / "owner_authority.json"):
    path.write_text(json.dumps(auth, indent=2) + "\n")
print("WIRED", attempt, D, tname, "stop", stop, "cap", cap, "extra_ids", extra_ids)
