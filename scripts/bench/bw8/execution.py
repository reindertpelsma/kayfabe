"""Reviewed one-lease admission, conservative planning and inherited operator wiring."""

import ast
import datetime as dt
import math
import re


def number(value):
    if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
        raise ValueError("invalid numeric receipt")
    return float(value)


def limits(grant):
    if grant in {"grant5", "grant6"}:
        return 1.4, 3.0, 3.5
    if grant == "":
        return 1.0, 2.0, 2.5
    raise ValueError("unknown lease grant")


def config_grant(config):
    grant = config.get("grant", "")
    limits(grant)
    if grant in {"grant5", "grant6"} and not any(
        name.endswith("/BW8_" + grant.upper() + ".txt") for name in config.get("grant_locks", {})
    ):
        raise ValueError("fresh grant5 source receipt required")
    return grant


def actual_rate(rate, plan):
    rate = number(rate)
    if not 0 < rate <= min(plan.get("rate_ceiling", 1.0), plan["rate_bound"]):
        raise ValueError("actual requested-disk rate exceeds reservation")
    return rate


def ready_budget(plan):
    from types import SimpleNamespace

    return SimpleNamespace(STOP_USD=plan["stop_usd"], MAX_RATE=plan.get("rate_ceiling", 1.0))


def require_offer(offer, blacklisted, *, grant=""):
    ceiling, _, _ = limits(grant)
    if offer.get("verification") != "verified":
        raise ValueError("verified offers only")
    if offer.get("vms_enabled") is not True or offer.get("is_vm_deverified") is not False:
        raise ValueError("verified VM capability required")
    if offer.get("gpu_name") != "RTX 5090" or offer.get("num_gpus") != 1:
        raise ValueError("one RTX 5090 required")
    country = re.search(r",\s*([A-Z]{2})$", str(offer.get("geolocation", "")))
    if not country or country[1] in {"CU", "IR", "KP", "SY", "CN", "HK", "MO"}:
        raise ValueError("country not admitted")
    if offer.get("machine_id") in blacklisted or offer.get("host_id") in blacklisted:
        raise ValueError("blacklist refusal")
    for field, floor in (
        ("disk_space", 180),
        ("cpu_cores_effective", 8),
        ("cpu_ram", 32768),
        ("reliability", 0.95),
        ("inet_down", 200),
    ):
        if number(offer.get(field)) < floor:
            raise ValueError("resource floor refusal")
    rate = number(offer["dph_base"]) + number(offer["storage_cost"]) * 180 / 720
    if not 0 < rate <= ceiling:
        raise ValueError("compute plus requested storage exceeds rate ceiling")
    number(offer["inet_down_cost"])
    number(offer["inet_up_cost"])
    return rate


def fit_plan(
    *, hourly, credit, now, transfer, deadline, estimated_seconds, shared_remaining, grant=""
):
    ceiling, stop, grant_cap = limits(grant)
    hourly, credit, transfer = number(hourly), number(credit), number(transfer)
    if not 0 < hourly <= ceiling or now.utcoffset() is None or deadline.utcoffset() is None:
        raise ValueError("rate or clock not admitted")
    cap = min(grant_cap, number(shared_remaining))
    if not stop < cap:
        raise ValueError("shared liability does not fit")
    bound = max(hourly * 1.15, hourly + 0.10)
    cost_seconds = int((stop - 0.50 - transfer) * 3600 / bound) - 60
    wall_seconds = int((deadline - now).total_seconds()) - 60
    seconds = min(6300, cost_seconds, wall_seconds)
    if seconds < estimated_seconds or seconds <= 0:
        raise ValueError("provisioning and existing observation/teardown margin do not fit")
    reservation = bound * seconds / 3600 + 0.50 + transfer
    if reservation >= stop or credit - reservation < 8:
        raise ValueError("stop or reserve refusal")
    return dict(
        armed_utc=now.isoformat(),
        deadline_utc=(now + dt.timedelta(seconds=seconds)).isoformat(),
        seconds=seconds,
        quoted_rate=hourly,
        rate_ceiling=ceiling,
        rate_bound=bound,
        credit_before=credit,
        reservation_usd=reservation,
        credit_after_reservation=credit - reservation,
        transfer_usd=transfer,
        nonruntime_margin_usd=0.50,
        stop_usd=stop,
        cap_usd=cap,
        reserve_usd=8.0,
        leases_authorized=1,
        hard_deadline_utc=deadline.isoformat(),
        estimated_seconds=estimated_seconds,
    )


