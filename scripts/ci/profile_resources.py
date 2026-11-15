#!/usr/bin/env python3
"""Fail closed when payload stages outgrow the profile resource table; GPU-free."""

import ast
import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def stage_names(source):
    names = set()

    def value(node, bindings):
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            return node.value
        if isinstance(node, ast.Name) and node.id in bindings:
            return bindings[node.id]
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
            return value(node.left, bindings) + value(node.right, bindings)
        raise ValueError("unresolved payload stage name")

    def visit(node, bindings):
        if isinstance(node, ast.For) and isinstance(node.target, ast.Name):
            try:
                items = ast.literal_eval(node.iter)
            except (ValueError, TypeError):
                items = None
            if isinstance(items, (list, tuple)) and all(
                isinstance(i, str) for i in items
            ):
                for item in items:
                    for child in node.body + node.orelse:
                        visit(child, dict(bindings, **{node.target.id: item}))
                return
        if (
            isinstance(node, ast.Call)
            and isinstance(node.func, ast.Name)
            and node.func.id == "make_stage"
        ):
            if not node.args:
                raise ValueError("unnamed payload stage")
            names.add(value(node.args[0], bindings))
        for child in ast.iter_child_nodes(node):
            visit(child, bindings)

    visit(ast.parse(source), {})
    return names


def manual_only(source):
    """{stage: flag name} from the payload's literal MANUAL_ONLY_STAGES, and the literal value of each flag (2026-11-15).
    A manual-only stage runs only when named by --only, so it carries no budget row and charges no profile; the moment its
    flag is True (it is in a default plan) it must leave this table and get a row, which is a Worker change."""
    tree = ast.parse(source)
    consts = {}
    for node in tree.body:
        if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
            try:
                consts[node.targets[0].id] = ast.literal_eval(node.value)
            except (ValueError, TypeError):
                pass
    if any(
        isinstance(n, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "MANUAL_ONLY_STAGES" for t in n.targets)
        for n in tree.body
    ) and "MANUAL_ONLY_STAGES" not in consts:
        raise ValueError("MANUAL_ONLY_STAGES must be a literal {stage: flag} dict")
    table = consts.get("MANUAL_ONLY_STAGES", {})
    if not isinstance(table, dict) or not all(isinstance(k, str) and isinstance(v, str) for k, v in table.items()):
        raise ValueError("MANUAL_ONLY_STAGES must be a literal {stage: flag} dict")
    return table, {flag: consts.get(flag) for flag in table.values()}


def profile_numbers_valid(profile):
    disk = profile["diskGb"]
    return (
        type(disk) in (int, float)
        and 0 < disk <= 10000
        and math.isfinite(disk)
        and disk == math.floor(disk)
        and all(
            type(profile[key]) in (int, float)
            and math.isfinite(profile[key])
            and 0 <= profile[key] <= 10000
            for key in ["bandwidthDownGb", "bandwidthUpGb"]
        )
    )


def validate(data, source):
    errors = []
    profiles = data.get("payloadProfiles")
    stages = data.get("payloadStageResources")
    if not isinstance(profiles, dict) or not isinstance(stages, dict):
        return ["resource tables must be objects"]
    spec = data.get("stageSlots", {}).get("specviewperf")
    if (
        not isinstance(spec, dict)
        or spec.get("lanes") != ["lanea", "laneb"]
        or spec.get("requiredRowsPerDie")
        != ["specviewperf_lanea", "specviewperf_laneb"]
        or spec.get("requiresSeparateRentals") is not True
    ):
        errors.append("SPEC lane/required-row contract")
    if set(profiles) != {"small", "middle", "large", "adhoc"}:
        errors.append("profile names")
    try:
        manual, flags = manual_only(source)
        missing = stage_names(source) - stages.keys() - manual.keys()
        errors.extend(
            "manual-only stage is in a default plan: " + name
            for name, flag in sorted(manual.items())
            if flags[flag] is not False
        )
        errors.extend("manual-only stage has a budget row: " + name for name in sorted(manual) if name in stages)
        errors.extend("unbudgeted payload stage: " + name for name in sorted(missing))
    except ValueError as error:
        errors.append(str(error))
    for name, stage in stages.items():
        if (
            stage.get("status") not in {"MEASURED", "INFERRED"}
            or not isinstance(stage.get("source"), str)
            or not (ROOT / stage.get("source", "")).is_file()
            or not set(stage.get("profiles", [])) <= profiles.keys()
        ):
            errors.append("stage provenance/schema: " + name)
        for key in ["bandwidthDownGb", "bandwidthUpGb", "diskFloorGb"]:
            if (
                type(stage.get(key)) not in (int, float)
                or not math.isfinite(stage[key])
                or stage[key] < 0
            ):
                errors.append("stage allowance: " + name)
    if errors:
        return errors
    for name, profile in profiles.items():
        keys = {"diskGb", "bandwidthDownGb", "bandwidthUpGb", "maxInetUsdPerGb"}
        if not isinstance(profile, dict) or set(profile) != keys:
            errors.append("profile keys: " + name)
            continue
        if not profile_numbers_valid(profile):
            errors.append("profile numeric fields: " + name)
            continue
        active = [s for s in stages.values() if name in s["profiles"]]
        if not active:
            errors.append("profile has no stages: " + name)
            continue
        for key in ["bandwidthDownGb", "bandwidthUpGb"]:
            if profile.get(key) != math.ceil(sum(s[key] for s in active)):
                errors.append("profile stage sum: " + name + "/" + key)
        if profile.get("diskGb", 0) < max(s["diskFloorGb"] for s in active):
            errors.append("profile disk floor: " + name)
        cap = profile.get("maxInetUsdPerGb")
        if type(cap) not in (int, float) or not math.isfinite(cap) or not 0 < cap <= 1:
            errors.append("profile per-GB cap: " + name)
    return errors


if __name__ == "__main__":
    problems = validate(
        json.loads((ROOT / "ci/vast-broker/defaults.json").read_text()),
        (ROOT / "scripts/ci/vast_payload.py").read_text(),
    )
    for problem in problems:
        print(problem)
    print("profile resources: " + ("FAIL" if problems else "PASS"))
    raise SystemExit(bool(problems))
