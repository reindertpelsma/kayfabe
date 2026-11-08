"""Minimal local ASSIGNMENT v1 validation (stdlib only).

The local convenience format coexists with the J2 assignment schema snapshot.
Both are validated before the runner starts; only the Worker constructs plans.
"""

import json
import math
import re

IDENTIFIER = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_.-]{0,95}\Z")
MAX_BYTES = 1024 * 1024


class AssignmentError(ValueError):
    """Refuse an assignment before any step starts."""


def identifier(value):
    return isinstance(value, str) and IDENTIFIER.fullmatch(value) is not None


def number(value, *, positive=False):
    try:
        return (
            type(value) in (int, float)
            and math.isfinite(value)
            and (value > 0 if positive else value >= 0)
        )
    except OverflowError:
        return False


def validate(value):
    if not isinstance(value, dict):
        raise AssignmentError("assignment must be an object")
    if "runs" in value:
        from .planner_assignment import validate_wire

        return validate_wire(value)
    required = {"version", "id", "budget_seconds", "steps"}
    allowed = required | {"reserve_seconds", "inputs"}
    if not required <= value.keys() or value.keys() - allowed:
        raise AssignmentError("missing or unknown assignment field")
    if type(value["version"]) is not int or value["version"] != 1:
        raise AssignmentError("unsupported assignment version")
    if not identifier(value["id"]):
        raise AssignmentError("invalid assignment id")
    if not number(value["budget_seconds"], positive=True):
        raise AssignmentError("invalid budget_seconds")
    reserve = value.get("reserve_seconds", 0)
    if not number(reserve) or reserve >= value["budget_seconds"]:
        raise AssignmentError("invalid reserve_seconds")
    inputs = value.get("inputs", [])
    if (
        not isinstance(inputs, list)
        or not all(identifier(i) for i in inputs)
        or len(set(inputs)) != len(inputs)
    ):
        raise AssignmentError("invalid inputs")
    steps = value["steps"]
    if not isinstance(steps, list) or not 1 <= len(steps) <= 256:
        raise AssignmentError("steps must be a nonempty list of at most 256 entries")
    seen = set()
    for step in steps:
        if not isinstance(step, dict) or set(step) - {"id", "parameters"}:
            raise AssignmentError("invalid step entry")
        name = step.get("id")
        if not identifier(name) or name in seen:
            raise AssignmentError("invalid or duplicate step id")
        seen.add(name)
        if not isinstance(step.get("parameters", {}), dict):
            raise AssignmentError("step parameters must be an object")
    return value


def unique_object(pairs):
    result = {}
    for key, val in pairs:
        if key in result:
            raise AssignmentError("duplicate JSON key")
        result[key] = val
    return result


def load(source):
    """Read a path or text stream with a bounded input size; reject duplicate keys."""
    if hasattr(source, "read"):
        raw = source.read(MAX_BYTES + 1)
    else:
        with open(source, encoding="utf-8") as stream:
            raw = stream.read(MAX_BYTES + 1)
    if len(raw.encode("utf-8")) > MAX_BYTES:
        raise AssignmentError("assignment exceeds 1 MiB")
    try:
        return validate(json.loads(raw, object_pairs_hook=unique_object))
    except (ValueError, RecursionError) as error:
        raise AssignmentError("invalid assignment JSON: " + str(error)) from error
