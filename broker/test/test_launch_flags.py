#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0 OR Apache-2.0
"""The broker's start-up flags (--wayland-display, --runtime-dir, --x11-display,
--xauthority, --allow-uid) and the socket directory rules, black-box, with no
display server and no GPU.

A compositor is not needed to prove WHICH socket the broker connects to: a
listening AF_UNIX socket (Wayland) or abstract socket (X11) stands in for it,
and what matters is who gets the connection.  Each flag test carries a DECOY in
the environment (the variable the flag replaces, pointing at a second listener)
and demands that the decoy is never touched.  Each fallback test is the same
with no flag, and demands the environment still works.

The socket-directory cases that need a second user (an owner we do not trust)
run when this process is root, and print SKIP otherwise: the pure decision they
exercise is covered, for every owner, in test/test_launch.c.

    python3 test/test_launch_flags.py ./nvkvm-display-broker
"""
import os
import select
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time

BROKER = None
FAILS = []
CHECKS = [0]


def check(cond, msg):
    CHECKS[0] += 1
    if not cond:
        FAILS.append(msg)
        print("FAIL", msg, file=sys.stderr)


def clean_env(**extra):
    env = {k: v for k, v in os.environ.items()
           if k not in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET", "XDG_RUNTIME_DIR",
                        "DISPLAY", "XAUTHORITY", "SUDO_UID", "LISTEN_FDS",
                        "LISTEN_PID")}
    env.update(extra)
    return env


def run_to_exit(args, env, timeout=5):
    """Run the broker until it exits by itself; return (rc, stderr)."""
    p = subprocess.run([BROKER] + args, env=env, stdin=subprocess.DEVNULL,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                       text=True, timeout=timeout)
    return p.returncode, p.stderr


def unix_listener(path):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.bind(path)
    s.listen(4)
    return s


def accepted(listener, wait):
    r, _, _ = select.select([listener], [], [], wait)
    if not r:
        return None
    conn, _ = listener.accept()
    return conn


def display_flag_run(args, env, listener, decoys, want=True):
    """Start the broker, expect `listener` to get the connection (or not), and
    expect no decoy to.  The connection is closed, which ends the broker."""
    p = subprocess.Popen([BROKER] + args, env=env, stdin=subprocess.DEVNULL,
                         stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                         text=True)
    try:
        conn = accepted(listener, 5 if want else 1.0) if listener else None
        data = b""
        if conn:
            conn.settimeout(1)
            try:
                data = conn.recv(64)     # X11: the setup request; Wayland: none
            except (socket.timeout, OSError):
                pass
            conn.close()
        for d in decoys:
            check(accepted(d, 0.3) is None,
                  "a decoy (the environment) was connected to: %s" % args)
        try:
            p.wait(timeout=5)
        except subprocess.TimeoutExpired:
            p.kill()
            p.wait()
            check(False, "broker did not exit after the connection was closed: %s" % args)
        err = p.stderr.read()
    finally:
        if p.poll() is None:
            p.kill()
            p.wait()
        p.stderr.close()
    return conn is not None, data, err


# --- Wayland ----------------------------------------------------------------

