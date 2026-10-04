"""Check actual manifest dependencies, including target-specific and renamed ones.

Also M6 (docs/design/V3_SEC_PERIMETER.md §1.5, audit S1-14): kf3's resolved normal+build
closure is exactly the kf-* members plus the reviewed external set in
scripts/ci/perimeter.toml `[kf3]`, with pinned features and build scripts.
"""
import json
import os
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[2]
# These are algorithms and vocabulary, not host adapters. Include test/build edges:
# an OS-backed test can live in the harness instead of changing the core's contract.
PURE = frozenset(("kf-util", "kf-arch", "kf-abi", "kf-chip", "kf-gsp", "kf-trap",
                  "kf-core", "kf-rm", "kf-trace", "kf-crec", "kf-disp", "kf-oprom"))


def dependencies(manifest, workspace):
    for key in ("dependencies", "build-dependencies", "dev-dependencies"):
        for name, value in manifest.get(key, {}).items():
            value = value if isinstance(value, dict) else {}
            if value.get("workspace"):
                value = workspace[name]
                value = value if isinstance(value, dict) else {}
            yield value.get("package", name)
    for target in manifest.get("target", {}).values():
        yield from dependencies(target, workspace)


def violations(name, deps):
    return [dep for dep in deps if
            (name.startswith("kf-") and dep.startswith("kayfabe-")) or
            (dep == "kayfabe-doorbell" and name != "kayfabe-chips") or
            (name in PURE and dep not in PURE)]


def kf3_closure(meta, root_name):
    """Package ids reachable from `root_name` over normal and build edges (any target)."""
    pk = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    roots = [i for i in meta["workspace_members"] if pk[i]["name"] == root_name]
    if len(roots) != 1:
        raise ValueError(f"kf3 root {root_name!r} is not exactly one workspace member")
    seen, stack = set(), roots
    while stack:
        x = stack.pop()
        if x in seen:
            continue
        seen.add(x)
        for dep in nodes[x]["deps"]:
            if any(k["kind"] in (None, "build") for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    return seen


def kf3_violations(meta, kf3, root):
    """M6: membership, custom builds and enabled features of kf3's closure."""
    pk = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    members = set(meta["workspace_members"])
    external = kf3.get("external", {})
    member_features = kf3.get("member_features", {})
    out = []
    closure = kf3_closure(meta, kf3["root"])
    builds = set()
    for x in sorted(closure, key=lambda i: pk[i]["name"]):
        p = pk[x]
        name = p["name"]
        rel = os.path.relpath(os.path.realpath(p["manifest_path"]), os.path.realpath(root))
        parts = Path(rel).parts
        is_kf_member = (x in members and len(parts) == 3 and parts[0] == "crates"
                        and parts[1].startswith("kf-") and parts[2] == "Cargo.toml")
        if not is_kf_member and name not in external:
            out.append(f"M6 kf3 closure holds {name} ({rel}), neither a crates/kf-* member nor a reviewed external")
        if any("custom-build" in t["kind"] for t in p["targets"]):
            builds.add(name)
        want = external.get(name) if name in external else member_features.get(name, [])
        got = sorted(nodes[x]["features"])
        if got != sorted(want):
            out.append(f"M6 {name} enabled features {got} != pinned {sorted(want)}")
    if builds != set(kf3.get("custom_build", [])):
        out.append(f"M6 kf3 build scripts {sorted(builds)} != pinned {sorted(kf3.get('custom_build', []))}")
    return out


def cargo_metadata(root):
    run = subprocess.run(["cargo", "metadata", "--format-version", "1", "--locked"], cwd=root,
                         capture_output=True, text=True)
    if run.returncode != 0:
        raise SystemExit(f"cargo metadata --locked failed:\n{run.stderr}")
    return json.loads(run.stdout)


def main():
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
    failures = []
    count = 0
    for member in workspace["members"]:
        manifest = tomllib.loads((ROOT / member / "Cargo.toml").read_text())
        name = manifest["package"]["name"]
        count += name.startswith("kf-")
        for dep in violations(name, dependencies(manifest, workspace["dependencies"])):
            failures.append(f"{name} has forbidden dependency {dep}")
    if count < 18:
        failures.append(f"only {count} v3 crates checked; expected at least 18")
    kf3 = tomllib.loads((ROOT / "scripts/ci/perimeter.toml").read_text())["kf3"]
    meta = cargo_metadata(ROOT)
    m6 = kf3_violations(meta, kf3, ROOT)
    failures += m6
    for failure in failures:
        print(failure)
    print(f"V3_ISOLATION crates={count} violations={len(failures)} "
          f"kf3_closure={len(kf3_closure(meta, kf3['root']))}")
    return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
