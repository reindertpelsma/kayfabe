#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""RUSTC_WRAPPER for the compiler location gate, G1 (docs/design/V3_SEC_PERIMETER.md §1.2).

Cargo runs `<this> <rustc> <args…>` for every compilation unit, workspace member or not. For
each unit whose package lies inside the checkout (`CARGO_MANIFEST_DIR`), this wrapper:

- appends `--force-warn unsafe_code`, which no source attribute, `-A`, or `--cap-lints` can
  lower (measured 2026-10-04, rustc 1.99.0), so rustc itself reports every `unsafe` block,
  `unsafe fn`, `unsafe impl`/`trait`, `unsafe extern` block, unsafe attribute and `global_asm!`
  after `cfg` and macro expansion;
- for a unit that is neither class U nor exempt, appends `-F unsafe_code` FIRST, so an `allow`
  anywhere fails early with E0453 (the order matters: the last of the two flags wins);
- passes rustc's stderr through unchanged, and records the unit and every `unsafe_code` or
  E0453 diagnostic (primary file and line, and every macro call-site) in `$KF_WRAPLOG/`, one
  JSON file per invocation, so concurrent units never interleave.

The verdict is `perimeter.py location`, not this file. This file only refuses (exit 97, with a
named message) what would make its record untrustworthy:
- `RUSTC_BOOTSTRAP` set (unstable flags would be accepted);
- argv[1] is not the pinned toolchain's rustc (`KF_PINNED_RUSTC`), so nothing can be interposed
  by `RUSTC`, `build.rustc` or a workspace wrapper;
- an in-repo unit carrying `--cap-lints`, any `-Z`, or no `--error-format=json`.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tomllib
import uuid

REFUSED = 97
VALUE_OPTS = {"--crate-name", "--crate-type", "--cfg", "--check-cfg", "-C", "--codegen", "-L", "-l", "--extern",
              "--out-dir", "--target", "--cap-lints", "--print", "-o", "--explain", "--sysroot", "--edition",
              "-A", "-W", "-D", "-F", "--force-warn", "--allow", "--warn", "--deny", "--forbid", "-Z",
              "--emit", "--json", "--error-format", "--env-set", "--remap-path-prefix"}


def refuse(msg: str) -> None:
    sys.stderr.write(f"rustc_location_wrapper: REFUSED: {msg}\n")
    sys.stderr.flush()
    sys.exit(REFUSED)


def under(path: str, root: str) -> bool:
    return path == root or path.startswith(root.rstrip("/") + "/")


def classify(pkg_rel: str, cfg: dict) -> str:
    for c in cfg.get("crates", []):
        if c.get("path") == pkg_rel:
            return c.get("class", "?")
    for e in cfg.get("exempt", []):
        if under(pkg_rel, e["path"]):
            return "exempt"
    return "F"


def parse_args(args: list[str]) -> dict:
    info = {"crate_name": None, "crate_types": [], "test": False, "src": None, "target": None,
            "features": [], "out_dir": None, "extra_filename": "", "error_json": False, "cap_lints": False,
            "z": False}
    i = 0
    while i < len(args):
        a = args[i]
        val = args[i + 1] if i + 1 < len(args) else None
        if a == "--test":
            info["test"] = True
        elif a in ("--cap-lints",) or a.startswith("--cap-lints="):
            info["cap_lints"] = True
        elif a == "-Z" or (a.startswith("-Z") and len(a) > 2):
            info["z"] = True
        elif a.startswith("--error-format="):
            info["error_json"] = a.split("=", 1)[1] == "json"
        elif a == "--error-format" and val is not None:
            info["error_json"] = val == "json"
        if a in VALUE_OPTS and val is not None:
            if a == "--crate-name":
                info["crate_name"] = val
            elif a == "--crate-type":
                info["crate_types"].append(val)
            elif a == "--target":
                info["target"] = val
            elif a == "--out-dir":
                info["out_dir"] = val
            elif a == "--cfg" and val.startswith('feature="'):
                info["features"].append(val[len('feature="'):-1])
            elif a in ("-C", "--codegen") and val.startswith("extra-filename="):
                info["extra_filename"] = val.split("=", 1)[1]
            i += 2
            continue
        if a.startswith("--crate-type="):
            info["crate_types"].append(a.split("=", 1)[1])
        elif a.startswith("--target="):
            info["target"] = a.split("=", 1)[1]
        elif a.startswith("--out-dir="):
            info["out_dir"] = a.split("=", 1)[1]
        elif a.startswith("-Cextra-filename="):
            info["extra_filename"] = a.split("=", 1)[1]
        elif not a.startswith("-") and a.endswith(".rs") and info["src"] is None:
            info["src"] = a
        i += 1
    return info