def test_wayland(tmp):
    rt = os.path.join(tmp, "rt"); os.mkdir(rt)
    decoy_rt = os.path.join(tmp, "decoy"); os.mkdir(decoy_rt)
    real = unix_listener(os.path.join(rt, "wayland-test"))
    decoy = unix_listener(os.path.join(decoy_rt, "wayland-decoy"))
    sock = os.path.join(tmp, "b.sock")
    base = ["--backend", "wayland", "--socket", sock]
    decoy_env = clean_env(WAYLAND_DISPLAY="wayland-decoy",
                          XDG_RUNTIME_DIR=decoy_rt, WAYLAND_SOCKET="99")

    # name + directory flags beat WAYLAND_DISPLAY, XDG_RUNTIME_DIR, WAYLAND_SOCKET
    ok, _, err = display_flag_run(
        base + ["--wayland-display", "wayland-test", "--runtime-dir", rt],
        decoy_env, real, [decoy])
    check(ok, "--wayland-display NAME --runtime-dir DIR did not reach DIR/NAME:\n" + err)

    # an absolute name needs no runtime directory at all (empty environment)
    ok, _, err = display_flag_run(
        base + ["--wayland-display", os.path.join(rt, "wayland-test")],
        clean_env(), real, [decoy])
    check(ok, "an absolute --wayland-display did not connect:\n" + err)
    # ... and still beats the decoy variables
    ok, _, err = display_flag_run(
        base + ["--wayland-display", os.path.join(rt, "wayland-test")],
        decoy_env, real, [decoy])
    check(ok, "an absolute --wayland-display lost to the environment:\n" + err)

    # --runtime-dir alone: the NAME is still WAYLAND_DISPLAY's
    ok, _, err = display_flag_run(
        base + ["--runtime-dir", rt], clean_env(WAYLAND_DISPLAY="wayland-test"),
        real, [decoy])
    check(ok, "--runtime-dir alone did not look WAYLAND_DISPLAY up in DIR:\n" + err)

    # the environment fallback still works with no flag at all
    ok, _, err = display_flag_run(
        base, clean_env(WAYLAND_DISPLAY="wayland-test", XDG_RUNTIME_DIR=rt),
        real, [decoy])
    check(ok, "no flag: WAYLAND_DISPLAY + XDG_RUNTIME_DIR no longer work:\n" + err)

    # a relative name with nowhere to look: refused, named, nothing connected
    rc, err = run_to_exit(base + ["--wayland-display", "wayland-test"], clean_env())
    check(rc != 0 and "--runtime-dir" in err and "relative" in err,
          "a relative name without a directory must be refused by name:\n" + err)

    # a missing socket names the path it tried
    rc, err = run_to_exit(
        base + ["--wayland-display", "nope", "--runtime-dir", rt], clean_env())
    check(rc != 0 and os.path.join(rt, "nope") in err,
          "a missing Wayland socket must be named in the error:\n" + err)

    # auto + --wayland-display: only the flagged backend is tried (no X11 fall-through)
    ok, _, err = display_flag_run(
        ["--backend", "auto", "--socket", sock, "--wayland-display",
         os.path.join(rt, "wayland-test")],
        clean_env(DISPLAY=":599"), real, [])
    check(ok and "--wayland-display was given" in err and "trying the X11" not in err,
          "auto with --wayland-display must try Wayland only:\n" + err)
    real.close(); decoy.close()


# --- X11 ---------------------------------------------------------------------

def x11_listener(n):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.bind("\0/tmp/.X11-unix/X%d" % n)
    s.listen(4)
    return s


def xauth_file(path, display, cookie):
    host = socket.gethostname().encode()
    num = str(display).encode()
    name = b"MIT-MAGIC-COOKIE-1"
    def f(b):
        return len(b).to_bytes(2, "big") + b
    with open(path, "wb") as fh:
        fh.write((256).to_bytes(2, "big") + f(host) + f(num) + f(name) + f(cookie))


