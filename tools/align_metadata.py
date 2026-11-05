#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Bounded evidence-only API alignment; never compares or authors reply bodies.

Usage: align_metadata.py NATIVE_OR_PRETTY.tsv VIRTUAL_OR_PRETTY.tsv OUTPUT.tsv
Wire input is kf-census sequence TSV. Pretty input has ordinal, function,
control_or_class and effective_result. Every command appears in the output,
including unmatched calls, frees and updates. Matching API keys infer only an
order-preserving correspondence, not identical handles, bodies or semantics.
Reply association is function FIFO, checked against successful reply keys.
Repeated keys remain explicitly ambiguous; separate status layers are retained.
"""
import collections
import csv
from dataclasses import dataclass
import difflib
from pathlib import Path
import sys

MAX_BYTES = 16 * 1024 * 1024
MAX_ROWS = 20000
MAX_COMMANDS = 10000
MAX_ALIGNMENT_CELLS = 16 * 1024 * 1024


def number(value):
    text = str(value).strip()
    result = int(text, 16 if text.lower().startswith("0x") else 10)
    if not 0 <= result <= 0xffffffff:
        raise ValueError("metadata number is outside u32")
    return result


def optional(value):
    return None if value in (None, "", "-", "none", "unknown") else number(value)


def hx(value, missing="-"):
    return missing if value is None else hex(value)


@dataclass
class Command:
    ordinal: int
    function: int
    selector: int | None
    envelope: int | None
    control_status: int | None
    effective: int | None
    reply_ordinal: int | None = None
    pairing: str = "pretty trace; body unavailable"

    @property
    def key(self):
        return self.function, self.selector


def read_commands(path):
    if path.stat().st_size > MAX_BYTES:
        raise ValueError("metadata input exceeds byte bound")
    with path.open(encoding="utf-8", newline="") as f:
        reader = csv.DictReader(f, delimiter="\t")
        fields = set(reader.fieldnames or ())
        if not {"function", "control_or_class"} <= fields:
            raise ValueError("missing API metadata columns")
        rows = []
        for row in reader:
            if len(rows) >= MAX_ROWS:
                raise ValueError("metadata row bound exceeded")
            rows.append(row)
    wire = "dir" in fields
    required = {"element_sequence", "rpc_result", "control_status"} if wire else {"ordinal", "effective_result"}
    if not required <= fields:
        raise ValueError("missing sequence/status metadata columns")
    pending = collections.defaultdict(collections.deque)
    if wire:
        for row in rows:
            fn = number(row["function"])
            if row["dir"] not in ("cmd", "status"):
                raise ValueError("unknown metadata direction")
            if row["dir"] == "status" and fn < 0x1000:
                pending[fn].append(row)
    commands = []
    for row in rows:
        if wire and row["dir"] != "cmd":
            continue
        if len(commands) >= MAX_COMMANDS:
            raise ValueError("metadata command bound exceeded")
        ordinal = number(row["element_sequence"] if wire else row["ordinal"])
        if commands and ordinal <= commands[-1].ordinal:
            raise ValueError("command order must be strictly increasing")
        fn, selector = number(row["function"]), optional(row["control_or_class"])
        if not wire:
            commands.append(Command(ordinal, fn, selector, optional(row.get("trace_result")),
                                    None, optional(row["effective_result"])))
            continue
        reply = pending[fn].popleft() if pending[fn] else None
        if reply is None:
            commands.append(Command(ordinal, fn, selector, None, None, None,
                                    pairing="missing reply; status unknown"))
            continue
        envelope, control = optional(reply["rpc_result"]), optional(reply["control_status"])
        reply_key = optional(reply["control_or_class"])
        if envelope == 0 and reply_key != selector:
            raise ValueError("successful reply key conflict; FIFO association refused")
        effective = None if envelope is None else (envelope or control or 0)
        basis = "function FIFO; keys agree; handles/bodies unavailable"
        if reply_key != selector:
            basis = "function FIFO; error reply key erased; association inferred"
        commands.append(Command(ordinal, fn, selector, envelope, control, effective,
                                number(reply["element_sequence"]), basis))
    return commands, sum(len(q) for q in pending.values())


FIELDS = ["relation", "native_command_sequence", "native_status_sequence",
          "function", "control_or_class", "native_effective_result", "virtual_ordinal",
          "virtual_effective_result", "effective_status_equal", "matching_block_length",
          "native_envelope", "native_control_status", "virtual_envelope",
          "virtual_control_status", "api_key_repeated", "native_pair_basis",
          "virtual_pair_basis", "next_native_function", "next_native_control_or_class"]


def align(native, virtual):
    # Refuse excessive work rather than silently dropping a prefix or suffix.
    if len(native) * len(virtual) > MAX_ALIGNMENT_CELLS:
        raise ValueError("alignment work bound exceeded; split explicit comparable phases")
    na, va = collections.Counter(c.key for c in native), collections.Counter(c.key for c in virtual)
    matcher = difflib.SequenceMatcher(a=[c.key for c in native],
                                     b=[c.key for c in virtual], autojunk=False)

    def row(i, j, relation, block=0):
        a, b = native[i] if i is not None else None, virtual[j] if j is not None else None
        c = a or b
        equal = "unknown" if a is None or b is None or a.effective is None or b.effective is None \
                else str(int(a.effective == b.effective))
        following = native[i + 1] if a is not None and i + 1 < len(native) else None
        return [relation, a.ordinal if a else "-", a.reply_ordinal if a and a.reply_ordinal is not None else "-",
                hex(c.function), hx(c.selector), hx(a.effective, "unknown") if a else "-",
                b.ordinal if b else "-", hx(b.effective, "unknown") if b else "-", equal, block,
                hx(a.envelope) if a else "-", hx(a.control_status) if a else "-",
                hx(b.envelope) if b else "-", hx(b.control_status) if b else "-",
                int(na[c.key] > 1 or va[c.key] > 1), a.pairing if a else "-", b.pairing if b else "-",
                hex(following.function) if following else "-", hx(following.selector) if following else "-"]

    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag == "equal":
            for i, j in zip(range(i1, i2), range(j1, j2)):
                yield row(i, j, "match", i2 - i1)
        else:
            for i in range(i1, i2):
                yield row(i, None, "native_only")
            for j in range(j1, j2):
                yield row(None, j, "virtual_only")


def main(argv):
    if len(argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        native_path, virtual_path, output = map(Path, argv[1:])
        if output.resolve() in (native_path.resolve(), virtual_path.resolve()):
            raise ValueError("output must not replace input evidence")
        native, nr = read_commands(native_path)
        virtual, vr = read_commands(virtual_path)
        result = list(align(native, virtual))
        with output.open("w", encoding="utf-8", newline="") as f:
            writer = csv.writer(f, delimiter="\t", lineterminator="\n")
            writer.writerow(FIELDS)
            writer.writerows(result)
        counts = collections.Counter(r[0] for r in result)
        print(f"native_commands={len(native)} virtual_commands={len(virtual)} "
              f"matched={counts['match']} native_only={counts['native_only']} virtual_only={counts['virtual_only']}")
        print(f"unpaired_native_replies={nr} unpaired_virtual_replies={vr}; "
              "API alignment and FIFO pairing are inferred; bodies are not compared")
        return 0
    except (OSError, ValueError, KeyError, csv.Error) as error:
        print(f"align_metadata: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
