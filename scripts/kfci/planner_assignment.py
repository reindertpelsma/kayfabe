"""Validate the assignment-reachable J2 schema snapshot, without planning or I/O.

This restricted interpreter supports exactly the schema keywords used by the
committed snapshot. It resolves only local definitions, never remote schemas.
Worker admission/pricing/coverage remain Worker responsibilities.
"""

import json
import math
from pathlib import Path
import re
from .assignment import AssignmentError

SCHEMA = json.loads(
    (Path(__file__).parent / "schemas/planner-assignment.schema.json").read_text()
)


def finite(value):
    try:
        return type(value) in (int, float) and math.isfinite(value)
    except OverflowError:
        return False


def same(left, right):
    if type(left) is bool or type(right) is bool:
        return type(left) is type(right) and left == right
    return left == right


def string_length(value):
    """Match the J2 JavaScript interpreter's UTF-16 code-unit bounds."""
    return len(value.encode("utf-16-le", errors="surrogatepass")) // 2


def check(schema, value, path="assignment"):
    if "$ref" in schema:
        reference = schema["$ref"]
        if not reference.startswith("#/$defs/"):
            raise AssignmentError("unsupported schema reference")
        return check(SCHEMA["$defs"][reference[8:]], value, path)
    if "const" in schema and not same(schema["const"], value):
        return path + " const"
    if "enum" in schema and not any(same(item, value) for item in schema["enum"]):
        return path + " enum"
    if "anyOf" in schema and not any(
        check(item, value, path) is None for item in schema["anyOf"]
    ):
        return path + " anyOf"
    if (
        "oneOf" in schema
        and sum(check(item, value, path) is None for item in schema["oneOf"]) != 1
    ):
        return path + " oneOf"
    kind = schema.get("type")
    correct_type = {
        "null": value is None,
        "string": type(value) is str,
        "boolean": type(value) is bool,
        "number": finite(value),
        "integer": finite(value)
        and int(value) == value
        and abs(value) <= 9007199254740991,
        "array": type(value) is list,
        "object": type(value) is dict,
    }
    if kind is not None and not correct_type.get(kind, False):
        return path + " type"
    if finite(value) and (
        value < schema.get("minimum", -math.inf)
        or value > schema.get("maximum", math.inf)
    ):
        return path + " range"
    if type(value) is str:
        if (
            not schema.get("minLength", 0)
            <= string_length(value)
            <= schema.get("maxLength", math.inf)
        ):
            return path + " string length"
        if "pattern" in schema:
            pattern = schema["pattern"]
            # J2 uses ECMAScript regexes without multiline or Unicode flags.
            # Python's $ otherwise admits a final newline, and \d is Unicode.
            if pattern.endswith("$"):
                pattern = pattern[:-1] + r"\Z"
            if re.search(pattern, value, re.ASCII) is None:
                return path + " pattern"
    if type(value) is list:
        if (
            not schema.get("minItems", 0)
            <= len(value)
            <= schema.get("maxItems", math.inf)
        ):
            return path + " array length"
        for index, item in enumerate(value):
            error = check(schema.get("items", {}), item, path + "[%d]" % index)
            if error:
                return error
    if type(value) is dict:
        if len(value) > schema.get("maxProperties", math.inf):
            return path + " properties"
        for name in schema.get("required", ()):
            if name not in value:
                return path + "." + name + " required"
        for name, item in value.items():
            child = schema.get("properties", {}).get(name)
            additional = schema.get("additionalProperties", True)
            if child is None and additional is False:
                return path + "." + name + " unknown"
            if child is not None or type(additional) is dict:
                error = check(
                    child if child is not None else additional, item, path + "." + name
                )
                if error:
                    return error
    return None


def json_bounds(value):
    pending = [(value, 0)]
    count = 0
    while pending:
        item, depth = pending.pop()
        count += 1
        if depth > 64 or count > 150000:
            raise AssignmentError("assignment JSON exceeds structural bounds")
        if item is None or type(item) is bool:
            continue
        if type(item) is str and string_length(item) <= 8192:
            continue
        if finite(item):
            continue
        if type(item) is dict and all(type(key) is str for key in item):
            pending.extend((child, depth + 1) for child in item.values())
        elif type(item) is list:
            pending.extend((child, depth + 1) for child in item)
        else:
            raise AssignmentError("assignment must be finite JSON data")


def validate_wire(value):
    json_bounds(value)
    error = check(SCHEMA, value)
    if error:
        raise AssignmentError(error)
    if value["status"] != "executable":
        raise AssignmentError("proposed assignment cannot execute")
    ids = [entry["id"] for entry in value["runs"]]
    if len(ids) != len(set(ids)):
        raise AssignmentError("duplicate step instance id")
    if not all(
        re.fullmatch(r"[A-Za-z0-9_.:-]{1,100}", name) for name in [value["id"], *ids]
    ):
        raise AssignmentError("invalid execution marker id")
    pins = {pin["id"]: pin for pin in value["driver_pins"]}
    if len(pins) != len(value["driver_pins"]):
        raise AssignmentError("duplicate driver pin")
    for pin in pins.values():
        if pin["status"] != "PINNED" or any(
            pin[key] is None for key in ("version", "url", "sha256", "size")
        ):
            raise AssignmentError("execution requires complete pinned drivers")
    for entry in value["runs"]:
        if any(pin not in pins for pin in entry["drivers"].values()):
            raise AssignmentError("step refers to an undeclared driver pin")
    unsupported_seed = (
        value["seed_hints"]
        or value["seed_exports"]
        or any(entry["optional_seed_for"] is not None for entry in value["runs"])
    )
    if unsupported_seed:
        raise AssignmentError("seed execution requires a migrated seed step")
    if value["worst_case_minutes"] > value["reservation_minutes"]:
        raise AssignmentError("worst case exceeds reservation")
    return value
