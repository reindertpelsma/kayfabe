#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The `install_flow` payload stage: the documented user flow, run on a CI GPU box (stdlib only).

STATUS: LIVE as code, 2026-11-15. First GPU-box run 2026-11-15 (release shakeout, RTX 4090, sha 84b541cae): it failed three times on harness
defects (the mirror clone of the blob-less Worker checkout never finished, the mirror directory was not named `kayfabe`, the cup8 log
directory did not exist), all fixed in this file, and then ended PASS in 258 s (see docs/howto/building.md, "How the commands on this
page are tested"). OWNER_RULINGS section AS: the user flow is "compile QEMU (or use the attested binary),
then use the VM with your own tools with a kf3 GPU device", and the exact commands we tell users to run are tested on the
boxes that run for CI. Section AR: run as an unprivileged uid in group kvm with no capabilities; root steps are explicit.

What it does, in order (the table is PLAN below; every marker in the docs must be in PLAN or in NOT_RUN):
  1. root setup (py): a fresh unprivileged user, a prefix owned by it, a local mirror of the exact commit under test;
  2. the `kf-run` blocks of docs/howto/building.md, read from the doc by scripts/ci/doc_steps.py, as that user, one root
     step (`packages_install`) excepted;
  3. root setup (py): the guest image, the ssh key and the device nodes readable by that user;
  4. the `kf-args` block of docs/howto/attach-to-existing-vm.md, plus a throwaway VM's disk and network arguments, boots the
     provisioned Linux guest as that user; the CUDA probe cup8 (scripts/bench/cup8.c of the tested commit) runs in it.
The result (per-step rc and uid, which steps needed root, the sha256 of every doc consulted, the substitutions) is one JSON
line `KF_INSTALL_FLOW_RESULT {...}` and the file given by --out; vast_payload.py grades it.

  install_flow.py plan [--checkout DIR]              print the plan; run nothing; touch nothing
  install_flow.py run --sha SHA --checkout DIR --prefix DIR --out FILE [--attested-binary PATH]

ATTESTED BINARY (docs/design/V3_BOX_DOWNLOADER_AND_SUPPLY_CHAIN.md section 2, DESIGN-ONLY). The interface is `resolve_binary()`:
given the commit it returns a verified binary path or None (compile). Today it always returns None and `--attested-binary`
is REFUSED (exit 2, `attested_not_implemented`): the download-and-`gh attestation verify` client is a later step, and an
unverified path must never be accepted in its place.
"""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import doc_steps  # noqa: E402

SCHEMA = 1
DOCS = (
    "building.md",
    "installing.md",
    "requirements.md",
    "attach-to-existing-vm.md",
    "linux-guest.md",
)
USER = "kfinstall"
MARK = ".kf-install-flow"
UUID = "6b1d2a40-6f3e-4c3a-9a55-0f1e2d3c4b5a"
FB_PLACEHOLDER = "fb-mb=8192"
UUID_PLACEHOLDER = "<a fixed uuid for this VM>"
REPO_URL = "https://github.com/reindertpelsma/kayfabe.git"
GUEST_IP = "192.168.77.2"
CUP8_PASS = re.compile(r"^CUP8 RESULT N=2048 bad=0 maxerr=0 C\[0\]=\S+\(exp \S+\) -> PASS$", re.M)
KF3_BUILT = re.compile(r"^KF3_BUILT (/\S+) rev=([0-9a-f]{8}) ", re.M)
KF3_HOST_LINE = re.compile(r"^(?:\[[^\]]*\] )?kf3: host GPU minor=\d+ ", re.M)

# (kind, doc, step): the order the stage runs. "doc" blocks come from the Markdown; "py" actions are this file's.
PLAN = (
    ("py", None, "root_setup"),
    ("doc", "building.md", "clone"),
    ("doc", "building.md", "packages_list"),
    ("doc", "building.md", "packages_install"),
    ("doc", "building.md", "rust_toolchain"),
    ("doc", "building.md", "workspace_build"),
    ("doc", "building.md", "qemu_build"),
    ("doc", "building.md", "raw_client"),
    ("py", None, "guest_access"),
    ("doc", "attach-to-existing-vm.md", "qemu_args"),
    ("py", None, "boot_and_probe"),
)
PY_USER = {"root_setup": "root", "guest_access": "root", "boot_and_probe": "unpriv"}
# marked steps the stage deliberately does not run, with the reason; empty today
NOT_RUN = {}


class FlowError(Exception):
    def __init__(self, reason):
        self.reason = reason
        super().__init__(reason)


def resolve_binary(sha, attested_path=None):
    """The attested-binary hook. TODO (later step, not this one): download the CI-built binary for `sha`, run
    `gh attestation verify` against the pinned workflow identity, check its sha256 against the attestation, and return
    its path; until then there is nothing to trust, so a supplied path is refused and the answer is "compile"."""
    if attested_path:
        raise FlowError("attested_not_implemented")
    return None


def load_docs(checkout):
    """{name: Parsed} for every doc in DOCS, read from the tree under test."""
    base = Path(checkout) / "docs/howto"
    return {name: doc_steps.load(base / name, name) for name in DOCS}


def check_plan(docs):
    """Every marked step is planned or excused, and every planned doc step exists. Raises FlowError otherwise."""
    marked = {(name, s.name) for name, parsed in docs.items() for s in parsed.steps}
    planned = {(d, s) for kind, d, s in PLAN if kind == "doc"}
    if marked - planned - set(NOT_RUN):
        raise FlowError("unplanned_marker")
    if planned - marked:
        raise FlowError("planned_step_missing")
    if len({s for _, _, s in PLAN}) != len(PLAN):
        raise FlowError("plan_duplicate")


def find(docs, doc, step):
    for s in docs[doc].steps:
        if s.name == step:
            return s
    raise FlowError("planned_step_missing")


def doc_hashes(docs):
    return {name: parsed.sha256 for name, parsed in docs.items()}


def plan_text(docs):
    out = []
    for kind, doc, step in PLAN:
        if kind == "doc":
            s = find(docs, doc, step)
            out.append(f"KF_INSTALL_FLOW_PLAN {kind} {doc} {step} {s.kind} user={s.user} timeout={s.timeout}")
        else:
            out.append(f"KF_INSTALL_FLOW_PLAN {kind} - {step} - user={PY_USER[step]}")
    for name, parsed in docs.items():
        out.append(f"KF_INSTALL_FLOW_DOC {name} sha256={parsed.sha256} steps={len(parsed.steps)} fences={parsed.fences}")
    return out


# ------------------------------------------------------------ pure helpers

def fb_mb_for(vram_mib):
    """attach-to-existing-vm.md, table row `fb-mb`: under 10 GiB use VRAM minus 2048 MiB in whole GiB."""
    if vram_mib < 10240:
        value = (vram_mib - 2048) // 1024 * 1024
        if value < 1024:
            raise FlowError("card_too_small")
        return value
    return 8192


def substitutions(mirror, vram):
    """The whole documented substitution list (and nothing else is replaced): the clone URL, the VM's UUID placeholder, and on
    a card under 10 GiB the `fb-mb` value the attach doc's own table prescribes."""
    subs = [
        doc_steps.Substitution("clone", REPO_URL, "file://" + str(mirror)),
        doc_steps.Substitution("qemu_args", UUID_PLACEHOLDER, UUID),
    ]
    fb = fb_mb_for(vram)
    if fb != 8192:
        subs.append(doc_steps.Substitution("qemu_args", FB_PLACEHOLDER, f"fb-mb={fb}"))
    return subs


def drop_argv(uid, gid, kvm_gid):
    """setpriv: that uid and gid, supplementary group of /dev/kvm only, every capability set emptied, no_new_privs."""
    for n in (uid, gid, kvm_gid):
        if not isinstance(n, int) or n <= 0:
            raise FlowError("drop_identity")
    return [
        "setpriv", "--reuid", str(uid), "--regid", str(gid), "--groups", str(kvm_gid),
        "--inh-caps=-all", "--ambient-caps=-all", "--bounding-set=-all", "--no-new-privs",
    ]


def qemu_command(qemu, words, overlay, serial, port):
    """The documented argument block, then the disk and network of a throwaway VM (the doc: take your VM's command line
    and make these changes). Nothing documented is dropped or reordered."""
    return [
        str(qemu), *words,
        "-smp", "3", "-display", "none", "-no-reboot", "-msg", "timestamp=on",
        "-serial", f"file:{serial}",
        "-drive", f"if=virtio,file={overlay},format=qcow2",
        "-netdev", f"user,id=n0,net=192.168.77.0/24,host=192.168.77.1,hostfwd=tcp:127.0.0.1:{port}-{GUEST_IP}:22",
        "-device", "virtio-net-pci,netdev=n0,mac=52:54:00:12:34:56",
    ]


def grade_cup8(text):
    return bool(CUP8_PASS.search(text))


def built_binary(log_text, sha, prefix):
    """(path, rev) from build_kf3.sh's own KF3_BUILT line; the rev must be the commit under test and the path inside the prefix."""
    found = KF3_BUILT.findall(log_text)
    if len(found) != 1:
        raise FlowError("kf3_built_line")
    path, rev = found[0]
    if rev != sha[:8] or not doc_steps.inside(prefix, path):
        raise FlowError("kf3_built_identity")
    return path, rev


def result_doc(sha, docs, runner, subs, binary, guest, verdict, reason=None, root_setup=()):
    doc = {
        "schema": SCHEMA, "verdict": verdict, "sha": sha, "docs": doc_hashes(docs),
        "steps": [{k: r[k] for k in ("doc", "step", "user", "rc", "ms") if k in r}
                  | ({"uid": r["uid"], "cap_eff": r["cap_eff"]} if "uid" in r else {}) for r in runner.records],
        "root_steps": runner.root_steps() + list(root_setup),
        "substitutions": [{"step": s.step, "literal": s.literal} for s in subs],
        "binary": binary, "guest": guest,
    }
    if reason:
        doc["reason"] = reason
    return doc


# ------------------------------------------------------------ box actions (root and unprivileged)

def sh(argv, **kw):
    return subprocess.run(argv, check=False, **kw)


def prepare_prefix(prefix):
    prefix = Path(prefix)
    if not re.fullmatch(r"/[A-Za-z0-9_./-]+", str(prefix)) or str(prefix) == "/":
        raise FlowError("prefix")
    if prefix.exists():
        if not (prefix / MARK).exists() and any(prefix.iterdir()):
            raise FlowError("prefix_not_ours")
        shutil.rmtree(prefix)
    prefix.mkdir(parents=True)
    (prefix / MARK).write_text("kf install flow\n")
    return prefix.resolve()


def ensure_user(prefix):
    """The unprivileged user. Returns (uid, gid, kvm_gid). An existing account is reused only if it is not root."""
    import pwd

    try:
        entry = pwd.getpwnam(USER)
    except KeyError:
        if sh(["useradd", "-M", "-d", str(prefix / "home"), "-s", "/bin/bash", USER]).returncode:
            raise FlowError("useradd") from None
        entry = pwd.getpwnam(USER)
    if entry.pw_uid == 0:
        raise FlowError("user_is_root")
    return entry.pw_uid, entry.pw_gid, os.stat("/dev/kvm").st_gid


def has_missing_objects(rev_list_output):
    """`git rev-list --objects --missing=print` marks an object the store lacks with a leading `?`."""
    return any(line.startswith("?") for line in rev_list_output.splitlines())


def mirror_source(checkout, sha):
    """Where the mirror is cloned from: the checkout when it holds every object of `sha`, else the public repository.
    The Worker's /workspace/kf-ci-src is a depth-1, blob-less, sparse clone (18k blobs absent); a bare clone of it never finished
    (measured 2026-11-15, one box: git pack-objects at 94 percent CPU for 13+ minutes, killed), a bare clone of the public URL took 25 s."""
    listing = sh(["git", "-c", "safe.directory=" + str(checkout), "-C", str(checkout), "rev-list", "--objects", "--missing=print", sha],
                 capture_output=True, text=True, timeout=300)
    if listing.returncode == 0 and not has_missing_objects(listing.stdout):
        return str(checkout)
    return REPO_URL


def make_mirror(checkout, prefix, sha, uid, gid):
    """A bare repository holding the commit under test as `main`; the documented clone URL is replaced by its file URL."""
    mirror = prefix / REPO_URL.rsplit("/", 1)[1]  # `git clone <URL>` names its directory after the URL, and the docs then `cd kayfabe`
    try:
        if sh(["git", "clone", "--bare", "--quiet", mirror_source(checkout, sha), str(mirror)], timeout=900).returncode:
            raise FlowError("mirror_clone")
        if sh(["git", f"--git-dir={mirror}", "cat-file", "-e", sha + "^{commit}"]).returncode and sh(
            ["git", f"--git-dir={mirror}", "fetch", "--quiet", "origin", sha], timeout=900
        ).returncode:
            raise FlowError("mirror_commit_missing")
    except subprocess.TimeoutExpired:
        raise FlowError("mirror_timeout") from None
    for argv in (["update-ref", "refs/heads/main", sha], ["symbolic-ref", "HEAD", "refs/heads/main"]):
        if sh(["git", f"--git-dir={mirror}", *argv]).returncode:
            raise FlowError("mirror_ref")
    return mirror


def chown_tree(path, uid, gid):
    for root, dirs, files in os.walk(path):
        for name in (*dirs, *files):
            os.lchown(os.path.join(root, name), uid, gid)
    os.lchown(path, uid, gid)


def vram_mib():
    p = sh(["nvidia-smi", "--query-gpu=memory.total", "--format=csv,noheader,nounits"], capture_output=True, text=True)
    values = [int(x) for x in p.stdout.split() if x.isdigit()]
    if p.returncode or not values:
        raise FlowError("vram_unknown")
    return min(values)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_gpu_idle():
    for _ in range(60):
        p = sh(["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits"], capture_output=True, text=True)
        used = [int(x) for x in p.stdout.split() if x.isdigit()]
        if used and max(used) <= 512:
            return
        time.sleep(0.5)


def guest_access(prefix, uid, gid, image, key):
    """Root step: the user's own copy of the ssh key, read access to the guest image and its folders, the device nodes.
    Returns the names of the root actions taken (recorded in the result as root steps)."""
    key_copy = prefix / "guest_key"
    shutil.copyfile(key, key_copy)
    os.chmod(key_copy, 0o600)
    os.chown(key_copy, uid, gid)
    actions = ["copy_ssh_key", "device_nodes"]
    for d in (image.parent, *image.parents):
        if d == Path("/"):
            break
        mode = d.stat().st_mode
        if not mode & 0o001:
            os.chmod(d, mode | 0o001)
            actions.append("traverse_" + d.name)
    if not image.stat().st_mode & 0o004:
        os.chmod(image, image.stat().st_mode | 0o004)
        actions.append("image_readable")
    sh(["nvidia-modprobe", "-c", "0", "-u"])  # documented: creates the nodes if they are missing
    return key_copy, actions


def cup8_log(prefix):
    """The probe's own output goes beside the documented steps' logs (doc_steps.Runner puts those under <prefix>/run/logs)."""
    logs = Path(prefix) / "run" / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    return logs / "cup8.out"


def probe_guest(prefix, drop, qemu, words, image, key, clone, log):
    """Boot, ssh in, nvidia-smi, cup8, power off. Returns the guest record; raises FlowError on a failed check."""
    env = {"HOME": str(prefix / "home"), "PATH": doc_steps.DEFAULT_PATH, "TMPDIR": str(prefix / "tmp")}
    overlay = prefix / "overlay.qcow2"
    r = sh([*drop, "qemu-img", "create", "-q", "-f", "qcow2", "-b", str(image), "-F", "qcow2", str(overlay)], env=env)
    if r.returncode:
        raise FlowError("overlay")
    port = free_port()
    qlog = prefix / "qemu.log"
    cmd = qemu_command(qemu, words, overlay, prefix / "guest_serial.log", port)
    ssh = [*drop, "ssh", "-i", str(key), "-p", str(port), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
           "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=5", "-o", "ServerAliveInterval=10", "-o", "ServerAliveCountMax=6",
           "ubuntu@127.0.0.1"]
    wait_gpu_idle()
    guest = {"qemu_args_source": "attach-to-existing-vm.md", "cup8": "NOT_RUN", "nvidia_smi_rc": None, "kf3_log_line": False}
    log("KF_DOCSTEP_START doc=- step=boot_and_probe user=unpriv timeout=1800")
    started = time.monotonic()
    with open(qlog, "wb") as out:
        child = subprocess.Popen([*drop, *cmd], env=env, stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT)
    try:
        up = False
        while time.monotonic() - started < 600:
            if child.poll() is not None:
                raise FlowError("qemu_exited")
            if sh([*ssh, "true"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
                up = True
                break
            time.sleep(5)
        if not up:
            raise FlowError("guest_ssh_timeout")
        guest["boot_s"] = int(time.monotonic() - started)
        guest["kf3_log_line"] = bool(KF3_HOST_LINE.search(qlog.read_text(errors="replace")))
        if not guest["kf3_log_line"]:
            raise FlowError("no_kf3_log_line")
        smi = sh([*ssh, "nvidia-smi"], env=env, capture_output=True, text=True, timeout=120)
        guest["nvidia_smi_rc"] = smi.returncode
        if smi.returncode:
            raise FlowError("guest_nvidia_smi")
        src = (clone / "scripts/bench/cup8.c").read_bytes()
        if sh([*ssh, "cat > /tmp/cup8.c"], env=env, input=src).returncode:
            raise FlowError("push_cup8")
        if sh([*ssh, "gcc -O0 -o /tmp/cup8 /tmp/cup8.c -lcuda -lm"], env=env, capture_output=True).returncode:
            raise FlowError("build_cup8")
        run = sh([*ssh, "cd /tmp && timeout 900 ./cup8"], env=env, capture_output=True, text=True, timeout=1000)
        cup8_log(prefix).write_text(run.stdout)
        guest["cup8"] = "PASS" if (run.returncode == 0 and grade_cup8(run.stdout)) else "FAIL"
        if guest["cup8"] != "PASS":
            raise FlowError("cup8_failed")
        sh([*ssh, "sudo poweroff"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            child.wait(timeout=120)
        except subprocess.TimeoutExpired:
            pass
    finally:
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=30)
            except subprocess.TimeoutExpired:
                child.kill()
        log(f"KF_DOCSTEP_END doc=- step=boot_and_probe rc={0 if guest['cup8'] == 'PASS' else 1} ms={int((time.monotonic() - started) * 1000)}")
    return guest


# ------------------------------------------------------------ the run

def run(args, log=print):
    sha, checkout = args.sha, Path(args.checkout).resolve()
    if not re.fullmatch("[0-9a-f]{40}", sha):
        raise FlowError("sha")
    docs = load_docs(checkout)
    check_plan(docs)
    resolve_binary(sha, args.attested_binary)
    if os.geteuid() != 0:
        raise FlowError("not_root")
    prefix = prepare_prefix(args.prefix)
    uid, gid, kvm_gid = ensure_user(prefix)
    mirror = make_mirror(checkout, prefix, sha, uid, gid)
    drop = drop_argv(uid, gid, kvm_gid)
    subs = substitutions(mirror, vram_mib())
    chown_tree(prefix, uid, gid)
    os.chmod(prefix, 0o755)
    runner = doc_steps.Runner(prefix / "run", drop=drop, require_unpriv=True, log=log)
    chown_tree(prefix, uid, gid)
    root_setup = ["root_setup"]
    binary = {"source": "compiled"}
    guest = {"cup8": "NOT_RUN"}
    words = []
    verdict, reason = "FAIL", None
    clone = runner.work / "kayfabe"
    key = binary_path = None
    try:
        for kind, doc, step in PLAN:
            if kind == "doc":
                s, _used = doc_steps.substitute(find(docs, doc, step), subs, prefix)
                if s.kind == "args":
                    words = doc_steps.args_words(s)
                    log(f"KF_DOCSTEP_ARGS doc={doc} step={step} words={len(words)}")
                    continue
                runner.run(s)
                if step == "clone":
                    head = sh(["git", "-c", "safe.directory=" + str(clone), "-C", str(clone), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
                    if head != sha:
                        raise FlowError("clone_not_the_commit")
                if step == "qemu_build":
                    text = (runner.logs / f"{runner.counter:02d}-qemu_build.log").read_text(errors="replace")
                    binary_path, rev = built_binary(text, sha, prefix)
                    binary.update(rev=rev)
            elif step == "guest_access":
                key, more = guest_access(prefix, uid, gid, Path(args.guest_image), Path(args.ssh_key))
                root_setup += more
            elif step == "boot_and_probe":
                guest = probe_guest(prefix, drop, binary_path, words, Path(args.guest_image), key, clone, log)
        verdict = "PASS"
    except (FlowError, doc_steps.DocStepError) as error:
        reason = error.reason if isinstance(error, FlowError) else "step_failed"
        log("KF_INSTALL_FLOW_FAIL reason=" + reason)
    return result_doc(sha, docs, runner, subs, binary, guest, verdict, reason, root_setup)


def parse_args(argv):
    p = argparse.ArgumentParser(prog="install_flow.py", description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    pl = sub.add_parser("plan")
    pl.add_argument("--checkout", default=str(Path(__file__).resolve().parents[2]))
    r = sub.add_parser("run")
    r.add_argument("--sha", required=True)
    r.add_argument("--checkout", required=True)
    r.add_argument("--prefix", required=True)
    r.add_argument("--out", required=True)
    r.add_argument("--attested-binary", default=None)
    r.add_argument("--guest-image", default="/workspace/bench/guest.qcow2")
    r.add_argument("--ssh-key", default="/workspace/bench/guest_key")
    return p.parse_args(argv)


def main(argv=None):
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        if args.cmd == "plan":
            docs = load_docs(args.checkout)
            check_plan(docs)
            print("\n".join(plan_text(docs)))
            return 0
        result = run(args)
    except (FlowError, doc_steps.DocStepError) as error:
        print("KF_INSTALL_FLOW_REFUSED reason=" + (error.reason if isinstance(error, FlowError) else "doc_refused " + str(error)[:160]), flush=True)
        return 2
    text = json.dumps(result, separators=(",", ":"), sort_keys=True)
    Path(args.out).write_text(text + "\n")
    print("KF_INSTALL_FLOW_RESULT " + text, flush=True)
    return 0 if result["verdict"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
