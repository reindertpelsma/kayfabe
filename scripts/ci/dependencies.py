"""Check actual manifest dependencies, including target-specific and renamed ones."""
from pathlib import Path
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
    for failure in failures:
        print(failure)
    print(f"V3_ISOLATION crates={count} violations={len(failures)}")
    return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
