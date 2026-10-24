"""Wire the BW4 round B1n operator workspace from the B1l workspace (GB205, default budget).

Substitution plus the named B1n edits only (owner 2026-10-24):
- paths: own worktree /workspace/bw4-wt, evidence ROUND_B1n_bw4/attempt1, new workspace;
- owned-id set gains the B1l/B1m leases; label claude-bw4-B1n1-OBSERVATION-20261024;
- offers: only dies with a pinned observation pair (GB205 RTX 5070, GB202 RTX 5090), and
  'deverified' hosts are excluded (owner: not 'deverified'); price, country, known-bad and the
  default 16-core profile unchanged; stop 2.00 / cap 2.50 / reserve 8 unchanged;
- authority per die from the guard's DIE_PAIRS (launch_ready via b1n_authority.build);
- bundle carries the B1n probes; exports/polls name the B1n arms; dry loop covers new suites.
Usage: operator_wire_b1n.py
"""

from pathlib import Path
import json
import shutil

SRC = Path("/workspace/bw3-round-b1l-a1_m30498-run")
D = Path("/workspace/bw4-round-b1n-a1-run")
OLD_W, W = "/workspace/bw-native-edge-wt", "/workspace/bw4-wt"
OLD_T, TNAME = "ROUND_B1l_0500/attempt1_m30498", "ROUND_B1n_bw4/attempt1"
T = Path(W) / "traces/blackwell_ab_20261023" / TNAME
ARMS = ["on_urt", "on_relay_off", "on_tdr60", "lx_reload", "on_diag_pnp"]
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
            ("        57453849,\n", "        57453849,\n        57457694,\n        57465679,\n"),
            (
                '    assert authority["exact_channels"] == ["0x14", "0x1000000b"]\n',
                "    import saved_xid32_observation as SX\n\n"
                '    assert authority["die_pairs"] == {k: list(v) for k, v in SX.DIE_PAIRS.items()}\n',
            ),
            (
                '        "RTX_5060_Ti": "GB206",\n        "RTX_5060": "GB206",\n'
                '        "RTX_5080": "GB203",\n        "RTX_5070_Ti": "GB203",\n',
                "",
            ),
            ('        "RTX_5050": "GB207",\n', ""),
            (
                'x.get("verification") in ("verified", "unverified", "deverified")',
                'x.get("verification") in ("verified", "unverified")  # BW4: not deverified',
            ),
            ("single_market_B1l_1_m30498.json", "single_market_B1n_1.json"),
            ('"codex-bw3-B1l1-OBSERVATION-20261023"', '"claude-bw4-B1n1-OBSERVATION-20261024"'),
            ("B1l attempt1 (total stop/cap)", "B1n attempt1 (owner 2026-10-24 per-die pairs)"),
        ],
    )
    launch = (D / "launch_ready.py").read_text()
    head, rest = launch.split("authority = {\n", 1)
    tail = rest[rest.index('for path in [D / "observation_authority.json"') :]
    launch = (
        head
        + (
            "# BW4 B1n: the authority comes from the guard's pinned per-die table.\n"
            "sys.path.insert(0, str(K))\n"
            'sys.path.insert(0, "/workspace/bw4-wt/scripts/bench/blackwell")\n'
            "import b1n_authority as BA  # noqa: E402\n"
            "import saved_xid32_observation as SX  # noqa: E402\n\n"
            "authority = BA.build(lease_id=iid, die=die, start=start, end=end, table=SX.DIE_PAIRS)\n"
        )
        + tail
    )
    (D / "launch_ready.py").write_text(launch)
    edit(
        "upload.py",
        [
            (
                '    "enable580_once.ps1",\n',
                '    "enable580_once.ps1",\n    "enable580_cycle.ps1",\n',
            ),
        ],
    )
    edit(
        "export_meta.py",
        [
            ("for arm in ['on_observe_b','on_observe_b1']:", "for arm in " + repr(ARMS) + ":"),
            (
                "'observed_b1_tests_summary.json']",
                "'observed_b1_tests_summary.json','probed_die.json','fatal_collect.log',"
                "'fatal_collect_receipt.json','prep_on_tdr60.log','prep_on_tdr60.txt']",
            ),
        ],
    )
    probe = (D / "milestone_probe.py").read_text()
    old = (
        '    "arm_on_observe_b.log",\n    "arm_on_observe_b1.log",\n'
        '    "collect_on_observe_b.log",\n    "collect_on_observe_b1.log",\n'
    )
    assert probe.count(old) == 1
    new = "".join(f'    "arm_{a}.log",\n    "collect_{a}.log",\n' for a in ARMS)
    new += '    "prep_on_tdr60.log",\n    "fatal_collect.log",\n'
    probe = probe.replace(old, new).replace(
        'for arm in ["on_observe_b", "on_observe_b1"]:', "for arm in " + json.dumps(ARMS) + ":"
    )
    (D / "milestone_probe.py").write_text(probe)
    poll = (D / "poll.py").read_text()
    old = (
        "    'arm_on_observe_b.log', 'collect_on_observe_b.log', 'on_observe_b_panic',\n"
        "    'arm_on_observe_b1.log', 'collect_on_observe_b1.log', 'on_observe_b1_panic',\n"
    )
    assert poll.count(old) == 1
    new = "".join(f"    'arm_{a}.log', 'collect_{a}.log', '{a}_panic',\n" for a in ARMS)
    (D / "poll.py").write_text(poll.replace(old, new + "    'fatal_collect.log',\n"))
    edit(
        "launch_ready.py",
        [
            (
                '    host_verification="UNVERIFIED-HOST",\n)\n',
                '    host_verification="UNVERIFIED-HOST",\n'
                '    exact_channels=authority["exact_channels"],\n'
                "    die_pairs={k: list(v) for k, v in SX.DIE_PAIRS.items()},\n"
                '    owner_decisions="2026-10-24 BW4: per-die pairs; collection boot after fatal",\n)\n',
            ),
        ],
    )
    rehearsal = W + "/traces/blackwell_ab_20261023/ROUND_B1n_bw4/rehearsal"
    edit(
        "dry_loop.py",
        [
            (
                '        "test_box_fetch_retry",\n    ),\n]\n',
                '        "test_box_fetch_retry",\n    ),\n'
                '    (BW, "test_die_pair_table"),  # BW4: per-die pinned pairs, diagnostic enable\n'
                f'    (Path("{rehearsal}"), "test_b1n_kit"),  # BW4: B1n plan, capture, fatal collect\n'
                f'    (Path("{rehearsal}"), "test_b1n_snippets"),  # BW4: box heredocs, live dummy\n'
                f'    (Path("{rehearsal}"), "test_b1n_ops"),  # BW4: offer scope, budget, label\n'
                "]\n",
            ),
            (
                "    return [BW, PR, PR_NATIVE, Path(run_kit), Path(workspace)]\n",
                f'    return [BW, PR, PR_NATIVE, Path(run_kit), Path(workspace), Path("{rehearsal}")]\n',
            ),
        ],
    )
    edit(
        "export_arm.py",
        [
            (
                "paths+=sorted(folder.glob('*_dmesg_*.txt'))\n",
                "paths+=sorted(folder.glob('*_dmesg_*.txt'))\n"
                "paths+=[root/('arm_'+run+'_prep_check.txt'),root/('prep_'+run+'.txt'),"
                "root/('prep_'+run+'.log'),root/('capture_guest_'+run+'.txt')]\n"
                "paths+=[folder/n for n in ['guest_staged.txt','binary_sha256.txt']]\n"
                "paths+=sorted(folder.glob('run_bw4_*_probe.log'))\n",
            ),
        ],
    )
    auth = json.loads((SRC / "owner_authority.json").read_text())
    auth.pop("exact_channels")
    auth.update(
        controller="owner 2026-10-24 BW4: B1n on GB205 (m30498 else next qualifying GB20x with a "
        "pinned pair), stop 2.00 / cap 2.50, fresh; per-die pairs; collection boot after fatal",
        die_pairs={"GB205": ["0x14", "0x1000000b"], "GB202": ["0x14", "0x0e00000b"]},
        attempt=1,
        plan_stop_usd=2.0,
        plan_cap_usd=2.5,
        prior_round_runtime_usd=0.0,
        arms=ARMS,
        deverified_admitted=False,
        profile="default (16-core floor); a < 16-core deviation needs its own label",
    )
    for key in (
        "cpu_profile_floor",
        "guest_smp",
        "build_jobs",
        "fuzz_cores",
        "lease_max_seconds",
        "inventory",
        "fuzz_rule",
        "provider_retry",
    ):
        auth.pop(key, None)
    for path in (D / "owner_authority.json", T / "owner_authority.json"):
        path.write_text(json.dumps(auth, indent=2) + "\n")
    shutil.copy2(SRC / "known_bad_machines.json", D / "known_bad_machines.json")
    shutil.copy2(T / "run_kit" / "capture_pull.py", D / "capture_pull.py")
    print("WIRED", D, TNAME)


if __name__ == "__main__":
    main()
