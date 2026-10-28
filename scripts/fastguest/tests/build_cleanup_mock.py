"""Inert executable fixtures for guest-builder cleanup tests; never access devices."""

import json
import os
from pathlib import Path
import shutil
import signal
import sys

name = Path(sys.argv[0]).name
arguments = sys.argv[1:]
log = Path(os.environ["BUILDER_CALLS"])
with log.open("a") as stream:
    stream.write(json.dumps([name, *arguments]) + "\n")

if name in ("pgrep", "ldd"):
    sys.exit(1)
if name == "uname":
    print("fixture-kernel-that-does-not-exist")
elif name == "cp":
    if os.environ.get("BUILDER_SIGNAL_AT") == "host-copy":
        os.kill(os.getppid(), int(os.environ["BUILDER_SIGNAL"]))
        sys.exit(1)
    source, destination = map(Path, arguments[-2:])
    if not source.is_file():
        sys.exit(1)
    shutil.copyfile(source, destination)
elif name == "qemu-nbd":
    if arguments[0] == "--disconnect":
        attempts = sum(
            json.loads(line)[:2] == ["qemu-nbd", "--disconnect"]
            for line in log.read_text().splitlines()
        )
        if os.environ.get("BUILDER_RELEASE_RETRY") == "1" and attempts == 1:
            sys.exit(1)
    else:
        if os.environ.get("BUILDER_SIGNAL_AT") == "attach":
            os.kill(os.getppid(), int(os.environ["BUILDER_SIGNAL"]))
        sys.exit(int(os.environ.get("BUILDER_ATTACH_RC", "0")))
elif name == "mount":
    if os.environ.get("BUILDER_SIGNAL_AT") == "mount":
        os.kill(os.getppid(), int(os.environ["BUILDER_SIGNAL"]))
        sys.exit(1)
    if os.environ.get("BUILDER_MOUNT_RC") == "1":
        sys.exit(1)
    root = Path(arguments[-1])
    modules = root / "lib/modules/fixture-kernel/kernel"
    modules.mkdir(parents=True)
    (modules / "nvidia.ko").write_text("inert module fixture\n")
    (modules.parent / "modules.dep").write_text("kernel/nvidia.ko:\n")
    (root / "boot").mkdir()
    (root / "boot/vmlinuz-fixture-kernel").write_text("inert kernel fixture\n")
elif name == "cpio":
    sys.stdin.buffer.read()
    sys.stdout.buffer.write(b"inert archive fixture\n")
    sys.exit(int(os.environ.get("BUILDER_PACK_RC", "0")))
elif name not in ("busybox", "modprobe", "partprobe", "udevadm", "umount", "sleep"):
    raise RuntimeError("unexpected fixture command: " + name)
