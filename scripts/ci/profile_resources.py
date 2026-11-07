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
        raise ValueError('unresolved payload stage name')

    def visit(node, bindings):
        if isinstance(node, ast.For) and isinstance(node.target, ast.Name):
            try:
                items = ast.literal_eval(node.iter)
            except (ValueError, TypeError):
                items = None
            if isinstance(items, (list, tuple)) and all(isinstance(i, str) for i in items):
                for item in items:
                    for child in node.body + node.orelse:
                        visit(child, dict(bindings, **{node.target.id: item}))
                return
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == 'make_stage':
            if not node.args:
                raise ValueError('unnamed payload stage')
            names.add(value(node.args[0], bindings))
        for child in ast.iter_child_nodes(node):
            visit(child, bindings)

    visit(ast.parse(source), {})
    return names


def validate(data, source):
    errors = []
    profiles = data['payloadProfiles']
    stages = data['payloadStageResources']
    if set(profiles) != {'small', 'middle', 'large', 'adhoc'}:
        errors.append('profile names')
    try:
        missing = stage_names(source) - stages.keys()
        errors.extend('unbudgeted payload stage: ' + name for name in sorted(missing))
    except ValueError as error:
        errors.append(str(error))
    for name, stage in stages.items():
        if (stage.get('status') not in {'MEASURED', 'INFERRED'}
                or not isinstance(stage.get('source'), str)
                or not (ROOT / stage.get('source', '')).is_file()
                or not set(stage.get('profiles', [])) <= profiles.keys()):
            errors.append('stage provenance/schema: ' + name)
        for key in ['bandwidthDownGb', 'bandwidthUpGb', 'diskFloorGb']:
            if type(stage.get(key)) not in (int, float) or not math.isfinite(stage[key]) or stage[key] < 0:
                errors.append('stage allowance: ' + name)
    if errors:
        return errors
    for name, profile in profiles.items():
        active = [s for s in stages.values() if name in s['profiles']]
        for key in ['bandwidthDownGb', 'bandwidthUpGb']:
            if profile.get(key) != math.ceil(sum(s[key] for s in active)):
                errors.append('profile stage sum: ' + name + '/' + key)
        if profile.get('diskGb', 0) < max(s['diskFloorGb'] for s in active):
            errors.append('profile disk floor: ' + name)
        cap = profile.get('maxInetUsdPerGb')
        if type(cap) not in (int, float) or not math.isfinite(cap) or not 0 < cap <= 1:
            errors.append('profile per-GB cap: ' + name)
    return errors


if __name__ == '__main__':
    problems = validate(json.loads((ROOT / 'ci/vast-broker/defaults.json').read_text()),
                        (ROOT / 'scripts/ci/vast_payload.py').read_text())
    for problem in problems:
        print(problem)
    print('profile resources: ' + ('FAIL' if problems else 'PASS'))
    raise SystemExit(bool(problems))
