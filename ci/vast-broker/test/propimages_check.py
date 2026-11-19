"""The box side of the Worker's propimages instruction, read from the code the box runs (stdlib only).

stdin JSON, one of:
  {"mode": "registry"}                        the application ids the runner has a definition for
  {"mode": "roundtrip", "text": "..."}        the instruction text through scripts/ci/payload_instruction.parse: its canonical bytes
                                              and the propimage_runspec.validate_list form of its images
  {"mode": "cli", "argv": [...], "images": [...], "strict": bool}
                                              the image flags of a standalone command line (split_flags) and the instruction's
                                              image list (validate_list), both normalized
Prints JSON. Refusals are returned as {"refused": "<name>"}, never raised, so a test can assert the exact name.
"""

import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "scripts/ci"))
sys.path.insert(0, str(ROOT / "scripts/bench/windows"))
import payload_instruction as pinst  # noqa: E402
import propimage_registry  # noqa: E402
import propimage_runspec as spec  # noqa: E402

data = json.load(sys.stdin)
mode = data["mode"]
out = {}
try:
    if mode == "registry":
        out["ids"] = sorted(propimage_registry.load_registry())
    elif mode == "roundtrip":
        inst = pinst.parse(data["text"].encode())
        out["canonical"] = inst.canonical_bytes().decode()
        out["runspec"] = None
        if inst.propimages:
            out["runspec"] = spec.validate_list(inst.propimages, inst.propimages_strict)
    elif mode == "cli":
        rest, cli = spec.split_flags(data["argv"])
        out["rest"] = rest
        out["cli"] = spec.validate(cli) if cli else None
        out["worker"] = spec.validate_list(data["images"], data["strict"])
    else:
        out["refused"] = "MODE"
except (pinst.InstructionError, spec.SpecError) as exc:
    out["refused"] = str(exc)
print(json.dumps(out, sort_keys=True))
