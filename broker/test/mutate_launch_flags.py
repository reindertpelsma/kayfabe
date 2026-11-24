#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0 OR Apache-2.0
"""Mutation check for the WIRING of the start-up flags (the call sites of
nb_launch.c): break the broker on purpose, one defect at a time, rebuild it, and
demand that test/test_launch_flags.py notices -- at least TWO failing checks per
mutant where the test can see it twice, and at least one always.

This is the companion of mutate_launch.py, which breaks the pure decisions.  A
decision that is right but not CALLED (a flag parsed and then ignored, the
environment read before the flag) is what this one is for.

Each mutant is an exact textual replacement in a COPY of the built broker
directory (the real tree is never touched; only the one changed object and the
link are rebuilt).  A mutant whose text is not found exactly once is an error.
A mutant marked root-only is skipped (and said so) when this process is not
root, because the case that sees it needs a second uid.

    python3 test/mutate_launch_flags.py [-v]     # needs a built ./nvkvm-display-broker
"""
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.dirname(HERE)

# (name, file, old, new, min_failing, root_only)
MUTANTS = [
    ("the Wayland flags are not passed on (libwayland reads the environment)",
     "nb_session_wl.c", "cfg->wayland_display, cfg->runtime_dir,", "NULL, NULL,", 2, False),
    ("a resolved Wayland path is not connected to (WAYLAND_SOCKET and the environment win)",
     "nb_session_wl.c", "        if (path[0]) {\n            struct sockaddr_un sa;", "        if (0) {\n            struct sockaddr_un sa;", 2, False),
    ("the Wayland path is built from the environment's name first",
     "nb_session_wl.c", "cfg->wayland_display, cfg->runtime_dir,", "getenv(\"WAYLAND_DISPLAY\"), cfg->runtime_dir,", 1, False),
    ("a relative Wayland name with no directory is not refused",
     "nb_session_wl.c", "        if (rc == NB_WL_NO_RUNTIME_DIR) {", "        if (0) {", 1, False),
    ("a failed Wayland connect does not name the path",
     "nb_session_wl.c", "nb_err(\"cannot connect to the Wayland socket %s: %s\", path,", "nb_err(\"cannot connect to the Wayland socket %s: %s\", \"\",", 1, False),
    ("--x11-display is not passed to xcb_connect",
     "nb_session_x11.c", "x->c = xcb_connect(cfg->x11_display, &screen_n);", "x->c = xcb_connect(NULL, &screen_n);", 2, False),
    ("--xauthority is not applied",
     "nb_session_x11.c", "if (cfg->xauthority && setenv(", "if (0 && cfg->xauthority && setenv(", 1, False),
    ("--xauthority is applied even when it was not given",
     "nb_session_x11.c", "if (cfg->xauthority && setenv(\"XAUTHORITY\", cfg->xauthority, 1) != 0) {", "if (setenv(\"XAUTHORITY\", cfg->xauthority ? cfg->xauthority : \"/nonexistent\", 1) != 0) {", 1, False),
    ("auto lets the environment's Wayland win over --x11-display",
     "nb_common.c", "(!flagged && getenv(\"WAYLAND_DISPLAY\") != NULL)", "(getenv(\"WAYLAND_DISPLAY\") != NULL)", 1, False),
    ("auto lets the environment's DISPLAY in next to --wayland-display",
     "nb_common.c", "(!flagged && getenv(\"DISPLAY\") != NULL)", "(getenv(\"DISPLAY\") != NULL)", 1, False),
    ("auto ignores --wayland-display and trusts only the environment",
     "nb_common.c", "bool have_wl = cfg->wayland_display ||\n                   (!flagged && getenv(\"WAYLAND_DISPLAY\") != NULL);", "bool have_wl = getenv(\"WAYLAND_DISPLAY\") != NULL;", 1, False),
    ("auto never reads the environment any more",
     "nb_common.c", "(!flagged && getenv(\"WAYLAND_DISPLAY\") != NULL)", "(0)", 1, False),
    ("the flag/backend conflict is not checked",
     "nvkvm_broker.c", "const char *bad = nb_display_flag_conflict(&cfg);", "const char *bad = NULL;", 2, False),
    ("a malformed --x11-display is not refused",
     "nvkvm_broker.c", "if (cfg.x11_display && !strchr(cfg.x11_display, ':')) {", "if (0) {", 1, False),
    ("--allow-uid does not mark itself (SUDO_UID is still read)",
     "nvkvm_broker.c", "cfg.allow_uid_flag = true; }", "}", 1, True),
    ("SUDO_UID is no longer a fallback",
     "nvkvm_broker.c", "            cfg.n_allow_uid < NB_MAX_ALLOW_IDS) {\n            cfg.allow_uid[cfg.n_allow_uid++] = sudo_uid;", "            0) {\n            cfg.allow_uid[cfg.n_allow_uid++] = sudo_uid;", 1, True),
    ("--allow-uid is parsed but not added",
     "nvkvm_broker.c", "cfg.allow_uid[cfg.n_allow_uid++] = u;\n            cfg.allow_uid_flag = true;", "cfg.allow_uid_flag = true;", 2, False),
    ("--allow-uid takes any text",
     "nvkvm_broker.c", "if (nb_parse_uid(v, &u) != 0) {", "if (nb_parse_uid(v, &u) != 0 && 0) {", 1, False),
    ("the socket directory is not prepared at all",
     "nvkvm_broker.c", "int rc = nb_prepare_socket_dir(path, cfg);", "int rc = 0;", 2, False),
    ("a missing socket directory is not created",
     "nvkvm_broker.c", "        if (mkdir(dir, 0700) < 0) {", "        if (0) {", 1, False),
    ("the created directory is left with the umask's mode",
     "nvkvm_broker.c", "        if (chmod(dir, (mode_t)mode) < 0) {\n            nb_err(\"chmod %04o %s: %s\", mode, dir, strerror(errno));\n            return -errno;\n        }",
     "        (void)mode;", 1, False),
    ("the created directory is 0755",
     "nvkvm_broker.c", "mode = nb_sockdir_create_mode(cfg->socket_mode);", "mode = 0755;", 1, False),
    ("the created directory ignores --socket-mode",
     "nvkvm_broker.c", "mode = nb_sockdir_create_mode(cfg->socket_mode);", "mode = nb_sockdir_create_mode(0600);", 1, False),
    ("an existing directory is not judged",
     "nvkvm_broker.c", "    v = nb_sockdir_verdict(st.st_mode, st.st_uid, geteuid(), cfg->allow_uid,\n                           cfg->n_allow_uid);", "    v = NB_DIR_OK;", 1, False),
    ("the allow list is not used to trust an owner",
     "nvkvm_broker.c", "geteuid(), cfg->allow_uid,\n                           cfg->n_allow_uid);", "geteuid(), cfg->allow_uid,\n                           0);", 1, True),
    ("a refused directory is only warned about",
     "nvkvm_broker.c", "    return -EACCES;\n}\n\nstatic int nb_listen", "    return 0;\n}\n\nstatic int nb_listen", 1, False),
]


