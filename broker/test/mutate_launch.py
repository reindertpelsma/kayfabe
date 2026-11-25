#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0 OR Apache-2.0
"""Mutation check for the start-up decisions (nb_launch.c): break the source on
purpose, one defect at a time, and demand that test-launch notices -- at least
TWO distinct failing checks per mutant (one failing check is a coincidence; two
is a net).

Each mutant is an exact textual replacement in a COPY of nb_launch.c (the real
file is never touched).  A mutant whose text is not found exactly once is itself
an error, so the table cannot rot silently when the code moves.

    python3 test/mutate_launch.py         # all mutants
    python3 test/mutate_launch.py -v      # also print the failing checks
"""
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.dirname(HERE)

# (name, old text, new text)
MUTANTS = [
    # --- the Wayland socket path ---------------------------------------------
    ("no flag still builds a path (libwayland's own environment is bypassed)",
     "    if (!flag_name && !flag_runtime_dir) {\n        return 0;",
     "    if (!flag_name && !flag_runtime_dir && 0) {\n        return 0;"),
    ("only --wayland-display counts as a flag (--runtime-dir alone is ignored)",
     "    if (!flag_name && !flag_runtime_dir) {",
     "    if (!flag_name) {"),
    ("the environment name beats the flag",
     "name = flag_name ? flag_name\n                     : (env_name && *env_name ? env_name : \"wayland-0\");",
     "name = (env_name && *env_name) ? env_name\n                     : (flag_name ? flag_name : \"wayland-0\");"),
    ("the default name is not wayland-0",
     ": (env_name && *env_name ? env_name : \"wayland-0\");",
     ": (env_name && *env_name ? env_name : \"wayland-1\");"),
    ("the environment directory beats --runtime-dir",
     "dir = flag_runtime_dir ? flag_runtime_dir : env_runtime_dir;",
     "dir = env_runtime_dir ? env_runtime_dir : flag_runtime_dir;"),
    ("an absolute name is joined to the directory",
     "    if (name[0] == '/') {", "    if (0) {"),
    ("a relative name with no directory is guessed instead of refused",
     "        if (!dir || !*dir) {\n            return NB_WL_NO_RUNTIME_DIR;\n        }",
     "        if (!dir) {\n            dir = \"\";\n        }"),
    ("an empty directory is a directory",
     "        if (!dir || !*dir) {", "        if (!dir) {"),
    ("an empty name is accepted",
     "    if (!*name) {\n        return NB_WL_EMPTY_NAME;\n    }", ""),
    ("an empty WAYLAND_DISPLAY is a name",
     "(env_name && *env_name ? env_name : \"wayland-0\")",
     "(env_name ? env_name : \"wayland-0\")"),
    ("the length check is off by one (a path filling sun_path)",
     "(size_t)n >= sizeof(((struct sockaddr_un *)0)->sun_path)",
     "(size_t)n > sizeof(((struct sockaddr_un *)0)->sun_path)"),
    ("the caller's buffer size is not checked",
     "if (n < 0 || (size_t)n >= cap ||", "if (n < 0 ||"),
    ("a too-long path leaves its text in the output",
     "        if (cap) {\n            out[0] = '\\0';\n        }\n        return NB_WL_TOO_LONG;",
     "        return NB_WL_TOO_LONG;"),
    # --- the socket directory ---------------------------------------------------
    ("a file is accepted as the socket directory",
     "    if (!S_ISDIR(st_mode)) {\n        return NB_DIR_NOT_DIR;\n    }", ""),
    ("any owner is trusted",
     "    if (!owner_ok) {\n        return NB_DIR_FOREIGN_OWNER;\n    }", ""),
    ("root is not trusted",
     "bool owner_ok = st_uid == self || st_uid == 0;",
     "bool owner_ok = st_uid == self;"),
    ("only the first allowed uid is trusted",
     "for (i = 0; i < n_trusted && !owner_ok; i++) {",
     "for (i = 0; i < n_trusted && i < 1 && !owner_ok; i++) {"),
    ("the allow list is not consulted",
     "for (i = 0; i < n_trusted && !owner_ok; i++) {",
     "for (i = 0; i < 0 && !owner_ok; i++) {"),
    ("another trusted owner may leave the directory open",
     "    if (st_uid != self && (st_mode & 0022) && !(st_mode & S_ISVTX)) {\n        return NB_DIR_FOREIGN_WIDE;\n    }",
     ""),
    ("group write is not looked at",
     "(st_mode & 0022) && !(st_mode & S_ISVTX)", "(st_mode & 0002) && !(st_mode & S_ISVTX)"),
    ("other write is not looked at",
     "(st_mode & 0022) && !(st_mode & S_ISVTX)", "(st_mode & 0020) && !(st_mode & S_ISVTX)"),
    ("the sticky bit does not excuse a shared directory",
     "(st_mode & 0022) && !(st_mode & S_ISVTX)", "(st_mode & 0022)"),
    ("our own directory is judged like a stranger's",
     "    if (st_uid != self && (st_mode & 0022)", "    if ((st_mode & 0022)"),
    ("a created directory is never 0700",
     "    unsigned m = 0700;", "    unsigned m = 0755;"),
    ("a group socket mode adds no search bit",
     "    if (socket_mode & 0060) {\n        m |= 0010;\n    }", ""),
    ("an other socket mode adds no search bit",
     "    if (socket_mode & 0006) {\n        m |= 0001;\n    }", ""),
    ("a group socket mode adds write too",
     "        m |= 0010;", "        m |= 0070;"),
    # --- SUDO_UID and --allow-uid ---------------------------------------------------
    ("--allow-uid does not replace SUDO_UID",
     "if (self_uid != 0 || allow_uid_flag || !sudo_uid_env) {",
     "if (self_uid != 0 || !sudo_uid_env) {"),
    ("a non-root broker reads SUDO_UID",
     "if (self_uid != 0 || allow_uid_flag || !sudo_uid_env) {",
     "if (allow_uid_flag || !sudo_uid_env) {"),
    ("SUDO_UID's value is not the uid added",
     "*out = (uid_t)strtoul(sudo_uid_env, NULL, 10);", "*out = 0;"),
    ("--allow-uid accepts (uid_t)-1",
     "if (v > 4294967294ull) {", "if (v > 4294967295ull) {"),
    ("--allow-uid does not stop at overflow",
     "        if (v > 4294967294ull) {\n            return -ERANGE;     /* (uid_t)-1 is \"no uid\" to chown() */\n        }",
     ""),
    ("--allow-uid accepts the empty string",
     "    if (!s || !*s) {\n        return -EINVAL;\n    }", "    if (!s) {\n        return -EINVAL;\n    }"),
    # --- flag/backend conflicts -----------------------------------------------------
    ("--wayland-display is allowed with --backend x11",
     "    if (cfg->wayland_display && !wl_ok) {", "    if (0) {"),
    ("--x11-display is allowed with --backend wayland",
     "    if (cfg->x11_display && !x_ok) {", "    if (0) {"),
    ("--runtime-dir is allowed with --backend x11",
     "    if (cfg->runtime_dir && !wl_ok) {", "    if (0) {"),
    ("--xauthority is allowed with --backend wayland",
     "    if (cfg->xauthority && !x_ok) {", "    if (0) {"),
    ("the test backend takes the wayland flags",
     "bool wl_ok = strcmp(be, \"x11\") != 0 && strcmp(be, \"test\") != 0;",
     "bool wl_ok = strcmp(be, \"x11\") != 0;"),
    ("the test backend takes the x11 flags",
     "bool x_ok  = strcmp(be, \"wayland\") != 0 && strcmp(be, \"test\") != 0;",
     "bool x_ok  = strcmp(be, \"wayland\") != 0;"),
    ("auto refuses the display flags",
     "bool wl_ok = strcmp(be, \"x11\") != 0 &&", "bool wl_ok = strcmp(be, \"auto\") != 0 && strcmp(be, \"x11\") != 0 &&"),
]


