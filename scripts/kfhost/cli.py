"""kfhost command line: setup (explicit, one venv dir), check (read-only), run (the daemon)."""

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

from . import config as config_mod
from .daemon import Daemon
from .jobs import JobRunner
from .transport import WebsocketsTransport

LOCK = Path(__file__).with_name("requirements.lock")


def lock_problem(text):
    if "HASHES TODO" in text or "HASHES_TODO" in text:
        return "requirements.lock still has the HASHES TODO marker; the coordinator must generate the hashes first"
    if "--hash=sha256:" not in text:
        return "requirements.lock has no hashes"
    return None


def setup(venv, lock=LOCK, run=subprocess.run, out=print):
    """Create DIR as a venv and install the locked requirements into it. Touches nothing else."""
    problem = lock_problem(Path(lock).read_text())
    if problem:
        out("setup refused: " + problem)
        return 2
    venv = Path(venv).resolve()
    if venv.exists() and any(venv.iterdir()) and not (venv / "pyvenv.cfg").exists():
        out("setup refused: %s exists, is not empty and is not a venv" % venv)
        return 2
    steps = [
        [sys.executable, "-m", "venv", str(venv)],
        [str(venv / "bin" / "pip"), "install", "--require-hashes", "--only-binary", ":all:", "--no-input", "-r", str(lock)],
    ]
    for argv in steps:
        out("+ " + " ".join(argv))
        if run(argv, check=False).returncode != 0:
            out("setup failed at: " + argv[0])
            return 1
    out("done; run with: %s -m kfhost run --config FILE" % (venv / "bin" / "python"))
    return 0


def check(out=print, which=shutil.which, importer=__import__, exists=os.path.exists):
    """Print what is missing and how to install it. Changes nothing. rc 1 when a required item is missing."""
    missing = 0
    if sys.version_info < (3, 9):
        out("MISSING python >= 3.9 (found %d.%d): install python3 from your distribution" % sys.version_info[:2]); missing += 1
    if not which("git"):
        out("MISSING git: install it (e.g. apt-get install git)"); missing += 1
    try:
        importer("websockets")
    except ImportError:
        out("MISSING python package websockets: python3 -m kfhost setup --venv DIR, then run the daemon with DIR/bin/python"); missing += 1
    if not which("nvidia-smi"):
        out("note: nvidia-smi not found; GPU capabilities will be reported as unknown")
    if not exists("/dev/kvm"):
        out("note: /dev/kvm not present; kvm capability will be false")
    out("ok" if not missing else "%d required item(s) missing" % missing)
    return 1 if missing else 0


def run(args):
    try:
        cfg = config_mod.load(args.config)
    except (config_mod.ConfigError, OSError, ValueError) as error:
        print("kfhost: refusing to start: %s" % error, file=sys.stderr)
        return 2
    os.makedirs(cfg["work_dir"], mode=0o700, exist_ok=True)
    daemon = Daemon(cfg, WebsocketsTransport(), JobRunner)
    return daemon.run_forever()


def main(argv=None):
    p = argparse.ArgumentParser(prog="kfhost", description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("setup")
    s.add_argument("--venv", required=True)
    sub.add_parser("check")
    r = sub.add_parser("run")
    r.add_argument("--config", required=True)
    args = p.parse_args(argv)
    if args.cmd == "setup":
        return setup(args.venv)
    if args.cmd == "check":
        return check()
    return run(args)
