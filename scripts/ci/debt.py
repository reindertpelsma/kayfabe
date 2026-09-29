#!/usr/bin/env python3
"""Ratchet explicitly recorded diagnostic debt without hiding new sites.

This is not a test allowlist: errors and failed tests are never eligible. Old
count-only prose ceilings could trade one removed claim for a new unsupported
claim. Here each debt item is bound to its path, category and normalized text.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]


def identity(path: str, kind: str, text: str) -> str:
    normalized = " ".join(text.split())
    digest = hashlib.sha256(normalized.encode()).hexdigest()
    return f"{path}|{kind}|{digest}"


def compare(actual: dict[str, str], baseline: dict[str, str]) -> list[str]:
    """Removed debt never licenses a different new diagnostic."""
    return sorted(actual.keys() - baseline.keys())


def frozen() -> dict[str, str]:
    """Bind the frozen prototype's gate exceptions to its exact existing source."""
    crate = ROOT / "crates/kayfabe-doorbell"
    paths = sorted([crate / "Cargo.toml", *crate.rglob("*.rs")])
    return {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in paths if "target" not in path.parts}


def claims() -> dict[str, str]:
    spec = importlib.util.spec_from_file_location("claim_ledger", ROOT / "scripts/claim_ledger.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    found = {}
    for site in module.sweep():
        kinds = []
        if site.cls == module.UNATTRIBUTED:
            kinds.append("unattributed")
        if site.conflated:
            kinds.append("conflated")
        if site.bare_hw:
            kinds.append("bare-hardware")
        for kind in kinds:
            found[identity(str(site.path), kind, site.body)] = site.text
    return found


def clippy(log: Path) -> dict[str, str]:
    found = {}
    finished = False
    for line in log.read_text().splitlines():
        row = json.loads(line)
        if row.get("reason") == "build-finished":
            if not row["success"]:
                raise ValueError("Clippy build failed; errors cannot be recorded as debt")
            finished = True
        if row.get("reason") != "compiler-message":
            continue
        msg = row["message"]
        if msg["level"] == "error":
            raise ValueError(msg["rendered"])
        if msg["level"] != "warning":
            continue
        code = (msg.get("code") or {}).get("code", "warning")
        spans = [s for s in msg["spans"] if s["is_primary"]]
        if not spans:
            raise ValueError(f"unlocated warning cannot be baselined: {msg['message']}")
        for span in spans:
            path = span["file_name"]
            if Path(path).is_absolute():
                path = str(Path(path).relative_to(ROOT))
            # Pin the source at the diagnostic, not its line number (rustfmt moves lines).
            source = " ".join(t["text"] for t in span["text"])
            text = f"{msg['message']} :: {source}"
            found[identity(path, code, text)] = text
    if not finished:
        raise ValueError("Clippy log is truncated or has no successful completion marker")
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=("claims", "clippy", "frozen"))
    parser.add_argument("--log", type=Path)
    parser.add_argument("--record", action="store_true", help="explicitly record reviewed migration debt")
    args = parser.parse_args()
    if args.kind == "clippy" and args.log is None:
        parser.error("clippy requires --log")
    actual = {"claims": claims, "clippy": lambda: clippy(args.log), "frozen": frozen}[args.kind]()
    path = ROOT / "scripts/ci" / f"{args.kind}-debt.json"
    if args.record:
        path.write_text(json.dumps(actual, sort_keys=True, indent=2) + "\n")
        print(f"RECORDED {len(actual)} existing {args.kind} diagnostic sites in {path}")
        print("This is migration debt, not a claim that those diagnostics are fixed.")
        return 0
    baseline = json.loads(path.read_text())
    if args.kind == "frozen":
        changed = sorted(key for key in actual.keys() | baseline.keys()
                         if actual.get(key) != baseline.get(key))
        for key in changed:
            print(f"FROZEN SOURCE CHANGED: {key}")
        print(f"frozen prototype: files={len(actual)} changed={len(changed)}")
        return bool(changed)
    new = compare(actual, baseline)
    for key in new:
        print(f"NEW {key}: {actual[key]}")
    print(f"{args.kind}: existing-debt={len(actual) - len(new)} new={len(new)} "
          f"resolved={len(baseline.keys() - actual.keys())}")
    return bool(new)


if __name__ == "__main__":
    sys.exit(main())