def build_and_run(workdir):
    cc = os.environ.get("CC", "cc")
    exe = os.path.join(workdir, "t")
    cmd = [cc, "-O1", "-g", "-std=gnu11", "-w", "-D_GNU_SOURCE", "-I", workdir,
           "-I", os.path.join(workdir, "common"), "-o", exe,
           os.path.join(workdir, "test_launch.c"), os.path.join(workdir, "nb_launch.c")]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        return None, r.stderr
    try:
        r = subprocess.run([exe], capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return ["TIMEOUT"], ""
    fails = [l for l in r.stderr.splitlines() if l.startswith("FAIL ")]
    if r.returncode != 0 and not fails:
        fails = ["CRASH rc=%d" % r.returncode]
    return fails, ""


def main():
    verbose = "-v" in sys.argv
    bad = 0
    print("launch mutation check: %d mutants" % len(MUTANTS))
    for i, (name, old, new) in enumerate(MUTANTS, 1):
        with tempfile.TemporaryDirectory(prefix="mutlaunch") as tmp:
            for f in ("nvkvm_broker.h", "nb_launch.c"):
                shutil.copy(os.path.join(SRC, f), tmp)
            shutil.copytree(os.path.join(SRC, "common"), os.path.join(tmp, "common"))
            shutil.copy(os.path.join(HERE, "test_launch.c"), tmp)
            path = os.path.join(tmp, "nb_launch.c")
            text = open(path).read()
            if text.count(old) != 1:
                print("  ERROR  #%d %s: mutation text found %d times"
                      % (i, name, text.count(old)))
                bad += 1
                continue
            open(path, "w").write(text.replace(old, new))
            fails, err = build_and_run(tmp)
            if fails is None:
                print("  ERROR  #%d %s: does not compile\n%s" % (i, name, err))
                bad += 1
                continue
            distinct = {f.split(": ", 1)[-1] for f in fails}
            status = "killed" if len(distinct) >= 2 else "WEAK  " if distinct else "ALIVE "
            if len(distinct) < 2:
                bad += 1
            print("  %s #%d %s (%d failing checks)" % (status, i, name, len(distinct)))
            if verbose:
                for f in sorted(distinct):
                    print("           ", f)
    print("%s" % ("all mutants killed twice over" if not bad else "%d mutant(s) not killed twice" % bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
