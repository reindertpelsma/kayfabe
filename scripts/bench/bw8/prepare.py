"""Create an unexecuted observation overlay and fixed inventory; never contact a host."""

import argparse
import hashlib
import json
from pathlib import Path

SOURCE = "007ba1eb03db9e21d351e465e95b46cbc3134a95"
QEMU = "05d3d46f4e8429f5ffebacbd7ca9f5156f7d6f1141b4b10ce84732b174347448"
ARM = "b8a72bf4468179539ebfe397a5a5a7256010bf3793cdd2065df9d8c2d6826fa1"
BASELINE_PROCESS = "67605d7ca924330eb0eebdb5374b751789f3b160dd459a6286bcc9ad0cbd0292"
BASELINE_CONFIG = "01e9a12e735a754f1a5dd8b9ff59f44d5ddde810c194da081e27c6cbde6a3770"
BASELINE_COMMAND = "751f4bde752a75d4e1c6cde59df44e48228783e44c56981057e7bc81827e844f"
HERE = Path(__file__).resolve().parent


def inventory():
    return {
        "rental_authorized": False,
        "authority": "SEPARATE_CONTROLLER_GRANT_REQUIRED",
        "source": SOURCE,
        "qemu_sha256": QEMU,
        "baseline_arm_sha256": ARM,
        "baseline_process_sha256": BASELINE_PROCESS,
        "p19_sha256": "9e2207bc7f0fe1caa56b5fbc35cad57d51b063389dd50ffbb85851a50200732d",
        "baseline_config_sha256": BASELINE_CONFIG,
        "baseline_command_sha256": BASELINE_COMMAND,
        "route_parent": "b85d02bae3ed085a10cd1c8376c1a842e86c571b",
        "second_life_parent": "38fd948fd67d89ebf42827f56a3435355df592ae",
        "driver": "32.0.15.8088",
        "host_driver": "595.91.07",
        "image": "same BW6 image/media manifest; reverify before use",
        "image_digest": "UNVERIFIED: no retained whole-image digest",
        "media_sha256": {
            "Windows_ISO": "67cec5865eaa037a72ddc633a717a10a2bed50778862267223ddb9c60ef5da68",
            "Windows_580.88": "90c49f925b41ee062e02cc8094a16b6d057abfde4e85235a76d63ffe267f63a9",
            "virtio_0.1.302": "303f7ae40dad495d6ae474fdc571df58958a4dbc5c37a522d80f9a203867949d",
            "OpenSSH_10.0.0.0": "ddec9c53864280759cf9f74791cefd387100e3946aa849a1c138a4ed1b96b7d9",
        },
        "effective_flag_delta": {"KF3_COMPLETION_PROBE": "1000"},
        "baseline_flags": "exact BW6 p19.flags plus RPC_TRACE/BODY_TRACE=1, DIAG_PROGRESS=1000",
        "regression_seconds": 60,
        "acceptance_seconds": 600,
        "enable_count": 1,
        "applications": 0,
        "budget": {
            "stop_usd": 2.25,
            "cap_usd": 2.75,
            "max_hourly_usd": 1,
            "max_seconds": 7200,
            "reserve_usd": 8,
            "leases": 1,
        },
        "verification": "verified",
        "preferred_machine": 152098,
        "preferred_host": 702330,
        "denied_countries": ["CU", "IR", "KP", "SY", "CN", "HK", "MO"],
        "admission": "BW6 legacy; BW7 native-template-only",
        "guards": [
            "identity",
            "verified VM offer",
            "blacklist",
            "country",
            "precreate watchdog",
            "spend",
            "capture and hash before destroy",
            "exact owned id teardown",
            "API absence before timer cancellation",
        ],
        "retry_after_refusal": False,
        "falsifiers": {
            "H1-T": "Relevant host fence completes and expected guest release is present, "
            "yet that channel still times out.",
            "H1-G": "Complete correlated witness shows relevant host fence never completes "
            "before timeout.",
        },
        "correlation": "Relevant channel/fence/placement/release/armed event; no log is no proof",
        "probe_limits": "Translated slots do not cover every relayed twin; idle silence is valid",
        "operand_samples": "Up to16 bytes of completed operand DATA; PRIVATE, never metadata",
        "forbidden_modes": ["PB peek", "ring trace"],
        "inconclusive_or_survives": "UNRESOLVED",
    }


def validate_inventory(plan):
    # Preparation inputs are immutable and convey no execution or rental authority.
    return plan == inventory()


def verify_process_receipt(baseline, actual):
    try:
        if baseline["sha256"] != QEMU or actual["sha256"] != QEMU:
            return False
        for key in ("kf3_device", "harness_checkout"):
            if actual[key] != baseline[key]:
                return False
        expected = dict(baseline["flags"], KF3_COMPLETION_PROBE="1000")
        return actual["flags"] == expected and "KF3_COMPLETION_PROBE" not in baseline["flags"]
    except (KeyError, TypeError):
        return False


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError("missing/duplicate predecessor overlay anchor")
    return text.replace(old, new)


