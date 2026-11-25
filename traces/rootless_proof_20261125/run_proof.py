#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Rootless proof driver, run AS ROOT on the box (traces/rootless_proof_20261125/PREREGISTRATION.md).

  run_proof.py stage NAME [ARGS...]     one stage.sh stage, dropped to kfrun, with procsample.py around it
  run_proof.py pair                     wininstall and the suite at the same time (each with its own sampler)
  run_proof.py neg N1|N2|N3             one H3 control: change one permission, run one arm dropped, undo the change

Logs go to /root/rl/<label>.log (stage output) and /root/rl/<label>.procs.jsonl (sampler). Each log ends with
`RUN_PROOF_EXIT label=<label> rc=<n>`. Stdlib only.
"""
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = "/workspace/kf-src"
OUT = "/root/rl"
USER = "kfrun"
MEMLOCK = 8 * 1024 * 1024
STAGE = f"{SRC}/traces/rootless_proof_20261125/stage.sh"


def dropped(args, memlock=MEMLOCK, groups=True):
    lim = ["prlimit", f"--memlock={memlock}:{memlock}"] if memlock else []
    if groups:
        return lim + ["python3", "-I", f"{SRC}/scripts/ci/kfroot.py", "run", "--user", USER, "--no-memlock", "--",
                      "bash", STAGE, *args]
    # N3: the same drop without the kvm group
    return lim + ["setpriv", "--reuid", USER, "--regid", USER, "--clear-groups", "--inh-caps=-all", "--ambient-caps=-all",
                  "--bounding-set=-all", "--no-new-privs", "--", "bash", STAGE, *args]


def start(label, argv):
    stop = f"{OUT}/{label}.stop"
    if os.path.exists(stop):
        os.unlink(stop)
    samp = subprocess.Popen(["python3", "-I", f"{HERE}/procsample.py", "--user", USER, "--out", f"{OUT}/{label}.procs.jsonl",
                             "--stop-file", stop], stdout=open(f"{OUT}/{label}.procsum.txt", "w"), stderr=subprocess.STDOUT)
    log = open(f"{OUT}/{label}.log", "w")
    log.write(f"RUN_PROOF_START label={label} t={time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} argv={argv}\n")
    log.flush()
    proc = subprocess.Popen(argv, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, cwd="/")
    return proc, samp, log, stop


def finish(label, proc, samp, log, stop):
    rc = proc.wait()
    time.sleep(2)
    open(stop, "w").close()
    samp.wait()
    log.write(f"RUN_PROOF_EXIT label={label} rc={rc} t={time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}\n")
    log.close()
    return rc


def run(label, argv):
    return finish(label, *start(label, argv))


def neg(which):
    node = {"N1": "/dev/nvidiactl", "N2": "/dev/nvidia-uvm"}.get(which)
    old = None
    if node:
        old = os.stat(node).st_mode & 0o7777
        os.chmod(node, 0o600)
    try:
        return run(f"neg_{which}", dropped(["arm", f"rlneg_{which}"], groups=(which != "N3")))
    finally:
        if node:
            os.chmod(node, old)
            print(f"restored {node} to {oct(old)}")


def main():
    os.makedirs(OUT, exist_ok=True)
    cmd = sys.argv[1]
    if cmd == "stage":
        rc = run(sys.argv[2] + (("_" + sys.argv[3]) if len(sys.argv) > 3 else ""), dropped(sys.argv[2:]))
    elif cmd == "pair":
        a = start("wininstall", dropped(["wininstall"]))
        time.sleep(5)
        b = start("suite", dropped(["suite", "rlsuite"]))
        rb = finish("suite", *b)
        ra = finish("wininstall", *a)
        rc = ra or rb
    elif cmd == "neg":
        rc = neg(sys.argv[2])
    else:
        print(__doc__)
        rc = 2
    print(f"RUN_PROOF_DONE cmd={cmd} rc={rc}")
    return rc


if __name__ == "__main__":
    sys.exit(main())
