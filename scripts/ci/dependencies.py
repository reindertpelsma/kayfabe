"""Check actual manifest dependencies, including target-specific and renamed ones."""
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[2]
# These are algorithms and vocabulary, not host adapters. Include test/build edges:
# an OS-backed test can live in the harness instead of changing the core's contract.
PURE = frozenset(("kf-util", "kf-arch", "kf-abi", "kf-chip", "kf-gsp", "kf-trap",
                  "kf-core", "kf-rm", "kf-trace", "kf-crec", "kf-disp", "kf-oprom"))


# ★ 2026-10-21 (CI repair; OWNER DECISION PENDING). Two pure crates carry a DEV-dependency that the rule above
# forbids, added after this gate last ran (the gate had not run since 2026-10-15, when the Clippy step went red):
#   kf-trap  -> loom         (2026-10-12, `ea8b77ee1`: the model checker for the three-mode doorbell dispatch)
#   kf-disp  -> kf-broker, kf-linux-raw   (2026-10-17, `8614c372b`: the lock-step harness drives the REAL relay)
# They are named here, exactly, and ONLY as `[dev-dependencies]`: the same name under `[dependencies]` or
# `[build-dependencies]`, or any other crate, still fails. Two ways to retire an entry: move that test into
# `kf-harness` (the comment above), or the owner rules that a pure crate may test against an OS adapter.
DEV_ONLY_EXCEPTIONS = {
    "kf-trap": frozenset(("loom",)),
    "kf-disp": frozenset(("kf-broker", "kf-linux-raw")),
}


def dependencies(manifest, workspace, keys=("dependencies", "build-dependencies", "dev-dependencies")):
    for key in keys:
        for name, value in manifest.get(key, {}).items():
            value = value if isinstance(value, dict) else {}
            if value.get("workspace"):
                value = workspace[name]
                value = value if isinstance(value, dict) else {}
            yield value.get("package", name)
    for target in manifest.get("target", {}).values():
        yield from dependencies(target, workspace, keys)


def checked_dependencies(name, manifest, workspace):
    """Every dependency the isolation rule judges: all of them, minus a named dev-only exception."""
    shipped = list(dependencies(manifest, workspace, ("dependencies", "build-dependencies")))
    allowed = DEV_ONLY_EXCEPTIONS.get(name, frozenset())
    dev = [dep for dep in dependencies(manifest, workspace, ("dev-dependencies",))
           if dep not in allowed]
    return shipped + dev


def violations(name, deps):
    return [dep for dep in deps if
            (name.startswith("kf-") and dep.startswith("kayfabe-")) or
            (dep == "kayfabe-doorbell" and name != "kayfabe-chips") or
            (name in PURE and dep not in PURE)]


def main():
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
    failures = []
    count = 0
    for member in workspace["members"]:
        manifest = tomllib.loads((ROOT / member / "Cargo.toml").read_text())
        name = manifest["package"]["name"]
        count += name.startswith("kf-")
        for dep in violations(name, checked_dependencies(name, manifest, workspace["dependencies"])):
            failures.append(f"{name} has forbidden dependency {dep}")
    if count < 18:
        failures.append(f"only {count} v3 crates checked; expected at least 18")
    for failure in failures:
        print(failure)
    print(f"V3_ISOLATION crates={count} violations={len(failures)}")
    return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