def test_x11(tmp):
    n_real = 700 + os.getpid() % 150
    n_decoy = n_real + 150
    real = x11_listener(n_real)
    decoy = x11_listener(n_decoy)
    wl_dir = os.path.join(tmp, "wlrt"); os.mkdir(wl_dir)
    wl_decoy = unix_listener(os.path.join(wl_dir, "wayland-decoy"))
    sock = os.path.join(tmp, "x.sock")
    base = ["--backend", "x11", "--socket", sock]

    ok, data, err = display_flag_run(
        base + ["--x11-display", ":%d" % n_real],
        clean_env(DISPLAY=":%d" % n_decoy), real, [decoy])
    check(ok and data[:1] == b"l", "--x11-display did not reach its display:\n" + err)

    ok, _, err = display_flag_run(
        base, clean_env(DISPLAY=":%d" % n_real), real, [decoy])
    check(ok, "no flag: DISPLAY no longer works:\n" + err)

    # auto: a display flag names its backend; WAYLAND_DISPLAY is not a reason
    # to open a Wayland window
    ok, _, err = display_flag_run(
        ["--backend", "auto", "--socket", sock, "--x11-display", ":%d" % n_real],
        clean_env(WAYLAND_DISPLAY="wayland-decoy", XDG_RUNTIME_DIR=wl_dir),
        real, [decoy, wl_decoy])
    check(ok and "--x11-display was given" in err,
          "auto with --x11-display must try X11 only:\n" + err)
    # and with no flag, auto still prefers the environment's Wayland
    ok, _, err = display_flag_run(
        ["--backend", "auto", "--socket", sock],
        clean_env(WAYLAND_DISPLAY="wayland-decoy", XDG_RUNTIME_DIR=wl_dir),
        wl_decoy, [])
    check(ok and "WAYLAND_DISPLAY is set" in err,
          "no flag: auto no longer prefers WAYLAND_DISPLAY:\n" + err)

    # --xauthority: the cookie of THIS file is the one sent, not XAUTHORITY's
    good = os.path.join(tmp, "xa-good"); bad = os.path.join(tmp, "xa-bad")
    xauth_file(good, n_real, b"G" * 16)
    xauth_file(bad, n_real, b"B" * 16)
    ok, data, err = display_flag_run(
        base + ["--x11-display", ":%d" % n_real, "--xauthority", good],
        clean_env(XAUTHORITY=bad), real, [decoy])
    check(ok and b"MIT-MAGIC-COOKIE-1" in data and b"G" * 16 in data
          and b"B" * 16 not in data,
          "--xauthority did not decide the cookie sent: %r" % data)
    # the fallback: no flag, XAUTHORITY still read
    ok, data, err = display_flag_run(
        base + ["--x11-display", ":%d" % n_real], clean_env(XAUTHORITY=bad),
        real, [decoy])
    check(ok and b"B" * 16 in data,
          "no --xauthority: XAUTHORITY no longer read: %r" % data)
    real.close(); decoy.close(); wl_decoy.close()


# --- conflicts ----------------------------------------------------------------

def test_conflicts(tmp):
    sock = os.path.join(tmp, "c.sock")
    for backend, flag, val in (("x11", "--wayland-display", "w"),
                               ("wayland", "--x11-display", ":0"),
                               ("test", "--wayland-display", "w"),
                               ("test", "--x11-display", ":0"),
                               ("wayland", "--xauthority", "/x"),
                               ("x11", "--runtime-dir", "/rt")):
        rc, err = run_to_exit(["--backend", backend, "--socket", sock, flag, val],
                              clean_env())
        check(rc == 2 and flag in err and "no effect" in err,
              "%s with --backend %s must be refused by name (rc=%s):\n%s"
              % (flag, backend, rc, err))
    rc, err = run_to_exit(["--backend", "auto", "--socket", sock, "--x11-display", "nocolon"],
                          clean_env())
    check(rc == 2 and "not an X display" in err, "a malformed --x11-display:\n" + err)


# --- the test backend: allow-uid and the socket directory ----------------------

class Live:
    """A broker on the test backend, up once it says it is listening."""

    def __init__(self, args, env, umask=None):
        def pre():
            if umask is not None:
                os.umask(umask)
        self.p = subprocess.Popen([BROKER, "--backend", "test"] + args, env=env,
                                  stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
                                  stderr=subprocess.PIPE, text=True,
                                  preexec_fn=pre)
        self.err = ""
        os.set_blocking(self.p.stderr.fileno(), False)
        deadline = time.time() + 5
        while time.time() < deadline and "allowed uid" not in self.err \
                and self.p.poll() is None:
            select.select([self.p.stderr], [], [], 0.1)
            try:
                self.err += self.p.stderr.read() or ""
            except (BlockingIOError, TypeError):
                pass
        time.sleep(0.1)
        try:
            self.err += self.p.stderr.read() or ""
        except (BlockingIOError, TypeError):
            pass

    def stop(self):
        if self.p.poll() is None:
            self.p.send_signal(signal.SIGTERM)
        try:
            self.p.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.p.kill(); self.p.wait()
        self.p.stdin.close(); self.p.stderr.close()