def rel(path: str, cwd: str, root: str) -> str:
    p = os.path.realpath(path if os.path.isabs(path) else os.path.join(cwd, path))
    return os.path.relpath(p, root) if under(p, root) else p


def callsites(span: dict, cwd: str, root: str) -> list[list]:
    out = []
    exp = span.get("expansion")
    while exp:
        s = exp.get("span") or {}
        out.append([rel(s.get("file_name", "?"), cwd, root), s.get("line_start"), s.get("column_start"),
                    exp.get("macro_decl_name")])
        exp = s.get("expansion")
    return out


def diagnostic(d: dict, cwd: str, root: str) -> dict | None:
    code = (d.get("code") or {}).get("code")
    if code not in ("unsafe_code", "E0453"):
        return None
    spans = [s for s in d.get("spans", []) if s.get("is_primary")]
    if not spans:
        return {"code": code, "level": d.get("level"), "message": d.get("message"), "file": None, "line": 0,
                "col": 0, "callsites": []}
    s = spans[0]
    return {"code": code, "level": d.get("level"), "message": d.get("message"),
            "file": rel(s["file_name"], cwd, root), "line": s.get("line_start"), "col": s.get("column_start"),
            "callsites": callsites(s, cwd, root)}


def main() -> int:
    if "RUSTC_BOOTSTRAP" in os.environ:
        refuse("RUSTC_BOOTSTRAP is set; unstable flags would be accepted")
    if len(sys.argv) < 2:
        refuse("no rustc on the command line")
    rustc, args = sys.argv[1], sys.argv[2:]
    pinned = os.environ.get("KF_PINNED_RUSTC", "")
    if not pinned:
        refuse("KF_PINNED_RUSTC is not set")
    if os.path.realpath(rustc) != os.path.realpath(pinned):
        refuse(f"argv[1] {rustc!r} is not the pinned rustc {pinned!r} (RUSTC, build.rustc or a workspace "
               "wrapper was interposed)")
    root = os.path.realpath(os.environ.get("KF_REPO_ROOT", ""))
    manifest_dir = os.environ.get("CARGO_MANIFEST_DIR")
    if not root or not manifest_dir or not under(os.path.realpath(manifest_dir), root):
        os.execv(rustc, [rustc, *args])  # registry/git units and cargo's own probes: unchanged
    with open(os.environ["KF_PERIMETER_TOML"], "rb") as f:
        cfg = tomllib.load(f)
    pkg_rel = os.path.relpath(os.path.realpath(manifest_dir), root)
    info = parse_args(args)
    if info["cap_lints"]:
        refuse(f"--cap-lints on in-repo unit {pkg_rel} ({info['crate_name']})")
    if info["z"]:
        refuse(f"a -Z flag on in-repo unit {pkg_rel} ({info['crate_name']})")
    if not info["error_json"]:
        refuse(f"in-repo unit {pkg_rel} ({info['crate_name']}) has no --error-format=json")
    klass = classify(pkg_rel, cfg)
    extra = ["--force-warn", "unsafe_code"] if klass in ("U", "exempt") else \
        ["-F", "unsafe_code", "--force-warn", "unsafe_code"]
    cwd = os.getcwd()
    # close_fds=False: cargo's jobserver descriptors must reach rustc.
    proc = subprocess.Popen([rustc, *args, *extra], stderr=subprocess.PIPE, close_fds=False)
    diags = []
    assert proc.stderr is not None
    for raw in proc.stderr:
        sys.stderr.buffer.write(raw)
        sys.stderr.buffer.flush()
        try:
            d = json.loads(raw)
        except ValueError:
            continue
        if isinstance(d, dict) and d.get("$message_type") == "diagnostic":
            rec = diagnostic(d, cwd, root)
            if rec is not None:
                diags.append(rec)
    rc = proc.wait()
    unit = {"pass": os.environ.get("KF_PASS", "?"), "package": os.environ.get("CARGO_PKG_NAME"),
            "manifest_dir": pkg_rel, "class": klass, "crate_name": info["crate_name"],
            "crate_types": info["crate_types"], "test": info["test"],
            "src": rel(info["src"], cwd, root) if info["src"] else None, "target": info["target"],
            "features": sorted(info["features"]), "out_dir": info["out_dir"],
            "extra_filename": info["extra_filename"], "cwd": cwd, "rc": rc, "flags": extra}
    logdir = os.environ.get("KF_WRAPLOG")
    if not logdir:
        refuse("KF_WRAPLOG is not set")
    tmp = os.path.join(logdir, f".{uuid.uuid4().hex}.tmp")
    with open(tmp, "w") as f:
        json.dump({"unit": unit, "diags": diags}, f)
    os.rename(tmp, os.path.join(logdir, f"{os.getpid()}-{uuid.uuid4().hex}.json"))
    return rc


if __name__ == "__main__":
    sys.exit(main())
