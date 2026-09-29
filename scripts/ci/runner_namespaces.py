"""Give only this ephemeral CI checkout's test binaries the userns prerequisite.

Ubuntu's generic unprivileged_userns profile rejects disconnected memfd exec and
namespace capabilities. A named unconfined profile permits the tests to exercise
Kayfabe's own containment. AppArmor remains enabled; no sysctl is changed.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def profile_text(root: Path) -> str:
    path = str(root / "target")
    if not root.is_absolute() or any(char in path for char in '\n\r"\\*?[]{}'):
        raise ValueError("unsafe AppArmor attachment path")
    return ("abi <abi/4.0>,\ninclude <tunables/global>\n"
            f'profile kayfabe-ci-tests "{path}/**" flags=(unconfined) {{\n  userns,\n}}\n')


def main() -> int:
    if os.environ.get("GITHUB_ACTIONS") != "true":
        print("CI namespace profile: not a GitHub runner; host policy unchanged")
        return 0
    if Path(os.environ["GITHUB_WORKSPACE"]).resolve() != ROOT:
        raise ValueError("profile must attach only to the current checkout")
    sysctl = Path("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
    if not sysctl.exists() or sysctl.read_text().strip() != "1":
        print("CI namespace profile: additional runner userns restriction is absent")
        return 0
    remove = sys.argv[1:] == ["--remove"]
    if sys.argv[1:] and not remove:
        raise ValueError("expected no argument or --remove")
    with tempfile.TemporaryDirectory(prefix="kayfabe-ci-policy-") as directory:
        policy = Path(directory) / "profile"
        policy.write_text(profile_text(ROOT))
        subprocess.run(["sudo", "--", "apparmor_parser", "-R" if remove else "-r",
                        str(policy)], check=True)
    print(f"CI namespace profile: {'removed' if remove else 'installed'} for {ROOT / 'target'}/**")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