def allowed(err):
    return sorted(int(l.rsplit(" ", 1)[1]) for l in err.splitlines()
                  if l.strip().startswith("allowed uid") or "allowed uid" in l
                  and l.split("allowed uid")[1].strip().isdigit())


def test_allow_uid(tmp):
    sock = os.path.join(tmp, "u.sock")
    me = os.getuid()
    b = Live(["--socket", sock, "--allow-uid", "4242"], clean_env())
    got = allowed(b.err); b.stop()
    check(4242 in got and 0 in got and me in got,
          "--allow-uid adds to root and the invoking user: %s\n%s" % (got, b.err))
    b = Live(["--socket", sock, "--allow-uid", "4242", "--allow-uid", "4243"],
             clean_env())
    got = allowed(b.err); b.stop()
    check(4242 in got and 4243 in got, "--allow-uid is repeatable: %s" % got)
    for bad in ("abc", "-1", "4294967295", "", "1x"):
        rc, err = run_to_exit(["--socket", sock, "--backend", "test",
                               "--allow-uid", bad], clean_env())
        check(rc == 2 and "--allow-uid" in err, "--allow-uid %r must be refused:\n%s" % (bad, err))
    if me == 0:
        b = Live(["--socket", sock], clean_env(SUDO_UID="777"))
        got = allowed(b.err); b.stop()
        check(777 in got, "fallback: root + SUDO_UID adds that uid: %s" % got)
        b = Live(["--socket", sock, "--allow-uid", "4242"], clean_env(SUDO_UID="777"))
        got = allowed(b.err); b.stop()
        check(4242 in got and 777 not in got,
              "--allow-uid must replace SUDO_UID: %s" % got)
    else:
        print("SKIP: SUDO_UID is only read by a root broker (decision pinned in test_launch.c)")
        b = Live(["--socket", sock], clean_env(SUDO_UID="777"))
        got = allowed(b.err); b.stop()
        check(777 not in got, "a non-root broker must never read SUDO_UID: %s" % got)


def mode(path):
    return stat.S_IMODE(os.stat(path).st_mode)


