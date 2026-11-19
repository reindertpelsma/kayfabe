# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The box payload's instruction: one typed object, validated once at the boundary (stdlib only).

STATUS: LIVE, 2026-11-15 (OWNER_RULINGS section AR, stage S1). A requirement says what is wanted; the Worker
resolves it into an instruction; the payload takes only the instruction. Two sources feed ONE object:
a JSON file (this module, `--instruction FILE`) and the legacy KF_* environment (an adapter in
vast_payload.py that builds the same `Instruction`). The schema is ci/vast-broker/planner/schemas/instruction.schema.json,
mirrored byte for byte in scripts/kfci/schemas/; it is checked by the restricted interpreter of
scripts/kfci/planner_assignment.py (no remote refs, no I/O of its own).

The payload never reads the run token or the broker URL; the instruction file is the only thing it is told.
"""

import dataclasses
import hashlib
import json
from pathlib import Path
import sys

SCHEMA_ID = "kayfabe.payload-instruction/1"
MAX_BYTES = 65536
SCHEMA_PATH = Path(__file__).resolve().parents[1] / "kfci/schemas/instruction.schema.json"
# stage slots that can run (the others are reserved names, refused by the schema enum)
RUNNABLE_EXTRA_STAGES = ("specviewperf",)


class InstructionError(Exception):
    """The instruction is malformed or inconsistent. The message names a schema path, never a value."""


_LOADED = []


def _checker_and_schema():
    """The checker and the schema are loaded on first use: the KF_* path never needs them, so a box script or a
    test harness that copies only the files the environment path imports keeps working."""
    if not _LOADED:
        sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
        try:
            from kfci import planner_assignment
        finally:
            sys.path.pop(0)
        _LOADED.extend([planner_assignment, json.loads(SCHEMA_PATH.read_text())])
    return _LOADED[0], _LOADED[1]


def _object_pairs(pairs):
    out = {}
    for key, value in pairs:
        if key in out:
            raise InstructionError("duplicate key")
        out[key] = value
    return out


def _constant(name):
    raise InstructionError("non-finite number")


@dataclasses.dataclass(frozen=True)
class Artifact:
    name: str
    path: str
    sha256: str


@dataclasses.dataclass(frozen=True)
class Instruction:
    tier: str
    windows: bool
    windows_runs: int
    stages: tuple
    spec: object
    winapps: object
    linux_apps: object
    rehearsal: bool
    requirement_id: object = None
    tree_sha: object = None
    artifacts: tuple = ()
    # 2026-11-16: the images and applications of the generic proprietary-image runner (the objects of propimage_runspec.py); validated
    # by the schema here and by propimage_runspec.validate_list in vast_payload.py (named refusals)
    propimages: tuple = ()
    # strict (the Worker's shape): every listed image and app must run and any skip fails the stage; false = run what can run (docs/design/V3_PROPIMAGE_RUNNER.md)
    propimages_strict: bool = False
    # where it came from (worker / default / instruction); printed, never part of equality
    source: str = dataclasses.field(default="instruction", compare=False)

    def to_dict(self):
        out = {
            "schema": SCHEMA_ID,
            "tier": self.tier,
            "windows": self.windows,
            "windows_runs": self.windows_runs,
            "stages": list(self.stages),
            "spec": self.spec,
            "winapps": self.winapps,
            "linux_apps": self.linux_apps,
            "rehearsal": self.rehearsal,
        }
        if self.requirement_id is not None:
            out["requirement_id"] = self.requirement_id
        if self.tree_sha is not None:
            out["tree_sha"] = self.tree_sha
        if self.artifacts:
            out["artifacts"] = [dataclasses.asdict(a) for a in self.artifacts]
        if self.propimages:
            out["propimages"] = [dict(p) for p in self.propimages]
            out["propimages_strict"] = self.propimages_strict          # always written with the list: the emitted instruction shows the policy
        return out

    def canonical_bytes(self):
        return (json.dumps(self.to_dict(), indent=2, sort_keys=True) + "\n").encode()


def check_semantics(i):
    """Cross-field rules, the same ones the KF_* functions of vast_payload.py enforce on the environment."""
    if i.windows_runs == 2 and not i.windows:
        raise InstructionError("windows_runs needs windows")
    if i.winapps is not None and not i.windows:
        raise InstructionError("winapps needs windows")
    if i.rehearsal and i.tier != "large":
        raise InstructionError("rehearsal needs the large tier")
    if i.stages and i.tier != "large":
        raise InstructionError("stages need the large tier")
    if len(set(i.stages)) != len(i.stages) or any(s not in RUNNABLE_EXTRA_STAGES for s in i.stages):
        raise InstructionError("stages")
    wanted = "specviewperf" in i.stages
    if wanted and not i.windows:
        raise InstructionError("specviewperf needs windows")
    if wanted != (i.spec is not None):
        raise InstructionError("spec and stages disagree")
    if i.linux_apps == "middle" and i.tier not in ("middle", "large"):
        raise InstructionError("linux_apps middle needs middle or large")
    if i.linux_apps == "large" and i.tier != "large":
        raise InstructionError("linux_apps large needs large")
    # 2026-11-16 (owner ruling): a release rents two dies per family and the images are split across the two boxes, so the second
    # die's leg (tier middle) carries one image too
    if i.propimages and i.tier not in ("middle", "large"):
        raise InstructionError("propimages need the middle or large tier")
    if i.propimages_strict and not i.propimages:
        raise InstructionError("propimages_strict needs propimages")
    labels = [p.get("label") for p in i.propimages if p.get("label") is not None]
    if len(set(labels)) != len(labels):
        raise InstructionError("duplicate propimage label")
    names = set()
    for a in i.artifacts:
        if a.name in names:
            raise InstructionError("duplicate artifact name")
        names.add(a.name)
        parts = a.path.split("/")
        if a.path.startswith("/") or any(p in ("", ".", "..") for p in parts):
            raise InstructionError("artifact path")
    return i


def from_dict(doc):
    """Validate a decoded document against the schema and build the typed object. The only constructor
    for file input."""
    checker, schema = _checker_and_schema()
    try:
        checker.json_bounds(doc)
    except Exception:
        raise InstructionError("structural bounds") from None
    error = checker.check(schema, doc, "instruction", schema["$defs"])
    if error:
        raise InstructionError(error)
    return check_semantics(
        Instruction(
            tier=doc["tier"],
            windows=doc["windows"],
            windows_runs=int(doc["windows_runs"]),
            stages=tuple(doc["stages"]),
            spec=doc["spec"],
            winapps=doc["winapps"],
            linux_apps=doc["linux_apps"],
            rehearsal=doc["rehearsal"],
            requirement_id=doc.get("requirement_id"),
            tree_sha=doc.get("tree_sha"),
            artifacts=tuple(Artifact(**a) for a in doc.get("artifacts", [])),
            propimages=tuple(doc.get("propimages", [])),
            propimages_strict=doc.get("propimages_strict", False),
        )
    )


def parse(raw):
    """Bytes to Instruction. Bounded, strict UTF-8, no duplicate keys, no NaN/Infinity."""
    if len(raw) > MAX_BYTES:
        raise InstructionError("instruction too large")
    try:
        doc = json.loads(raw.decode("utf-8"), object_pairs_hook=_object_pairs, parse_constant=_constant)
    except (ValueError, UnicodeError, RecursionError):
        raise InstructionError("not JSON") from None
    return from_dict(doc)


def load(path):
    """(Instruction, sha256 of the exact bytes read). The file is read once; the hash is the provenance."""
    try:
        with open(path, "rb") as source:
            raw = source.read(MAX_BYTES + 1)
    except OSError:
        raise InstructionError("unreadable") from None
    return parse(raw), hashlib.sha256(raw).hexdigest()


def write(instruction, path):
    """Write the canonical form; returns its sha256."""
    raw = instruction.canonical_bytes()
    Path(path).write_bytes(raw)
    return hashlib.sha256(raw).hexdigest()