def main():
    verbose = "-v" in sys.argv
    bad = 0
    is_root = os.getuid() == 0
    if not os.path.exists(os.path.join(SRC, "nvkvm-display-broker")):
        raise SystemExit("build the broker first: make")
    print("launch-flags mutation check: %d mutants" % len(MUTANTS))
    for i, (name, fname, old, new, need, root_only) in enumerate(MUTANTS, 1):
        if root_only and not is_root:
            print("  SKIP   #%d %s (needs root)" % (i, name))
            continue
        with tempfile.TemporaryDirectory(prefix="mutflags") as tmp:
            work = os.path.join(tmp, "broker")
            subprocess.run(["cp", "-a", SRC, work], check=True)
            for stale in ("test-launch",):
                if os.path.exists(os.path.join(work, stale)):
                    os.unlink(os.path.join(work, stale))
            path = os.path.join(work, fname)
            text = open(path).read()
            if text.count(old) != 1:
                print("  ERROR  #%d %s: mutation text found %d times" % (i, name, text.count(old)))
                bad += 1
                continue
            open(path, "w").write(text.replace(old, new))
            b = subprocess.run(["make", "-s", "nvkvm-display-broker"], cwd=work,
                               capture_output=True, text=True)
            if b.returncode != 0:
                print("  ERROR  #%d %s: does not compile\n%s" % (i, name, b.stderr[-800:]))
                bad += 1
                continue
            try:
                r = subprocess.run([sys.executable, os.path.join(work, "test", "test_launch_flags.py"),
                                    os.path.join(work, "nvkvm-display-broker")],
                                   capture_output=True, text=True, timeout=180)
                fails = [l for l in r.stderr.splitlines() if l.startswith("FAIL ")]
                if r.returncode != 0 and not fails:
                    fails = ["CRASH rc=%d\n%s" % (r.returncode, r.stderr[-300:])]
            except subprocess.TimeoutExpired:
                fails = ["TIMEOUT"]
            n = len(fails)
            ok = n >= need
            if not ok:
                bad += 1
            print("  %s #%d %s (%d failing checks, need %d)"
                  % ("killed" if ok else ("WEAK  " if n else "ALIVE "), i, name, n, need))
            if verbose:
                for f in fails:
                    print("           ", f.splitlines()[0][:150])
    print("all mutants killed" if not bad else "%d mutant(s) not killed enough" % bad)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