def test_socket_dir(tmp):
    # created, 0700, even under a permissive umask; the socket is in it
    d = os.path.join(tmp, "kayfabe")
    b = Live(["--socket", os.path.join(d, "gpu0.sock")], clean_env(), umask=0)
    check(os.path.isdir(d) and mode(d) == 0o700,
          "the socket directory must be created 0700, got %s\n%s"
          % (oct(mode(d)) if os.path.isdir(d) else "nothing", b.err))
    check(os.path.exists(os.path.join(d, "gpu0.sock")), "the socket is not in the new directory")
    check("created the socket directory" in b.err, "creation is not announced:\n" + b.err)
    b.stop()

    # an existing directory of ours is left exactly as it is
    d2 = os.path.join(tmp, "mine"); os.mkdir(d2); os.chmod(d2, 0o755)
    b = Live(["--socket", os.path.join(d2, "s.sock")], clean_env())
    check(mode(d2) == 0o755 and "created the socket directory" not in b.err,
          "an existing directory must not be touched: %s" % oct(mode(d2)))
    check(os.path.exists(os.path.join(d2, "s.sock")), "no socket in the existing directory:\n" + b.err)
    b.stop()

    # a group socket mode adds search for the group to a NEW directory only
    d3 = os.path.join(tmp, "grp")
    b = Live(["--socket", os.path.join(d3, "s.sock"), "--socket-mode", "0660"],
             clean_env(), umask=0)
    check(mode(d3) == 0o710, "--socket-mode 0660: new directory must be 0710, got %s"
          % oct(mode(d3)))
    b.stop()

    # only ONE level is created
    deep = os.path.join(tmp, "a", "b", "s.sock")
    rc, err = run_to_exit(["--backend", "test", "--socket", deep], clean_env())
    check(rc != 0 and not os.path.exists(os.path.join(tmp, "a")) and "one level" in err,
          "two missing levels must be refused, not created:\n" + err)

    # the parent is a file
    f = os.path.join(tmp, "afile"); open(f, "w").close()
    rc, err = run_to_exit(["--backend", "test", "--socket", os.path.join(f, "s.sock")],
                          clean_env())
    check(rc != 0 and "not a directory" in err, "a file as parent:\n" + err)

    # a stale non-socket is still never touched
    os.makedirs(os.path.join(tmp, "stale"), mode=0o700)
    victim = os.path.join(tmp, "stale", "s.sock"); open(victim, "w").write("x")
    rc, err = run_to_exit(["--backend", "test", "--socket", victim], clean_env())
    check(rc != 0 and os.path.exists(victim) and "refusing to touch" in err,
          "a regular file at the socket path is refused as before:\n" + err)

    if os.getuid() != 0:
        print("SKIP: directories owned by another user need root to build "
              "(pure decision covered in test_launch.c)")
        return
    # foreign owner, 0700: refused by name; trusted by --allow-uid
    other = 65534
    d4 = os.path.join(tmp, "foreign"); os.mkdir(d4); os.chown(d4, other, other)
    os.chmod(d4, 0o700)
    rc, err = run_to_exit(["--backend", "test", "--socket", os.path.join(d4, "s.sock")],
                          clean_env())
    check(rc != 0 and "owned by uid %d" % other in err and "Refusing" in err
          and not os.path.exists(os.path.join(d4, "s.sock")),
          "a directory owned by an untrusted uid must be refused:\n" + err)
    b = Live(["--socket", os.path.join(d4, "s.sock"), "--allow-uid", str(other)],
             clean_env())
    check(os.path.exists(os.path.join(d4, "s.sock")),
          "the sudo shape (root broker, the user's own directory, --allow-uid) must work:\n" + b.err)
    b.stop()
    # foreign + writable by others: refused even for an allowed uid
    d5 = os.path.join(tmp, "wide"); os.mkdir(d5); os.chown(d5, other, other)
    os.chmod(d5, 0o777)
    rc, err = run_to_exit(["--backend", "test", "--socket", os.path.join(d5, "s.sock"),
                           "--allow-uid", str(other)], clean_env())
    check(rc != 0 and "writable by group or others" in err
          and not os.path.exists(os.path.join(d5, "s.sock")),
          "a wide-open directory of another user must be refused:\n" + err)
    # the same, unrelated owner: refused as untrusted
    rc, err = run_to_exit(["--backend", "test", "--socket", os.path.join(d5, "s.sock")],
                          clean_env())
    check(rc != 0 and "owned by uid" in err, "wide-open + untrusted owner:\n" + err)


def main():
    global BROKER
    if len(sys.argv) != 2:
        raise SystemExit("usage: test_launch_flags.py /path/to/broker")
    BROKER = os.path.abspath(sys.argv[1])
    have = subprocess.run([BROKER, "--help"], capture_output=True, text=True).stderr
    with tempfile.TemporaryDirectory(prefix="nbflags") as tmp:
        os.chmod(tmp, 0o755)
        for name, fn in (("conflicts", test_conflicts), ("allow-uid", test_allow_uid),
                         ("socket-dir", test_socket_dir)):
            sub = os.path.join(tmp, name); os.mkdir(sub); os.chmod(sub, 0o755)
            fn(sub)
        # the display tests need the backend compiled in
        for name, fn, probe in (("wayland", test_wayland, "wayland"),
                                ("x11", test_x11, "x11")):
            sub = os.path.join(tmp, name); os.mkdir(sub); os.chmod(sub, 0o755)
            r = subprocess.run([BROKER, "--backend", probe, "--socket",
                                os.path.join(sub, "p.sock")], env=clean_env(),
                               capture_output=True, text=True, timeout=5)
            if "unknown --backend" in r.stderr or "not compiled" in r.stderr:
                print("SKIP: %s backend not built" % name)
                continue
            fn(sub)
    print("%d checks, %d failed" % (CHECKS[0], len(FAILS)))
    sys.exit(1 if FAILS else 0)


if __name__ == "__main__":
    main()
