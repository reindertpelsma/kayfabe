"""Wire the BW5 round B1q (attempt 2) operator workspace from the B1n workspace (GB205).

B1q = the B1p wiring below plus the named B1q edits (controller 12:10): fresh workspace, evidence
dir and empty ledger (ROUND_B1q_bw5/attempt1); label claude-bw5-B1q1; the readiness pin is the
settled guard (console_pin.settle_pin with a re-scan per check; known_hosts pins the settled key);
every pin receipt is also written into the evidence; the dry loop adds test_console_pin_settle and
test_b1q_ops; the B1p lease 57498790 joins the owned-id set.

B1p wiring (unchanged):

Substitution plus the named B1p edits only (task 2026-10-24 BW5; controller 11:50:
  one combined revision 007ba1eb0 = relay fix b85d02bae + second-life fix 38fd948fd):
- paths: own worktree /workspace/bw5-wt, evidence ROUND_B1p_bw5/attempt1, new workspace;
- owned-id set gains the BW4 leases (57472433, 57485318, 57486060);
  label claude-bw5-B1p1-OBSERVATION-20261024;
- offers: GeForce RTX 5070 (GB205) only, machine 30498 pinned first (grant_pin.json); price,
  country, known-bad, not-'deverified' and the default 16-core profile unchanged; stop 2.00 /
  cap 2.50 / reserve 8 unchanged;
- the bundle carries fix_pin.json (inside the kit) instead of the B1 commit object;
- exports/polls/audit name the B1p arms and the fix build; the dry loop runs the B1p suites
  (kit, snippets, ops) in place of the B1n ones.
Usage: operator_wire_b1p.py
"""

from pathlib import Path
import json
import shutil

SRC = Path("/workspace/bw4-round-b1n-a1-run")
D = Path("/workspace/bw5-round-b1q-a1-run")
OLD_W, W = "/workspace/bw4-wt", "/workspace/bw5-wt"
OLD_T, TNAME = "ROUND_B1n_bw4/attempt1", "ROUND_B1q_bw5/attempt1"
T = Path(W) / "traces/blackwell_ab_20261023" / TNAME
REH_N = W + "/traces/blackwell_ab_20261023/ROUND_B1n_bw4/rehearsal"
REH_P = W + "/traces/blackwell_ab_20261023/ROUND_B1p_bw5/rehearsal"
REH_Q = W + "/traces/blackwell_ab_20261023/ROUND_B1q_bw5/rehearsal"
FIX = "007ba1eb03db9e21d351e465e95b46cbc3134a95"
B1N_ARMS = ["on_urt", "on_relay_off", "on_tdr60", "lx_reload", "on_diag_pnp"]
ARMS = ["on_fix", "on_fix_pnp", "lx_reload"]
RESCAN = (
    "            def rescan():\n"
    "                out = subprocess.run(\n"
    '                    ["ssh-keyscan", "-T", "5", "-t", "ed25519", "-p", str(port), ip],\n'
    "                    capture_output=True,\n"
    "                    text=True,\n"
    "                    timeout=10,\n"
    "                ).stdout\n"
    '                rows = [l.split() for l in out.splitlines() if l and not l.startswith("#")]\n'
    '                keys = [f[2] for f in rows if len(f) >= 3 and f[1] == "ssh-ed25519"]\n'
    "                return keys[0] if len(keys) == 1 else None\n\n"
)
EXPORT_PIN = (
    '        evidence = L.parent / "lease_metadata" / ("console_pin_" + str(iid) + ".jsonl")\n'
    "        evidence.parent.mkdir(parents=True, exist_ok=True)\n"
    '        evidence.write_text((D / ("console_pin_" + str(iid) + ".jsonl")).read_text())\n'
)
DECISIONS = '"2026-10-24 BW4: per-die pairs; collection boot after fatal"'
SKIP = (
    "rent_",
    "ready_",
    "destroy_",
    "provider_failure_",
    "timer_ready_",
    "precreate_watch",
    "round_plan",
    "own-id",
    "single_market",
    "start_exception",
    "budget_watch_",
    "auto_watch_",
    "console_pin_",
    "console_raw_",
    "console_keylines_",
    "rehearsal",
    "rules_lock",
    "preflight",
    "lease.json",
    "alias",
    "known_hosts",
    "ssh.conf",
    "initial_inventory",
    "launch.log",
    "bundle",
    "upload_receipt",
    "media_cache",
    "probe_hashes",
    "parallel_eight_plan",
    "milestones",
    "diag_",
    "observation_authority",
    "stretch",
    "box_fetch_receipt",
    "final_remote_audit.out",
    "dry_loop_pass",
    "helpers_green",
    "wrapper_",
    "grant_pin",
    "b1_commit",
    "b1_merge",
    "create_",
)


