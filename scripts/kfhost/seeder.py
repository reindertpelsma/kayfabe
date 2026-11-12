"""prepare_seeder: make nginx current only where system changes are allowed, else only report drift.

System changes (apt) happen ONLY for effective profile `owned` AND euid 0. container, borrowed and non-root
never change packages: they run the `python3 -m kfseed check` equivalent (installed vs candidate) and warn.
`run` is an injectable command runner: run(argv) -> (rc, text); rc None when the program is absent.
"""

import shutil
import subprocess

PKG = "nginx-light"


def default_run(argv, timeout=600):
    try:
        done = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, check=False)
    except (OSError, subprocess.SubprocessError):
        return None, ""
    return done.returncode, (done.stdout + done.stderr).strip()


def nginx_version(run):
    rc, text = run(["nginx", "-v"])
    return text.strip() if rc == 0 and text else None


def prepare_seeder(profile, euid, run=default_run, which=shutil.which):
    """Return {"available": bool, "warnings": [str], "log": [str]}. Never raises."""
    log, warnings = [], []
    old = nginx_version(run)
    log.append("seeder: nginx before: %s" % (old or "missing"))
    if profile == "owned" and euid == 0:
        mgr = "apt-get" if which("apt-get") else None
        if mgr is None:
            log.append("seeder: no apt-get (dnf/apk not handled); package left unchanged")
            warnings.append("nginx not updated: no supported package manager")
        else:
            steps = [["apt-get", "update"], ["apt-get", "install", "-y", "--only-upgrade", PKG]]
            if old is None:
                steps[1] = ["apt-get", "install", "-y", PKG]
            for argv in steps:
                rc, text = run(argv)
                log.append("seeder: %s -> rc=%s" % (" ".join(argv), rc))
                if rc != 0:
                    log.append("seeder: apt failed: " + (text or "")[-200:])
                    warnings.append("nginx update failed")
                    break
            new = nginx_version(run)
            log.append("seeder: nginx after: %s" % (new or "missing"))
            if new is None:
                log.append("seeder: nginx missing and not installable: seeding unavailable")
                return {"available": False, "warnings": warnings, "log": log}
            return {"available": True, "warnings": warnings, "log": log}
    else:
        rc, text = run(["python3", "-m", "kfseed", "check"])
        if rc is None or rc not in (0, 1):
            warnings.append("kfseed check unavailable; nginx currency unknown")
        elif rc != 0:
            warnings.append("nginx drifted from candidate: " + (text or "")[:200])
        log.append("seeder: no system changes allowed (profile %s, euid %s); kfseed check rc=%s" % (profile, euid, rc))
    if old is None:
        log.append("seeder: nginx missing and may not be installed here: seeding unavailable")
        return {"available": False, "warnings": warnings, "log": log}
    return {"available": True, "warnings": warnings, "log": log}
