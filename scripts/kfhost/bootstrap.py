#!/usr/bin/env python3
"""One-command join for a host from any provider. Stdlib only.

STATUS: DESIGN, NOT RUN ON HARDWARE (2026-11-12).

  python3 bootstrap.py --repo URL --commit SHA --worker-url URL --box-id ID --token-file FILE \\
      [--access-id ID --access-secret-file FILE] [--work-dir DIR] [--dry-run] [-- kfhost run flags]

It prints what it will do, then: clones the daemon at the PINNED commit into DIR/checkout and refuses unless
`git rev-parse HEAD` equals SHA; creates DIR/venv through `kfhost setup --venv`; writes DIR/config.json with
mode 0600; execs `kfhost run`. It touches nothing outside DIR, installs no system package, and takes secrets
only from files or stdin (`-`), never argv. --dry-run prints the plan and changes nothing.
"""

import argparse
import json
import os
import re
import stat
import subprocess
import sys
from pathlib import Path

HEX40 = re.compile(r"[0-9a-f]{40}\Z")


class BootstrapError(Exception):
    pass


def read_secret(spec, stdin):
    """A secret from a file (mode must not be group/world readable) or from stdin when spec is '-'."""
    if spec == "-":
        value = stdin.readline()
    else:
        mode = os.stat(spec).st_mode
        if not stat.S_ISREG(mode) or mode & 0o077:
            raise BootstrapError("%s must be a regular file readable only by you (chmod 600)" % spec)
        with open(spec, encoding="utf-8") as f:
            value = f.read()
    value = value.strip()
    if not value:
        raise BootstrapError("empty secret in %s" % spec)
    return value


def parser():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--repo", required=True, help="https URL of the public repository")
    p.add_argument("--commit", required=True, help="40-hex commit to pin")
    p.add_argument("--worker-url", required=True)
    p.add_argument("--box-id", required=True)
    p.add_argument("--token-file", required=True, help="file with the per-box token, or - for stdin")
    p.add_argument("--access-id")
    p.add_argument("--access-secret-file", help="file with the Access service-token secret, or - for stdin")
    p.add_argument("--work-dir", default="kfhost-box")
    p.add_argument("--dry-run", action="store_true")
    p.add_argument("run_flags", nargs="*", help="after --: flags for `kfhost run` (e.g. --exit-when-idle 50)")
    return p


def plan_lines(a, work):
    return [
        "kfhost bootstrap will, inside %s only:" % work,
        "  1. git clone %s at commit %s into %s/checkout and refuse unless HEAD equals it" % (a.repo, a.commit, work),
        "  2. create the venv %s/venv with `python3 -m kfhost setup --venv` (pip, hash-locked; no system packages)" % work,
        "  3. write %s/config.json with mode 0600 (secrets read from files/stdin, never argv)" % work,
        "  4. exec `kfhost run --config %s/config.json %s` as this user" % (work, " ".join(a.run_flags)),
    ]


def default_git(args, cwd):
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": str(cwd), "GIT_TERMINAL_PROMPT": "0",
           "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
    done = subprocess.run(["git", "-c", "http.followRedirects=false", *args], cwd=cwd, env=env,
                          capture_output=True, text=True, timeout=900, check=False)
    if done.returncode != 0:
        raise BootstrapError("git %s failed: %s" % (args[0], done.stderr.strip()[:200]))
    return done.stdout.strip()


def checkout(repo, commit, dest, git=default_git):
    dest.mkdir(parents=True, exist_ok=True)
    git(["init", "-q"], dest)
    git(["remote", "add", "origin", repo], dest)
    git(["fetch", "-q", "--depth=1", "origin", commit], dest)
    git(["checkout", "-q", "--detach", commit], dest)
    head = git(["rev-parse", "HEAD"], dest)
    if head != commit:
        raise BootstrapError("refusing: HEAD %s is not the pinned commit %s" % (head, commit))


def write_config(path, cfg):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(cfg, f)
    os.chmod(path, 0o600)


def main(argv=None, run=subprocess.run, execv=os.execv, stdin=sys.stdin, out=print, git=default_git):
    argv = list(sys.argv[1:] if argv is None else argv)
    a = parser().parse_args(argv)
    try:
        if not HEX40.fullmatch(a.commit):
            raise BootstrapError("--commit must be 40 lowercase hex")
        if not a.repo.startswith("https://"):
            raise BootstrapError("--repo must be an https URL")
        if not (a.access_id and a.access_secret_file):  # the daemon config requires the Access pair
            raise BootstrapError("--access-id and --access-secret-file are required")
        if a.token_file == "-" and a.access_secret_file == "-":
            raise BootstrapError("only one secret may come from stdin")
        work = Path(a.work_dir).resolve()
        for line in plan_lines(a, work):
            out(line)
        if a.dry_run:
            out("dry run: nothing was changed")
            return 0
        token = read_secret(a.token_file, stdin)  # read before any change so a bad file costs nothing
        secret = read_secret(a.access_secret_file, stdin)
        if (work / "checkout").exists() or (work / "config.json").exists():
            raise BootstrapError("%s already holds a checkout or config; use a fresh --work-dir" % work)
        work.mkdir(mode=0o700, parents=True, exist_ok=True)
        checkout(a.repo, a.commit, work / "checkout", git)
        scripts = work / "checkout" / "scripts"
        venv = work / "venv"
        setup = [sys.executable, "-m", "kfhost", "setup", "--venv", str(venv)]
        out("+ " + " ".join(setup))
        if run(setup, cwd=scripts, check=False).returncode != 0:
            raise BootstrapError("kfhost setup failed; nothing was started")
        cfg = {"worker_url": a.worker_url, "box_id": a.box_id, "box_token": token,
               "access_id": a.access_id, "access_secret": secret, "work_dir": str(work / "jobs")}
        write_config(work / "config.json", cfg)
        flags = [x for x in a.run_flags if x != "--"]
        cmd = [str(venv / "bin" / "python"), "-m", "kfhost", "run", "--config", str(work / "config.json"), *flags]
        out("+ " + " ".join(cmd))
        os.chdir(scripts)
        execv(cmd[0], cmd)
        return 0
    except (BootstrapError, OSError) as error:
        print("bootstrap: " + str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