def sub(text):
    return text.replace(str(SRC), str(D)).replace(OLD_W, W).replace(OLD_T, TNAME)


def edit(name, pairs):
    path = D / name
    text = path.read_text()
    for old, new in pairs:
        assert text.count(old) == 1, (name, old)
        text = text.replace(old, new)
    path.write_text(text)


def main():
    assert not D.exists()
    D.mkdir()
    for p in SRC.iterdir():
        if p.is_file() and p.suffix in (".py", ".json", ".txt") and not p.name.startswith(SKIP):
            (D / p.name).write_text(sub(p.read_text()))
    (D / "media").symlink_to(Path("/workspace/bw3-round-b1-run/media"), target_is_directory=True)
    (D / "stretch.json").write_text(
        json.dumps(
            {"host_verification": "UNVERIFIED-HOST", "attempts_used": 0, "provider_retries_used": 0}
        )
        + "\n"
    )
    (D / "grant_pin.json").write_text(json.dumps({"machine": 30498, "variant": "default"}) + "\n")
    edit(
        "ops.py",
        [
            (
                "        57465679,\n",
                "        57465679,\n        57472433,\n        57485318,\n"
                "        57486060,\n        57498790,\n",
            ),
            ('        "RTX_5090": "GB202",\n', ""),
            ("single_market_B1n_1.json", "single_market_B1q_1.json"),
            ('"claude-bw4-B1n1-OBSERVATION-20261024"', '"claude-bw5-B1q1-OBSERVATION-20261024"'),
            (
                "B1n attempt1 (owner 2026-10-24 per-die pairs)",
                "B1q attempt1 (BW5 attempt 2: fixes 007ba1eb0, settled pin)",
            ),
            (
                '        "measured_candidate": "48b027267a0e05c9ffdb4a049d34f4d719fd1837",\n',
                '        "measured_candidate": "48b027267a0e05c9ffdb4a049d34f4d719fd1837",\n'
                f'        "fix_candidate": "{FIX}",\n',
            ),
            # B1q: settled readiness pin (controller 12:10)
            (
                "                    proof = C.wait_for_pin(\n"
                "                        fields[2],\n"
                "                        console_text,\n",
                "                    proof = C.settle_pin(\n"
                "                        rescan,\n"
                "                        console_text,\n",
            ),
            (
                "            for fields in candidates:\n                if len(fields) < 3:\n",
                RESCAN
                + "            for fields in candidates:\n                if len(fields) < 3:\n",
            ),
            (
                '(D / "known_hosts").write_text(alias + " " + fields[1] + " " + fields[2] + "\\n")',
                '(D / "known_hosts").write_text(alias + " ssh-ed25519 " + proof["key"] + "\\n")',
            ),
            (
                '        print("CONSOLE_PIN", receipt["state"], receipt["polls"], flush=True)\n',
                '        print("CONSOLE_PIN", receipt["state"], receipt["polls"], flush=True)\n'
                + EXPORT_PIN,
            ),
        ],
    )
    edit(
        "upload.py",
        [
            ('files["sweep2/b1_commit.txt"] = D / "b1_commit.txt"\n', ""),
            ('files["sweep2/b1_merge.json"] = D / "b1_merge.json"\n', ""),
        ],
    )
    edit(
        "export_meta.py",
        [
            ("for arm in " + repr(B1N_ARMS) + ":", "for arm in " + repr(ARMS) + ":"),
            (
                "'prep_on_tdr60.log','prep_on_tdr60.txt']",
                "'prep_on_tdr60.log','prep_on_tdr60.txt','observed_fix_binary.json',"
                "'observed_fix_build.log']",
            ),
        ],
    )
    old = "".join(f'    "arm_{a}.log",\n    "collect_{a}.log",\n' for a in B1N_ARMS)
    new = "".join(f'    "arm_{a}.log",\n    "collect_{a}.log",\n' for a in ARMS)
    edit(
        "milestone_probe.py",
        [
            (old, new),
            ('    "observed_b1_build.log",\n', '    "observed_fix_build.log",\n'),
            ("for arm in " + json.dumps(B1N_ARMS) + ":", "for arm in " + json.dumps(ARMS) + ":"),
        ],
    )
    old = "".join(f"    'arm_{a}.log', 'collect_{a}.log', '{a}_panic',\n" for a in B1N_ARMS)
    new = "".join(f"    'arm_{a}.log', 'collect_{a}.log', '{a}_panic',\n" for a in ARMS)
    edit(
        "poll.py",
        [(old, new), ("'measured_build.log',", "'measured_build.log', 'observed_fix_build.log',")],
    )
    edit(
        "final_remote_audit.py",
        [
            (
                '    ("observed_b1", "/workspace/bw-observed-b1"),\n',
                '    ("observed_fix", "/workspace/bw-fix"),\n',
            ),
            (
                'if (r / "observed_b1_binary.json").exists():\n'
                '    assert sources["observed_b1"] == "cb80cbd29b7a05f904acd822f172485fd49938a9"\n',
                'if (r / "observed_fix_binary.json").exists():\n'
                f'    assert sources["observed_fix"] == "{FIX}"\n',
            ),
        ],
    )
    edit(
        "launch_ready.py",
        [
            (
                "    owner_decisions=" + DECISIONS + ",\n",
                "    owner_decisions=" + DECISIONS + ",\n" + f'    fix_source="{FIX}",\n',
            ),
        ],
    )
    edit(
        "dry_loop.py",
        [
            (
                f'        Path("{REH_N}"),\n        "test_b1n_kit",\n',
                f'        Path("{REH_P}"),\n        "test_b1p_kit",\n',
            ),
            (
                f'        Path("{REH_N}"),\n        "test_b1n_ops",\n',
                f'        Path("{REH_P}"),\n        "test_b1p_ops",\n',
            ),
            (
                f'        Path("{REH_N}"),\n        "test_b1n_snippets",\n',
                f'        Path("{REH_P}"),\n        "test_b1p_snippets",\n',
            ),
            (
                f'        Path("{REH_N}"),\n    ]\n',
                f'        Path("{REH_N}"),\n        Path("{REH_P}"),\n'
                f'        Path("{REH_Q}"),\n    ]\n',
            ),
            (
                '    (BW, "test_die_pair_table"),',
                '    (BW, "test_console_pin_settle"),  # B1q: settled readiness pin\n'
                f'    (Path("{REH_Q}"), "test_b1q_ops"),  # B1q: settled pin wiring, fresh paths\n'
                '    (BW, "test_die_pair_table"),',
            ),
        ],
    )
    auth = json.loads((SRC / "owner_authority.json").read_text())
    auth.update(
        controller="controller 12:10 BW5 grant B1q = attempt 2: validate fixes 007ba1eb0 on GB205 "
        "(m30498 else the next qualifying GeForce GB205), stop 2.00 / cap 2.50, fresh; settled pin",
        skus={"RTX_5070": "GB205"},
        arms=ARMS,
        fix_source=FIX,
        attempt=1,
        plan_stop_usd=2.0,
        plan_cap_usd=2.5,
        prior_round_runtime_usd=0.0,
        deverified_admitted=False,
        falsifier="H-FIX: Xid32 pair or any Xid, or a TDR/bugcheck within the 10-minute hold "
        "of on_fix; H-SL: more than one GSP-PUBLISH in the second life of on_fix_pnp, no "
        "Running, or not Code 0 on its second enable; refuted per fix on the actual die",
    )
    for path in (D / "owner_authority.json", T / "owner_authority.json"):
        path.write_text(json.dumps(auth, indent=2) + "\n")
    # The rent requires an existing, empty lease ledger (B1n's was made by hand; BW5 rent 1 stopped
    # on its absence before any create). Never reset a ledger that holds a lease.
    ledger = T / "owned_instances.json"
    if not ledger.exists():
        ledger.write_text("[]\n")
    assert json.loads(ledger.read_text()) == [], "the lease ledger is not empty; never re-wire"
    shutil.copy2(SRC / "known_bad_machines.json", D / "known_bad_machines.json")
    shutil.copy2(T / "run_kit" / "capture_pull.py", D / "capture_pull.py")
    print("WIRED", D, TNAME)


if __name__ == "__main__":
    main()