def once(text, old, new):
    if text.count(old) != 1:
        raise ValueError("missing or duplicated inherited operator anchor")
    return text.replace(old, new)


def wire_arm(text):
    text = once(
        text,
        "bw8_enable_observe.ps1 diag_min.ps1",
        "bw8_enable_observe.ps1 bw8_parser.ps1 diag_min.ps1",
    )
    anchors = [line for line in text.splitlines(keepends=True) if line.startswith("enable_cmd=")]
    if len(anchors) != 1:
        raise ValueError("single enable command required")
    preflight = (
        'python3 -E -s "$ready_tool" run "$readiness" verify_stage -- bash "$W" ssh '
        "'powershell -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\bw8_parser.ps1' "
        '</dev/null >"$(dirname "$readiness")/arm_${run}_parser.txt" 2>&1\n'
        'grep -Eq "^BW8_PARSER_GREEN=1[[:space:]]*$" '
        '"$(dirname "$readiness")/arm_${run}_parser.txt"\n'
    )
    return once(text, anchors[0], preflight + anchors[0])


def wire_controller(text):
    text = once(text, '/arm.sh on "$run"', '/arm_bw8.sh on "$run"')
    lines = [
        line
        for line in text.splitlines(keepends=True)
        if re.search(r'bash [^\s]+/collect_bw.sh "\$run"', line)
    ]
    if len(lines) != 1:
        raise ValueError("single inherited capture command required")
    line = lines[0]
    replacement = line.replace("bash ", "python3 -I ", 1).replace(
        "/collect_bw.sh", "/capture_pack.py"
    )
    text = once(text, line, replacement)
    plans = [line for line in text.splitlines(keepends=True) if line.startswith("mapfile -t plan ")]
    if len(plans) != 1:
        raise ValueError("one inherited single-arm plan required")
    pin = (
        'test "$(sha256sum "${fix[0]}" | cut -d" " -f1)" = '
        "05d3d46f4e8429f5ffebacbd7ca9f5156f7d6f1141b4b10ce84732b174347448\n"
    )
    return once(text, plans[0], pin + plans[0])


def ready_body(text):
    tree = ast.parse(text)
    matches = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.If):
            continue
        test = node.test
        if (
            isinstance(test, ast.Compare)
            and isinstance(test.left, ast.Name)
            and test.left.id == "mode"
            and len(test.comparators) == 1
            and isinstance(test.comparators[0], ast.Constant)
            and test.comparators[0].value == "ready"
        ):
            matches.append(node)
    if len(matches) != 1:
        raise ValueError("one inherited readiness branch required")
    body = "\n".join(ast.get_source_segment(text, node) for node in matches[0].body)
    # Preserve every statement, including the original settled re-scan admission.
    if "C.settle_pin(" not in body:
        raise ValueError("settled identity admission absent")
    # Unparse only removes indentation; AST equality is checked by the caller's source lock.
    return ast.unparse(ast.Module(body=matches[0].body, type_ignores=[])) + "\n"


def arm_network(text, script, directory):
    # Paths are fixed protocol inputs from the private operator plan, never guest inputs.
    anchor = "trap cleanup_controller EXIT\n"
    launch = (
        f"mkdir -p {directory}\n"
        f'python3 -B {script} "$controller_pgid" '
        f">{directory}/network_watch.log 2>&1 &\n"
        "bw8_network_pid=$!\n"
        "for i in $(seq 1 50); do\n"
        f" [ -f {directory}/network_armed.json ] && break\n"
        ' kill -0 "$bw8_network_pid"\n sleep .1\ndone\n'
        f"[ -f {directory}/network_armed.json ]\n"
    )
    return once(text, anchor, anchor + launch)
