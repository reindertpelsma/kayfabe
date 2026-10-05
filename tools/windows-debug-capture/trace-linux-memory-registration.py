#!/usr/bin/env python3
"""Observe the native memory-registration probe through two scoped kprobes.

The controller runs as root only to install/remove its trace instance. The RM
client runs as nobody with every capability dropped and no_new_privs. Existing
trace instances and settings are left alone. No argument pointers are traced.
"""
import argparse
import fcntl
import os
from pathlib import Path
import signal
import subprocess
import sys


def trace_command(path, command):
    # A tracefs command endpoint, not a regular appendable file. Neither truncate
    # existing probes nor depend on regular-file append semantics.
    fd = os.open(path, os.O_WRONLY)
    try:
        data = command.encode()
        if os.write(fd, data) != len(data):
            raise RuntimeError("short tracefs command write")
    finally:
        os.close(fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("probe", type=Path)
    args = parser.parse_args()
    probe = args.probe.resolve(strict=True)
    trace_root = Path("/sys/kernel/tracing")
    group = "kf_memreg_20261005"
    instance = trace_root / "instances" / group
    definitions = trace_root / "kprobe_events"
    if os.geteuid() != 0:
        parser.error("the observer requires root; the actual probe drops privileges")
    if instance.exists() or group + "/" in definitions.read_text():
        parser.error("existing trace state uses this probe name; refusing to replace it")
    created = []
    instance_created = False
    with open("/tmp/kayfabe-fastguest.lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        try:
            for name, expression in [
                ("alloc", "p:{group}/alloc nvidia:rpcAllocMemory_v13_01"),
                ("result", "r:{group}/result nvidia:rpcAllocMemory_v13_01 status=$retval:u32"),
            ]:
                trace_command(definitions, expression.format(group=group) + "\n")
                created.append(name)
            instance.mkdir()
            instance_created = True
            (instance / "buffer_size_kb").write_text("64\n")
            for mode in ("plain", "registered"):
                child = subprocess.Popen([
                    sys.executable, "-c",
                    "import os,signal,sys; os.kill(os.getpid(),signal.SIGSTOP); "
                    "os.execv('/usr/bin/setpriv', ['/usr/bin/setpriv', "
                    "'--reuid=65534','--regid=65534','--clear-groups',"
                    "'--bounding-set=-all','--inh-caps=-all','--ambient-caps=-all',"
                    "'--no-new-privs',sys.argv[1],sys.argv[2]])",
                    str(probe), mode,
                ], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
                try:
                    _, stopped = os.waitpid(child.pid, os.WUNTRACED)
                    if not os.WIFSTOPPED(stopped):
                        raise RuntimeError("probe did not stop before RM access")
                    (instance / "set_event_pid").write_text(str(child.pid) + "\n")
                    (instance / "trace").write_text("")
                    (instance / "events" / group / "enable").write_text("1\n")
                    (instance / "tracing_on").write_text("1\n")
                    os.kill(child.pid, signal.SIGCONT)
                    output, _ = child.communicate(timeout=30)
                    (instance / "tracing_on").write_text("0\n")
                    (instance / "events" / group / "enable").write_text("0\n")
                    print(f"BEGIN mode={mode} pid={child.pid} rc={child.returncode}", flush=True)
                    print(output, end="", flush=True)
                    print((instance / "trace").read_text(), end="", flush=True)
                    print(f"END mode={mode}", flush=True)
                    if child.returncode != 0:
                        raise RuntimeError("native probe failed; see its recorded result")
                finally:
                    if child.poll() is None:
                        child.kill()
                        child.wait()
        finally:
            if instance_created:
                (instance / "tracing_on").write_text("0\n")
                (instance / "events" / group / "enable").write_text("0\n")
                instance.rmdir()
            for name in reversed(created):
                trace_command(definitions, f"-:{group}/{name}\n")
            print("OBSERVER_CLEANUP_COMPLETE", flush=True)


if __name__ == "__main__":
    main()