def arm_overlay(text):
    # Preserve every inherited phase/identity/Xid/cleanup/hold guard verbatim.
    text = replace_once(
        text,
        'python3 - "$profile" "$run" "$source" "$binary" <<\'PY\'',
        'python3 - "$profile" "$run" "$source" "$binary" "${BASH_SOURCE[0]}" <<\'PY\'',
    )
    text = replace_once(
        text,
        "'harness_sha256':",
        "'harness_sha256':hashlib.sha256(pathlib.Path(sys.argv[5]).read_bytes()).hexdigest(),"
        "'baseline_harness_sha256':",
    )
    text = replace_once(
        text,
        '"$enable_script" diag_min.ps1',
        '"$enable_script" bw8_enable_observe.ps1 diag_min.ps1',
    )
    text = replace_once(
        text,
        "export WINVM_QEMU=$binary",
        "export KF3_COMPLETION_PROBE=1000\nexport WINVM_QEMU=$binary",
    )
    bind = [
        line for line in text.splitlines(keepends=True) if 'bind_receipt.py "arm_${run}"' in line
    ]
    if len(bind) != 1:
        raise ValueError("missing/duplicate process receipt anchor")
    check = (
        'python3 -B "$(dirname "$ready_tool")/bw8_prepare.py" --verify-receipt '
        '"$(dirname "$ready_tool")/bw8_baseline_process.json" '
        '"$(dirname "$readiness")/arm_${run}_process_receipt.json"\n'
    )
    text = replace_once(text, bind[0], bind[0] + check)
    lines = text.splitlines(keepends=True)
    anchors = [line for line in lines if line.startswith('enable_cmd="')]
    if len(anchors) != 1 or "$enable_script" not in anchors[0]:
        raise ValueError("missing/duplicate enable command anchor")
    original = anchors[0]
    replacement = (
        'bw8_readiness=$(python3 -E -s "$ready_tool" value "$readiness" observer_total)\n'
        + original.replace(
            "$enable_script", "bw8_enable_observe.ps1 -ReadinessSeconds $bw8_readiness"
        )
    )
    text = replace_once(text, original, replacement)
    anchors = [
        line
        for line in text.splitlines(keepends=True)
        if line.startswith("cat ") and '_enable.txt"' in line
    ]
    if len(anchors) != 1:
        raise ValueError("missing/duplicate enable capture anchor")
    captured = anchors[0].split("cat ", 1)[1].strip()
    oracle = (
        'wait "$init_observer_pid"\n'
        'python3 -B "$(dirname "$ready_tool")/bw8_observer.py" '
        + captured
        + ' --readiness-seconds "$bw8_readiness"\n'
    )
    return replace_once(text, anchors[0], anchors[0] + oracle)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline_arm", type=Path, nargs="?")
    parser.add_argument("output", type=Path, nargs="?")
    parser.add_argument("--verify-receipt", type=Path, nargs=2)
    parser.add_argument("--baseline-process", type=Path)
    args = parser.parse_args()
    if args.verify_receipt:
        baseline, actual = [json.loads(p.read_text()) for p in args.verify_receipt]
        if not verify_process_receipt(baseline, actual):
            raise ValueError("effective source/configuration/flag delta mismatch")
        print("PROCESS_RECEIPT_MATCHED: sole flag delta KF3_COMPLETION_PROBE=1000")
        return
    if args.baseline_arm is None or args.output is None:
        parser.error("baseline arm and output required")
    data = args.baseline_arm.read_bytes()
    if hashlib.sha256(data).hexdigest() != ARM:
        raise ValueError("baseline arm hash mismatch")
    if args.baseline_process is None:
        parser.error("pinned baseline process receipt required")
    baseline = args.baseline_process.read_bytes()
    if hashlib.sha256(baseline).hexdigest() != BASELINE_PROCESS:
        raise ValueError("baseline process receipt hash mismatch")
    args.output.mkdir(exist_ok=False)
    files = {
        "arm_bw8.sh": arm_overlay(data.decode()),
        "bw8_enable_observe.ps1": (HERE / "enable_observe.ps1").read_text(),
        "bw8_observer.py": (HERE / "observer.py").read_text(),
        "bw8_prepare.py": Path(__file__).read_text(),
        "inventory.json": json.dumps(inventory(), indent=2) + "\n",
    }
    hashes = {}
    for name, text in files.items():
        (args.output / name).write_text(text)
        hashes[name] = hashlib.sha256(text.encode()).hexdigest()
    name = "bw8_baseline_process.json"
    (args.output / name).write_bytes(baseline)
    hashes[name] = BASELINE_PROCESS
    (args.output / "source_lock.json").write_text(json.dumps(hashes, indent=2) + "\n")
    print("PREPARED_ONLY: no commands executed; separate controller grant required")


if __name__ == "__main__":
    main()
