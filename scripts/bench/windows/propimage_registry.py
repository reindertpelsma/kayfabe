#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The application registry of the proprietary-image runner: application identity -> what the drivers know how to run (stdlib only).

STATUS: LIVE, 2026-11-16 (phase 1; design: docs/design/V3_PROPIMAGE_RUNNER.md). GPU-free tests: scripts/ci/test_propimage_runspec.py.

An application is known by a short identity (`resolve`, `3dmark`, ...). The registry holds one definition per identity; the runner
intersects it with what an image declares (state.json `software`, the declaration in the run spec, or a probe of the installed
programs) and runs only the intersection. Anything installed that no definition matches is reported NOT RUN (no driver), never an error.

PLUG-IN CONTRACT (one file per definition set, no shared table to merge): every `scripts/bench/windows/app_defs/*.py` module defines

    APPS = [ {id, title, timeout, kind, match, driver, harness_args, needs, aliases}, ... ]

  id            lower-case identity, `[a-z0-9][a-z0-9_]{0,31}`, unique across all modules (a duplicate is refused by name)
  title         a human name for reports
  timeout       seconds the app's own check may take (the runner plans boots from it)
  kind          the evidence kind a PASS must carry: FILE | UIA | NONE (propimage_check.verdict)
  match         list of regular expressions matched (case-insensitive, search) against installed-program display names and against
                `software` entries of a state.json; the id itself always matches
  driver        None = propimage_check.run_check (its CHECKS table); or "module:function" in scripts/bench/windows, called as
                function(name, out, guest, **options) and returning the same verdict row as run_check
  harness_args  extra command-line words the app needs on the propimage_check command (merged over the apps of one boot)
  group         apps of different groups never share a boot (default `core`; a definition set whose driver needs its own harness
                switches, for example `--app-set`, names its own group)
  solo          true: the app always boots alone, after the others (a GPU watchdog event there can wedge the guest)
  needs         names of resources the runner must provide (none are provided in phase 1: such an app is NOT RUN `NEEDS:<name>`)
  aliases       other spellings of the id accepted on the command line

A new application = a new file in app_defs/. Nothing else is edited.
"""
import importlib.util
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFS_DIR = HERE / "app_defs"
ID_RE = re.compile(r"[a-z0-9][a-z0-9_]{0,31}")
KINDS = ("FILE", "UIA", "NONE")
KEYS = {"id", "title", "timeout", "kind", "match", "driver", "harness_args", "needs", "aliases", "group", "solo"}
REQUIRED = {"id", "title", "timeout", "kind"}
PROVIDED = frozenset()          # resources the runner provides today (phase 1: none)


class RegistryError(ValueError):
    """A definition is malformed or collides with another. The message names the module and the identity."""


def _check(d, origin):
    if type(d) is not dict or not REQUIRED <= set(d) <= KEYS:
        raise RegistryError("%s: keys" % origin)
    if not ID_RE.fullmatch(str(d["id"])):
        raise RegistryError("%s: id" % origin)
    if d["kind"] not in KINDS or type(d["timeout"]) is not int or not 30 <= d["timeout"] <= 7200:
        raise RegistryError("%s: %s kind or timeout" % (origin, d["id"]))
    out = {"match": [], "driver": None, "harness_args": [], "needs": [], "aliases": [], "group": "core", "solo": False, "origin": origin}
    out.update(d)
    for key in ("match", "harness_args", "needs", "aliases"):
        if type(out[key]) is not list or not all(type(x) is str and 0 < len(x) <= 200 for x in out[key]):
            raise RegistryError("%s: %s %s" % (origin, d["id"], key))
    for rx in out["match"]:
        try:
            re.compile(rx)
        except re.error:
            raise RegistryError("%s: %s match" % (origin, d["id"])) from None
    if type(out["solo"]) is not bool or not ID_RE.fullmatch(str(out["group"])):
        raise RegistryError("%s: %s group or solo" % (origin, d["id"]))
    if not all(ID_RE.fullmatch(x) for x in out["aliases"]):
        raise RegistryError("%s: %s aliases" % (origin, d["id"]))
    if out["driver"] is not None and not re.fullmatch(r"[a-z_][a-z0-9_]*:[a-z_][a-z0-9_]*", str(out["driver"])):
        raise RegistryError("%s: %s driver" % (origin, d["id"]))
    return out


def load_registry(defs_dir=DEFS_DIR):
    """{id: definition} from every module in `defs_dir` (sorted by file name). A duplicate identity or alias is refused."""
    reg, names = {}, {}
    for path in sorted(Path(defs_dir).glob("*.py")):
        if path.name.startswith("_"):
            continue
        spec = importlib.util.spec_from_file_location("kf_app_defs_" + path.stem, path)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        apps = getattr(mod, "APPS", None)
        if type(apps) is not list:
            raise RegistryError("%s: no APPS list" % path.name)
        for raw in apps:
            d = _check(raw, path.name)
            for name in [d["id"], *d["aliases"]]:
                if name in names:
                    raise RegistryError("%s: %s already defined by %s" % (path.name, name, names[name]))
                names[name] = path.name
            reg[d["id"]] = d
    return reg


def canonical(reg, name):
    """The identity behind a spelling (id or alias), or None."""
    name = str(name).strip().lower()
    if name in reg:
        return name
    return next((i for i, d in reg.items() if name in d["aliases"]), None)


def identify(reg, text):
    """The identity whose id or `match` expressions fit this program name (installed-programs list, state.json software), or None."""
    hit = canonical(reg, text)
    if hit:
        return hit
    for ident, d in reg.items():
        if any(re.search(rx, str(text), re.I) for rx in d["match"]):
            return ident
    return None


def declared_names(software):
    """Strings from a state.json `software` value: a list of strings or of objects with `id` or `name`. Anything else is ignored."""
    out = []
    for item in software if type(software) is list else []:
        if type(item) is str:
            out.append(item)
        elif type(item) is dict:
            for key in ("id", "name"):
                if type(item.get(key)) is str:
                    out.append(item[key])
                    break
    return out[:512]


def discover(reg, names):
    """(supported ids in registry order, unsupported names) for a list of declared or probed program names."""
    found, unknown = set(), []
    for n in names:
        ident = identify(reg, n)
        if ident:
            found.add(ident)
        elif str(n).strip() and str(n) not in unknown:
            unknown.append(str(n)[:80])
    return [i for i in reg if i in found], unknown[:64]


def unmet_needs(definition, provided=PROVIDED):
    return [n for n in definition["needs"] if n not in provided]


def driver_for(definition):
    """(module path, function name) of a definition's driver, or None for propimage_check.run_check."""
    if definition["driver"] is None:
        return None
    mod, _, func = definition["driver"].partition(":")
    return HERE / (mod + ".py"), func
