"""Run the containment suite with a privileged observer, requiring its real bites.

The ordinary workspace run remains unprivileged. This second run is necessary to
inspect a deliberately non-dumpable child's namespaces and to check a real
capability drop from a parent that initially has capabilities.
"""
import json
import os
from pathlib import Path
import subprocess


def main() -> int:
    if os.environ.get("GITHUB_ACTIONS") != "true":
        print("Privileged CI observer: not a GitHub runner; no privilege change")
        return 0
    build = subprocess.run(["cargo", "test", "-p", "kayfabe-isolate-host", "--test",
                            "sandbox_escape", "--no-run", "--message-format=json"],
                           check=True, text=True, stdout=subprocess.PIPE)
    executables = {row["executable"] for line in build.stdout.splitlines()
                   if (row := json.loads(line)).get("reason") == "compiler-artifact"
                   and row["target"]["name"] == "sandbox_escape" and row.get("executable")}
    if len(executables) != 1:
        raise ValueError(f"expected exactly one sandbox test executable, got {len(executables)}")
    executable = Path(executables.pop()).resolve()
    target = Path(os.environ["GITHUB_WORKSPACE"]).resolve() / "target"
    if not executable.is_relative_to(target):
        raise ValueError("refusing a privileged executable outside this checkout's target")
    result = subprocess.run(["sudo", "--", str(executable), "--nocapture"], text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    print(result.stdout, end="")
    result.check_returncode()
    for name in ("the_sandboxed_child_lives_in_its_own_user_namespace",
                 "the_sandboxed_childs_capability_ceiling_is_empty_when_it_could_be_emptied",
                 "the_sandboxed_child_holds_no_capability_at_all"):
        if f"SANDBOX-GATE: RAN {name}\n" not in result.stdout:
            raise ValueError(f"privileged containment assertion did not run: {name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
