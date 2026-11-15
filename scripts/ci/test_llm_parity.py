"""2026-11-15 (OWNER_RULINGS section AS item 8): the llm_parity payload stage, GPU-free.

FALSIFIER (written before the code): an unpinned item is downloaded or run instead of refused by name; a hostile path (comma,
space, newline, `..`, a symlink, a writable directory) reaches a QEMU option or the daemon's argument list; the shared
directory is exported writable, with DAX, or by a daemon that keeps a capability; the ratio decides the verdict (a guest at 10
percent of native fails, or one at 150 percent passes only because of it); a zero, negative or NaN timing yields a ratio;
two arms that print different text pass; an arm that kept part of the model in system memory passes; a guest arm's malformed
line is believed; the stage is in a default plan, or is refused when named by --only; the record can carry a gate.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import stat
import subprocess
import sys
import tarfile
import tempfile
import textwrap
import unittest
from unittest.mock import patch

import llm_parity as lp
import llm_parity_arm as arm
import payload_instruction as pi
import vast_report

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SHA = "a" * 40
H = "1" * 64
H2 = "2" * 64

FAKE_OLLAMA = textwrap.dedent('''\
    #!/usr/bin/env python3
    import http.server, json, os, sys
    cfg = json.load(open(os.path.join(os.environ["HOME"], "..", "..", "fake.json")))
    if sys.argv[1] == "create":
        text = open(sys.argv[sys.argv.index("-f") + 1]).read()
        sys.exit(0 if text.startswith("FROM /") and "PARAMETER seed 1" in text and not cfg.get("create_fails") else 5)
    json.dump(dict(os.environ), open(os.path.join(os.environ["HOME"], "..", "..", "env.json"), "w"))
    if cfg.get("die"):
        sys.exit(3)
    host, port = os.environ["OLLAMA_HOST"].split(":")
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a): pass
        def send_doc(self, doc):
            b = json.dumps(doc).encode()
            self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
        def do_GET(self):
            if self.path == "/api/version": self.send_doc({"version": cfg.get("version", "0.0.1")})
            else: self.send_doc({"models": [{"name": "kf-parity", "size": cfg.get("size", 1000), "size_vram": cfg.get("vram", 1000)}]})
        def do_POST(self):
            self.rfile.read(int(self.headers["Content-Length"]))
            doc = {"response": cfg.get("text", "hello"), "eval_count": cfg.get("eval_count", 128), "eval_duration": cfg.get("eval_ns", 10**10),
                   "prompt_eval_count": 12, "total_duration": 2 * 10**10, "load_duration": 3 * 10**6}
            if cfg.get("context", True): doc["context"] = cfg.get("context_ids", [1, 2, 3])
            self.send_doc(doc)
    http.server.HTTPServer((host, int(port)), Handler).serve_forever()
''')


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def make_tar(path, binary=FAKE_OLLAMA, extra=(), name="bin/ollama"):
    with tarfile.open(path, "w:gz") as tar:
        if binary is not None:
            data = binary.encode()
            info = tarfile.TarInfo(name)
            info.size, info.mode = len(data), 0o755
            import io
            tar.addfile(info, io.BytesIO(data))
        for info, data in extra:
            import io
            tar.addfile(info, io.BytesIO(data) if data is not None else None)


def tinfo(name, kind=tarfile.REGTYPE, link="", size=0, mode=0o644):
    i = tarfile.TarInfo(name)
    i.type, i.linkname, i.size, i.mode = kind, link, size, mode
    return i


def arm_doc(label="host", **over):
    """A valid arm result (what llm_parity_arm.py prints), 3 measured runs of 128 tokens in 10 s."""
    runs = [{"eval_count": 128, "eval_duration_ns": 10**10, "prompt_eval_count": 12, "total_duration_ns": 2 * 10**10,
             "text_sha256": H, "token_ids_sha256": H2} for _ in range(3)]
    doc = {"schema": 1, "label": label, "ollama_version": "0.0.1",
           "inputs": {"model": {"bytes": 100_000_000, "read_secs": 0.5, "sha256": H}, "ollama": {"bytes": 200, "read_secs": 0.25, "sha256": H2}},
           "create_secs": 1.5, "load_ms": 800.0, "params": {"num_predict": 128, "seed": 1, "repeats": 3, "prompt_sha256": H},
           "warmup_text_sha256": H, "runs": runs, "text": "hello", "ps": {"size": 1000, "size_vram": 1000}, "gpu": "Fake GPU, 1.0"}
    doc.update(over)
    return doc


def arm_text(doc):
    return "noise line\nKF_LLM_ARM_RESULT " + json.dumps(doc) + "\n"


def pinned(**over):
    pins = json.loads((HERE / "llm_parity_pins.json").read_text())
    pins["ollama_version"] = "0.0.1"
    for n, (sha, size) in {"model": (H, 100_000_000), "ollama": (H2, 200), "virtiofsd": ("3" * 64, 300)}.items():
        pins["items"][n].update(sha256=sha, size=size, sources=[{"kind": "mirror", "url": "https://mirror.example.org/" + n}])
    pins.update(over)
    return pins


def duplicate(rec):
    return json.loads(json.dumps(rec))


class Pins(unittest.TestCase):
    def test_the_repository_pins_file_is_valid_and_every_hash_is_unset(self):
        pins = lp.load_pins()
        self.assertEqual(pins["ollama_version"], "UNSET")
        self.assertEqual({n: i["sha256"] for n, i in pins["items"].items()}, {"model": "UNSET", "ollama": "UNSET", "virtiofsd": "UNSET"})
        self.assertEqual(lp.unpinned(pins, lp.ITEM_NAMES), ["model", "ollama", "ollama_version", "virtiofsd"])

    def test_the_unpinned_repository_manifest_is_refused_by_name(self):
        with self.assertRaises(lp.StageError) as cm:
            lp.build_manifest(lp.load_pins(), lp.TRANSPORT_ITEMS["virtiofs"])
        self.assertEqual(cm.exception.reason, "unpinned")
        self.assertIn("model", cm.exception.detail)

    def test_one_unpinned_item_is_enough_to_refuse(self):
        pins = pinned()
        pins["items"]["ollama"].update(sha256="UNSET", size="UNSET")
        with self.assertRaises(lp.StageError) as cm:
            lp.build_manifest(pins, ("model", "ollama"))
        self.assertEqual((cm.exception.reason, cm.exception.detail), ("unpinned", "ollama"))

    def test_an_unset_ollama_version_refuses_even_with_every_hash_pinned(self):
        with self.assertRaises(lp.StageError) as cm:
            lp.build_manifest(pinned(ollama_version="UNSET"), ("model", "ollama"))
        self.assertEqual(cm.exception.detail, "ollama_version")

    def test_the_9p_transport_does_not_need_virtiofsd(self):
        pins = pinned()
        pins["items"]["virtiofsd"].update(sha256="UNSET", size="UNSET", sources=[])
        self.assertEqual([i["name"] for i in lp.build_manifest(pins, lp.TRANSPORT_ITEMS["9p"])["items"]], ["model", "ollama"])
        with self.assertRaises(lp.StageError):
            lp.build_manifest(pins, lp.TRANSPORT_ITEMS["virtiofs"])

    def test_a_pinned_manifest_is_the_downloaders_own_format_and_validates_there(self):
        import box_downloader as bd

        doc = lp.build_manifest(pinned(), lp.TRANSPORT_ITEMS["virtiofs"])
        manifest = bd.parse_manifest(json.dumps(doc).encode())
        self.assertEqual(doc["schema"], "kayfabe.download-manifest/1")
        self.assertEqual([(i.name, i.size, i.sha256) for i in manifest.items],
                         [("model", 100_000_000, H), ("ollama", 200, H2), ("virtiofsd", 300, "3" * 64)])

    def test_the_downloader_refuses_a_pin_with_no_source(self):
        pins = pinned()
        pins["items"]["model"]["sources"] = []
        with self.assertRaises(lp.StageError) as cm:
            lp.build_manifest(pins, ("model", "ollama"))
        self.assertEqual(cm.exception.reason, "pins_invalid")

    def test_a_dest_that_leaves_the_directory_is_refused_by_the_downloader(self):
        pins = pinned()
        pins["items"]["model"]["dest"] = "../escape"
        with self.assertRaises(lp.StageError):
            lp.build_manifest(pins, ("model", "ollama"))

    def test_half_pinned_item_is_refused(self):
        for key, value in (("sha256", "UNSET"), ("size", "UNSET")):
            pins = pinned()
            pins["items"]["model"][key] = value
            with self.assertRaises(ValueError):
                lp.check_pins(pins)

    def test_the_loader_refuses_a_half_pinned_file(self):
        with tempfile.TemporaryDirectory() as d:
            good = (HERE / "llm_parity_pins.json").read_text()
            half = good.replace('"sha256": "UNSET",\n      "size": "UNSET"', '"sha256": "%s",\n      "size": "UNSET"' % H, 1)
            self.assertNotEqual(half, good)
            (Path(d) / "p.json").write_text(half)
            with self.assertRaises(lp.StageError) as cm:
                lp.load_pins(Path(d) / "p.json")
            self.assertEqual(cm.exception.reason, "pins_invalid")

    def test_a_non_hex_or_uppercase_hash_in_a_file_is_refused_by_the_loader(self):
        with tempfile.TemporaryDirectory() as d:
            for bad in ("G" * 64, "A" * 64, "1" * 63):
                pins = pinned()
                pins["items"]["model"]["sha256"] = bad
                (Path(d) / "p.json").write_text(json.dumps(pins))
                with self.assertRaises(lp.StageError, msg=bad):
                    lp.load_pins(Path(d) / "p.json")

    def test_bad_hash_size_and_shape_are_refused(self):
        for mutate in (lambda p: p["items"]["model"].update(sha256="A" * 64), lambda p: p["items"]["model"].update(sha256="1" * 63),
                       lambda p: p["items"]["model"].update(size=0), lambda p: p["items"]["model"].update(size=True),
                       lambda p: p["items"]["model"].update(size=-5), lambda p: p["items"].pop("model"),
                       lambda p: p.update(extra=1), lambda p: p.update(schema="x"), lambda p: p.update(ollama_version="a b"),
                       lambda p: p["items"]["model"].update(extra=1)):
            pins = pinned()
            mutate(pins)
            with self.assertRaises(ValueError):
                lp.check_pins(pins)

    def test_the_pins_loader_refuses_nan_duplicates_and_junk(self):
        with tempfile.TemporaryDirectory() as d:
            good = (HERE / "llm_parity_pins.json").read_text()
            for name, text in (("nan", good.replace('"UNSET",\n  "model_id"', "NaN,\n  \"model_id\"", 1)), ("dup", good.replace('"schema"', '"schema": 1, "schema"', 1)),
                               ("junk", "not json"), ("big", " " * 70000)):
                p = Path(d) / name
                p.write_text(text)
                with self.assertRaises(lp.StageError) as cm:
                    lp.load_pins(p)
                self.assertEqual(cm.exception.reason, "pins_invalid", name)

    def test_the_cli_check_exits_3_while_unpinned_and_plan_exits_0(self):
        ck = subprocess.run([sys.executable, str(HERE / "llm_parity.py"), "check"], capture_output=True, text=True)
        self.assertEqual(ck.returncode, 3)
        self.assertIn("KF_LLM_PARITY_REFUSED reason=unpinned items=model,ollama,ollama_version,virtiofsd", ck.stdout)
        pl = subprocess.run([sys.executable, str(HERE / "llm_parity.py"), "plan"], capture_output=True, text=True)
        self.assertEqual(pl.returncode, 0)
        self.assertIn("gate=none", pl.stdout)


class ShareArguments(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = os.path.realpath(self.tmp.name)
        self.share = os.path.join(self.root, "share")
        os.mkdir(self.share, 0o755)
        os.chmod(self.share, 0o755)
        self.sock = os.path.join(self.root, "run", "kfllm.sock")

    def tearDown(self):
        self.tmp.cleanup()

    def test_the_daemon_runs_unprivileged_with_no_capability_and_exports_one_readonly_directory(self):
        argv = lp.virtiofsd_argv("/opt/virtiofsd", self.sock, self.share, 65534)
        self.assertEqual(argv[:2], ["setpriv", "--reuid"])
        self.assertEqual(argv[argv.index("--reuid") + 1], "65534")
        self.assertEqual(argv[argv.index("--regid") + 1], "65534")
        for flag in ("--clear-groups", "--inh-caps=-all", "--ambient-caps=-all", "--bounding-set=-all", "--no-new-privs", "--readonly"):
            self.assertIn(flag, argv)
        self.assertIn(f"--shared-dir={self.share}", argv)
        self.assertIn(f"--socket-path={self.sock}", argv)
        self.assertEqual(argv[argv.index("--sandbox") + 1], "none")
        self.assertLess(argv.index("--no-new-privs"), argv.index("/opt/virtiofsd"))

    def test_nothing_enables_dax_or_a_writable_export(self):
        argv = lp.virtiofsd_argv("/opt/virtiofsd", self.sock, self.share, 65534)
        text = " ".join(argv + lp.qemu_share_args("virtiofs", socket_path=self.sock) + lp.qemu_share_args("9p", shared_dir=self.share))
        for word in ("cache-size", "dax", "writeback", "--allow-direct-io", "--posix-acl", "--xattr", "--modcaps", "--sandbox=namespace",
                     "passthrough", "mapped-xattr", "mapped-file"):
            self.assertNotIn(word, text)

    def test_the_daemon_uid_may_not_be_root_negative_or_a_bool(self):
        for uid in (0, -1, True, "65534", 2**31, None, 1.5):
            with self.assertRaises(lp.StageError, msg=repr(uid)):
                lp.virtiofsd_argv("/opt/virtiofsd", self.sock, self.share, uid)

    def test_hostile_paths_never_reach_an_argument(self):
        bad = ["share", "/a,b", "/a b", "/a\nb", "/a\x00b", "/a=b", "/a;b", "/a'b", '/a"b', "/a$b", "/a*b", "/a/../b", "/a/./b", "/a//b",
               "/a/", "", "/", "-x", "/a\\b", "/aé", "/" + "a" * 5000, None, 5, b"/a"]
        for path in bad:
            for build in (lambda p: lp.virtiofsd_argv(p, self.sock, self.share, 65534),
                          lambda p: lp.virtiofsd_argv("/opt/v", p, self.share, 65534),
                          lambda p: lp.virtiofsd_argv("/opt/v", self.sock, p, 65534),
                          lambda p: lp.qemu_share_args("virtiofs", socket_path=p),
                          lambda p: lp.qemu_share_args("9p", shared_dir=p)):
                with self.assertRaises(lp.StageError, msg=repr(path)) as cm:
                    build(path)
                self.assertEqual(cm.exception.reason, "share_path_unsafe")

    def test_dotdot_and_dot_segments_are_refused_without_a_filesystem_check(self):
        for path in ("/x/../y", "/x/./y", "/../y"):
            with self.assertRaises(lp.StageError):
                lp.safe_path(path, check_fs=False)

    def test_a_group_writable_share_is_refused_by_the_daemon_argument_builder(self):
        os.chmod(self.share, 0o775)
        with self.assertRaises(lp.StageError):
            lp.virtiofsd_argv("/opt/v", self.sock, self.share, 65534)

    def test_the_export_is_readonly_on_the_daemon_and_on_9p(self):
        self.assertIn("--readonly", lp.virtiofsd_argv("/opt/v", self.sock, self.share, 65534))
        self.assertIn("readonly=on", " ".join(lp.qemu_share_args("9p", shared_dir=self.share)))

    def test_the_daemon_drops_every_capability_set(self):
        argv = lp.virtiofsd_argv("/opt/v", self.sock, self.share, 65534)
        self.assertEqual([a for a in argv if a.endswith("=-all")], ["--inh-caps=-all", "--ambient-caps=-all", "--bounding-set=-all"])

    def test_the_sandbox_mode_is_none_because_it_is_the_one_that_starts_without_root(self):
        argv = lp.virtiofsd_argv("/opt/v", self.sock, self.share, 65534)
        self.assertEqual(argv[argv.index("--sandbox") + 1], "none")
        self.assertEqual(argv.count("--sandbox"), 1)

    def test_root_is_never_the_daemons_uid(self):
        with self.assertRaises(lp.StageError):
            lp.safe_uid(0)
        with self.assertRaises(lp.StageError):
            lp.virtiofsd_argv("/opt/v", self.sock, self.share, 1, gid=0)
        self.assertEqual(lp.safe_uid(1), 1)

    def test_a_tag_with_a_comma_cannot_reach_the_guest_mount_line_or_the_device_line(self):
        for build in (lambda: lp.guest_mount_command("virtiofs", "kfllm,x"), lambda: lp.guest_mount_command("9p", "a b; reboot"),
                      lambda: lp.qemu_share_args("virtiofs", socket_path=self.sock, tag="t,cache-size=1G")):
            with self.assertRaises(lp.StageError):
                build()

    def test_a_comma_cannot_inject_a_qemu_property(self):
        with self.assertRaises(lp.StageError):
            lp.qemu_share_args("9p", shared_dir=self.root + ",readonly=off")
        with self.assertRaises(lp.StageError):
            lp.qemu_share_args("virtiofs", socket_path=self.root + "/s.sock,server")

    def test_an_equals_sign_alone_is_refused_in_a_socket_path(self):
        with self.assertRaises(lp.StageError):
            lp.qemu_share_args("virtiofs", socket_path=self.root + "/s.sock=1")

    def test_a_socket_path_of_exactly_the_limit_is_accepted_and_one_more_is_not(self):
        ok = "/" + "a" * (lp.SOCKET_PATH_MAX - 1)
        self.assertEqual(len(ok), lp.SOCKET_PATH_MAX)
        self.assertEqual(len(lp.qemu_share_args("virtiofs", socket_path=ok)), 4)
        with self.assertRaises(lp.StageError):
            lp.qemu_share_args("virtiofs", socket_path=ok + "a")

    def test_the_daemon_cannot_gain_privileges(self):
        argv = lp.virtiofsd_argv("/opt/v", self.sock, self.share, 65534)
        self.assertIn("--no-new-privs", argv)
        self.assertLess(argv.index("--no-new-privs"), argv.index("/opt/v"))

    def test_a_symlinked_or_missing_or_writable_share_is_refused(self):
        link = os.path.join(self.root, "link")
        os.symlink(self.share, link)
        for path in (link, os.path.join(self.root, "missing")):
            with self.assertRaises(lp.StageError):
                lp.qemu_share_args("9p", shared_dir=path)
        os.chmod(self.share, 0o775)
        with self.assertRaises(lp.StageError):
            lp.qemu_share_args("9p", shared_dir=self.share)
        os.chmod(self.share, 0o757)
        with self.assertRaises(lp.StageError):
            lp.virtiofsd_argv("/opt/v", self.sock, self.share, 65534)

    def test_a_symlink_in_the_middle_of_the_path_is_refused(self):
        os.symlink(self.share, os.path.join(self.root, "via"))
        with self.assertRaises(lp.StageError):
            lp.qemu_share_args("9p", shared_dir=os.path.join(self.root, "via"))

    def test_a_file_is_not_a_share(self):
        f = os.path.join(self.root, "f")
        Path(f).write_text("x")
        with self.assertRaises(lp.StageError):
            lp.qemu_share_args("9p", shared_dir=f)

    def test_a_socket_path_longer_than_sun_path_is_refused(self):
        long = "/" + "a" * (lp.SOCKET_PATH_MAX + 1)
        with self.assertRaises(lp.StageError):
            lp.qemu_share_args("virtiofs", socket_path=long)
        with self.assertRaises(lp.StageError):
            lp.virtiofsd_argv("/opt/v", long, self.share, 65534)

    def test_virtiofs_arguments(self):
        args = lp.qemu_share_args("virtiofs", socket_path=self.sock)
        self.assertEqual(args, ["-chardev", f"socket,id=kfllmc,path={self.sock}", "-device", "vhost-user-fs-pci,chardev=kfllmc,tag=kfllm"])

    def test_9p_fallback_arguments_are_readonly_and_unmapped(self):
        args = lp.qemu_share_args("9p", shared_dir=self.share)
        self.assertEqual(args, ["-fsdev", f"local,id=kfllm9p,path={self.share},security_model=none,readonly=on",
                                "-device", "virtio-9p-pci,fsdev=kfllm9p,mount_tag=kfllm"])

    def test_the_tag_is_validated(self):
        for tag in ("", "a b", "a,b", "a" * 32, "a/b", "a=b", "é", None, 5):
            with self.assertRaises(lp.StageError, msg=repr(tag)):
                lp.qemu_share_args("virtiofs", socket_path=self.sock, tag=tag)
            with self.assertRaises(lp.StageError):
                lp.guest_mount_command("virtiofs", tag)
        self.assertEqual(len(lp.qemu_share_args("virtiofs", socket_path=self.sock, tag="a" * 31)), 4)

    def test_an_unknown_transport_is_refused(self):
        for t in ("nfs", "", None, "VIRTIOFS"):
            with self.assertRaises(lp.StageError):
                lp.qemu_share_args(t, shared_dir=self.share, socket_path=self.sock)
            with self.assertRaises(lp.StageError):
                lp.guest_mount_command(t)

    def test_the_guest_mounts_readonly_at_a_fixed_point(self):
        v = lp.guest_mount_command("virtiofs")
        self.assertIn("mount -t virtiofs -o ro kfllm /mnt/kf-llm", v)
        n = lp.guest_mount_command("9p")
        self.assertIn("-t 9p -o trans=virtio,version=9p2000.L,ro,", n)
        self.assertTrue(n.endswith("kfllm /mnt/kf-llm"))

    def test_traversable_needs_search_permission_on_every_ancestor(self):
        inner = os.path.join(self.root, "a", "b")
        os.makedirs(inner)
        os.chmod(self.root, 0o755)
        os.chmod(os.path.join(self.root, "a"), 0o755)
        os.chmod(inner, 0o755)
        self.assertTrue(lp.traversable(inner))
        os.chmod(os.path.join(self.root, "a"), 0o750)
        self.assertFalse(lp.traversable(inner))


class Transport(unittest.TestCase):
    BOTH = frozenset(("vhost-user-fs-pci", "virtio-9p-pci"))

    def test_auto_prefers_virtiofs(self):
        self.assertEqual(lp.choose_transport("auto", self.BOTH), "virtiofs")

    def test_auto_falls_back_to_9p_only_when_virtiofs_is_missing(self):
        self.assertEqual(lp.choose_transport("auto", frozenset(("virtio-9p-pci",))), "9p")

    def test_auto_refuses_when_neither_device_exists(self):
        with self.assertRaises(lp.StageError) as cm:
            lp.choose_transport("auto", frozenset(("virtio-net-pci",)))
        self.assertEqual(cm.exception.reason, "no_share_transport")

    def test_a_named_transport_is_never_silently_replaced(self):
        with self.assertRaises(lp.StageError) as cm:
            lp.choose_transport("virtiofs", frozenset(("virtio-9p-pci",)))
        self.assertEqual(cm.exception.reason, "transport_unavailable")
        with self.assertRaises(lp.StageError):
            lp.choose_transport("9p", frozenset(("vhost-user-fs-pci",)))
        self.assertEqual(lp.choose_transport("9p", self.BOTH), "9p")

    def test_an_unknown_request_is_refused(self):
        with self.assertRaises(lp.StageError):
            lp.choose_transport("sshfs", self.BOTH)

    def test_auto_with_both_devices_and_noise_never_picks_9p(self):
        self.assertEqual(lp.choose_transport("auto", frozenset(("virtio-9p-pci", "e1000", "vhost-user-fs-pci", "virtio-net-pci"))), "virtiofs")

    def test_an_empty_device_list_offers_no_transport(self):
        for requested in ("auto", "virtiofs", "9p"):
            with self.assertRaises(lp.StageError):
                lp.choose_transport(requested, frozenset())

    def test_9p_requested_without_its_device_is_refused(self):
        with self.assertRaises(lp.StageError) as cm:
            lp.choose_transport("9p", frozenset(("vhost-user-fs-pci", "e1000")))
        self.assertEqual(cm.exception.reason, "transport_unavailable")

    def test_the_device_list_is_read_from_qemu_help_text(self):
        text = 'name "vhost-user-fs-pci", bus PCI\nname "virtio-9p-pci", bus PCI, alias "virtio-9p"\nname "x y"\n  name "indented"\n'
        self.assertEqual(lp.parse_device_list(text), self.BOTH)


class Maths(unittest.TestCase):
    def test_tok_s(self):
        self.assertAlmostEqual(lp.tok_s(128, 10**10), 12.8)

    def test_zero_negative_nan_and_wrong_types_are_not_measurements(self):
        for count, dur in ((0, 10**9), (128, 0), (-1, 10**9), (128, -5), (float("nan"), 10**9), (128, float("nan")), (128, float("inf")),
                           (True, 10**9), (128, True), (None, 1), (1, None), ("1", 1), (1.5, 1)):
            self.assertIsNone(lp.tok_s(count, dur), (count, dur))

    def test_median_refuses_any_unusable_member(self):
        self.assertEqual(lp.median_rate([1.0, 3.0, 2.0]), 2.0)
        for bad in ([], [None], [1.0, None], [0.0], [-1.0, 2.0], [float("nan")], [float("inf"), 1.0]):
            self.assertIsNone(lp.median_rate(bad), bad)

    def test_ratio_is_guest_over_host_from_medians(self):
        r = lp.ratio_block([10.0, 10.0, 100.0], [9.0, 9.5, 1.0])
        self.assertEqual((r["state"], r["host_tok_s"], r["guest_tok_s"], r["value"]), ("OK", 10.0, 9.0, 0.9))
        self.assertTrue(r["meets_min"])
        self.assertFalse(r["meets_preferred"])

    def test_one_slow_run_does_not_move_the_median(self):
        self.assertEqual(lp.median_rate([10.0, 10.0, 1000.0]), 10.0)
        self.assertEqual(lp.ratio_block([10.0, 10.0, 1000.0], [10.0, 10.0, 0.001])["value"], 1.0)

    def test_the_targets_are_facts_about_the_value(self):
        self.assertEqual([lp.ratio_block([10.0], [g])[k] for g, k in ((8.9, "meets_min"), (9.0, "meets_min"), (9.4, "meets_preferred"), (9.5, "meets_preferred"))],
                         [False, True, False, True])

    def test_an_invalid_timing_gives_no_ratio_at_all(self):
        for host, guest in (([0.0], [1.0]), ([1.0], [0.0]), ([-1.0], [1.0]), ([1.0], [-1.0]), ([float("nan")], [1.0]), ([1.0], [float("nan")]),
                            ([None], [1.0]), ([1.0], [None]), ([], [1.0]), ([1.0], []), ([float("inf")], [1.0])):
            r = lp.ratio_block(host, guest)
            self.assertEqual((r["state"], r["value"], r["meets_min"], r["meets_preferred"]), ("INVALID", None, None, None), (host, guest))

    def test_the_ratio_block_never_carries_a_gate(self):
        self.assertIs(lp.ratio_block([1.0], [1.0])["gate"], False)
        self.assertIs(lp.ratio_block([0.0], [1.0])["gate"], False)

    def test_a_tiny_guest_over_a_huge_host_is_still_positive(self):
        r = lp.ratio_block([1e12], [1e-9])
        self.assertEqual(r["state"], "INVALID" if r["value"] is None else "OK")


class ArmOutput(unittest.TestCase):
    def parse(self, text, label="host"):
        return lp.parse_arm_output(text, label)

    def test_a_valid_arm_parses(self):
        self.assertEqual(self.parse(arm_text(arm_doc()))["label"], "host")

    def test_zero_or_two_result_lines_are_refused(self):
        for text in ("nothing\n", arm_text(arm_doc()) + arm_text(arm_doc()), "KF_LLM_ARM_FAIL reason=port_busy\n"):
            with self.assertRaises(lp.StageError) as cm:
                self.parse(text)
            self.assertEqual(cm.exception.reason, "host_arm_failed")

    def test_the_failure_line_names_the_arms_own_reason(self):
        with self.assertRaises(lp.StageError) as cm:
            self.parse("KF_LLM_ARM_FAIL reason=port_busy\n", "guest")
        self.assertEqual((cm.exception.reason, cm.exception.detail), ("guest_arm_failed", "port_busy"))

    def test_the_label_must_match_the_arm_that_was_asked(self):
        with self.assertRaises(lp.StageError):
            self.parse(arm_text(arm_doc("guest")), "host")

    def test_hostile_guest_documents_are_refused(self):
        def runs(**kw):
            d = arm_doc("guest")
            d["runs"][0].update(kw)
            return d

        bad = [
            arm_doc("guest", extra=1), runs(eval_count=-1), runs(eval_count=True), runs(eval_duration_ns=1.5), runs(eval_count=2**63),
            runs(text_sha256="zz"), runs(token_ids_sha256="1"), arm_doc("guest", runs=[]), arm_doc("guest", runs=arm_doc()["runs"][:2]),
            arm_doc("guest", schema=2), arm_doc("guest", create_secs=float("nan")), arm_doc("guest", load_ms=-1),
            arm_doc("guest", ps={"size": 1, "size_vram": -1}), arm_doc("guest", ps={"size": 1}), arm_doc("guest", gpu="x" * 81),
            arm_doc("guest", text="x" * 201), arm_doc("guest", ollama_version="a b"), arm_doc("guest", ollama_version=5),
            arm_doc("guest", inputs={"model": {}}), arm_doc("guest", params={"num_predict": 1}),
        ]
        for doc in bad:
            with self.assertRaises(lp.StageError, msg=json.dumps(doc)[:200]):
                self.parse(arm_text(doc), "guest")

    def test_a_guest_cannot_pass_off_the_hosts_result(self):
        with self.assertRaises(lp.StageError):
            self.parse(arm_text(arm_doc("host")), "guest")

    def test_an_unknown_top_level_field_is_refused(self):
        doc = arm_doc("host")
        doc["note"] = "hello"
        with self.assertRaises(lp.StageError):
            self.parse(arm_text(doc))

    def test_a_duplicated_key_cannot_hide_a_value(self):
        text = "KF_LLM_ARM_RESULT " + json.dumps(arm_doc("guest")).replace('"ps": {"size": 1000, "size_vram": 1000}', '"ps": {"size": 1000, "size_vram": 0}, "ps": {"size": 1000, "size_vram": 1000}') + "\n"
        with self.assertRaises(lp.StageError):
            self.parse(text, "guest")

    def test_nan_and_duplicate_keys_in_the_json_are_refused(self):
        text = "KF_LLM_ARM_RESULT " + json.dumps(arm_doc("guest")).replace('"create_secs": 1.5', '"create_secs": NaN') + "\n"
        with self.assertRaises(lp.StageError):
            self.parse(text, "guest")
        text = "KF_LLM_ARM_RESULT " + json.dumps(arm_doc("guest")).replace('"schema": 1', '"schema": 1, "schema": 1', 1) + "\n"
        with self.assertRaises(lp.StageError):
            self.parse(text, "guest")

    def test_an_oversize_line_is_refused(self):
        doc = arm_doc("guest", text="x" * 200)
        doc["inputs"]["model"]["pad"] = "y" * (lp.MAX_ARM_OUTPUT + 1)
        with self.assertRaises(lp.StageError):
            self.parse(arm_text(doc), "guest")

    def test_the_rates_are_computed_here_never_taken_from_the_arm(self):
        doc = arm_doc("guest")
        doc["runs"][0]["tok_s"] = 9999
        with self.assertRaises(lp.StageError):
            self.parse(arm_text(doc), "guest")
        rec = lp.arm_record(arm_doc())
        self.assertEqual(rec["tok_s"], [12.8, 12.8, 12.8])


class Verdict(unittest.TestCase):
    def setUp(self):
        self.pins = pinned()
        self.host = arm_doc("host")
        self.guest = arm_doc("guest")

    def decide(self):
        return lp.decide(self.host, self.guest, self.pins, "virtiofs")

    def test_agreeing_arms_pass(self):
        v, reason, ratio, agree = self.decide()
        self.assertEqual((v, reason, ratio["value"], agree["equal"]), ("PASS", None, 1.0, True))

    def test_the_ratio_value_never_changes_the_verdict(self):
        for ns in (10**11, 10**10 * 100 // 90, 10**9, 10**8):   # guest 10 %, 90 %, 1000 %, 10000 % of native
            for r in self.guest["runs"]:
                r["eval_duration_ns"] = ns
            v, reason, ratio, _ = self.decide()
            self.assertEqual((v, reason), ("PASS", None), ns)
            self.assertIs(ratio["gate"], False)

    def test_a_ratio_below_the_minimum_is_recorded_and_still_passes(self):
        for r in self.guest["runs"]:
            r["eval_duration_ns"] = 10**11
        rec = lp.build_record(SHA, "virtiofs", self.pins, host=self.host, guest=self.guest)
        self.assertEqual((rec["verdict"], rec["ratio"]["value"], rec["ratio"]["meets_min"]), ("PASS", 0.1, False))

    def test_unusable_timing_fails_with_a_named_reason_and_no_ratio(self):
        self.guest["runs"][1]["eval_duration_ns"] = 0
        v, reason, ratio, _ = self.decide()
        self.assertEqual((v, reason, ratio["value"]), ("FAIL", "timing_invalid", None))

    def test_different_text_fails(self):
        self.guest["runs"][2]["text_sha256"] = "9" * 64
        self.assertEqual(self.decide()[:2], ("FAIL", "text_disagree"))

    def test_different_token_ids_fail_when_every_run_has_them(self):
        self.guest["runs"][0]["token_ids_sha256"] = "9" * 64
        self.assertEqual(self.decide()[:2], ("FAIL", "text_disagree"))

    def test_missing_token_ids_fall_back_to_text_only(self):
        for r in self.guest["runs"]:
            r["token_ids_sha256"] = None
        v, _, _, agree = self.decide()
        self.assertEqual((v, agree["token_ids_compared"]), ("PASS", False))

    def test_a_different_token_count_fails(self):
        self.guest["runs"][0]["eval_count"] = 127
        self.assertEqual(self.decide()[:2], ("FAIL", "text_disagree"))

    def test_part_of_the_model_in_system_memory_fails(self):
        self.guest["ps"] = {"size": 1000, "size_vram": 999}
        self.assertEqual(self.decide()[:2], ("FAIL", "not_on_gpu"))

    def test_a_model_with_no_gpu_residency_fails_on_either_arm(self):
        for arm_name in ("host", "guest"):
            self.setUp()
            getattr(self, arm_name)["ps"] = {"size": 1000, "size_vram": 0}
            self.assertEqual(self.decide()[:2], ("FAIL", "not_on_gpu"))
        self.setUp()
        self.host["ps"] = {"size": 0, "size_vram": 0}
        self.assertEqual(self.decide()[:2], ("FAIL", "not_on_gpu"))

    def test_an_input_that_does_not_match_the_pin_fails_on_either_arm(self):
        for arm_name, item, key, value in (("host", "model", "sha256", "9" * 64), ("guest", "ollama", "sha256", "9" * 64), ("guest", "model", "bytes", 101)):
            self.setUp()
            getattr(self, arm_name)["inputs"][item][key] = value
            self.assertEqual(self.decide()[:2], ("FAIL", "inputs_mismatch"), (arm_name, item, key))

    def test_ollama_must_be_the_pinned_version_on_both_arms(self):
        self.guest["ollama_version"] = "0.0.2"
        self.assertEqual(self.decide()[:2], ("FAIL", "version_mismatch"))
        self.setUp()
        self.pins["ollama_version"] = "0.0.9"
        self.assertEqual(self.decide()[:2], ("FAIL", "version_mismatch"))

    def test_the_two_arms_must_have_run_the_same_parameters(self):
        self.guest["params"]["num_predict"] = 64
        self.assertEqual(self.decide()[:2], ("FAIL", "params_differ"))


class Record(unittest.TestCase):
    def passing(self):
        return lp.build_record(SHA, "virtiofs", pinned(), host=arm_doc("host"), guest=arm_doc("guest"))

    def test_a_passing_record_validates_in_both_validators(self):
        rec = self.passing()
        self.assertEqual(rec["verdict"], "PASS")
        lp.validate_record(rec)
        vast_report.validate_llm_parity(rec)

    def test_a_record_names_its_pins_and_both_raw_timings(self):
        rec = self.passing()
        self.assertEqual(rec["items"]["model"], {"sha256": H, "size": 100_000_000})
        for arm_name in ("host", "guest"):
            self.assertEqual(rec["arms"][arm_name]["eval_duration_ns"], [10**10] * 3)
            self.assertEqual(rec["arms"][arm_name]["eval_count"], [128] * 3)
        self.assertEqual(rec["ratio"]["value"], 1.0)
        self.assertEqual(rec["arms"]["guest"]["share_read"]["model"]["MBps"], 200.0)

    def test_a_failed_guest_arm_keeps_the_host_arm_and_the_reason(self):
        rec = lp.build_record(SHA, "virtiofs", pinned(), host=arm_doc("host"), reason="guest_arm_failed", detail="guest_mount")
        self.assertEqual((rec["verdict"], rec["reason"], list(rec["arms"])), ("FAIL", "guest_arm_failed", ["host"]))
        vast_report.validate_llm_parity(rec)

    def test_a_refusal_record_validates(self):
        rec = lp.build_record(SHA, None, None, reason="unpinned", detail="model")
        lp.validate_record(rec)
        vast_report.validate_llm_parity(rec)
        self.assertEqual(rec["arms"], {})

    def test_the_two_validators_agree_on_a_corpus(self):
        base = self.passing()
        corpus = [base]
        edits = [
            lambda r: r.update(extra=1), lambda r: r.pop("arms"), lambda r: r.update(schema=2), lambda r: r.update(sha="g" * 40),
            lambda r: r.update(verdict="MAYBE"), lambda r: r.update(transport="nfs"), lambda r: r.update(reason="x"),
            lambda r: r["ratio"].update(gate=True), lambda r: r["ratio"].update(min=0.5), lambda r: r["ratio"].update(value=-1.0),
            lambda r: r["ratio"].update(value=float("nan")), lambda r: r["ratio"].update(state="INVALID"),
            lambda r: r["ratio"].update(meets_min="yes"), lambda r: r["agreement"].update(equal=False),
            lambda r: r["agreement"].update(mode="prefix"), lambda r: r["arms"]["host"].update(ran=False),
            lambda r: r["arms"]["host"].update(tok_s=[0.0, 1.0, 1.0]), lambda r: r["arms"]["host"].update(tok_s=[1.0]),
            lambda r: r["arms"]["guest"].update(gpu="x" * 81), lambda r: r["arms"]["guest"].pop("share_read"),
            lambda r: r["arms"]["guest"].update(eval_count=[1, 2]), lambda r: r["items"].update(model={"sha256": "z", "size": 1}),
            lambda r: r["items"].update(other={"sha256": H, "size": 1}), lambda r: r.update(detail="x" * 201),
            lambda r: r["arms"].update(other=r["arms"]["host"]), lambda r: r.update(ollama_version=""),
            lambda r: r["arms"]["guest"].update(size_vram=-1), lambda r: r["arms"]["guest"].update(load_ms=float("inf")),
            lambda r: r["items"]["model"].update(size=0), lambda r: r.update(arms={"host": r["arms"]["host"]}),
            lambda r: r["arms"]["host"].update(eval_count=[True, 128, 128]), lambda r: r["arms"]["host"].update(eval_duration_ns=[2**63, 1, 1]),
            lambda r: r["arms"]["guest"].update(size=True),
            lambda r: (r.update(verdict="FAIL"), r.pop("reason", None)),
            lambda r: r.update(detail="y" * 17000),
        ]
        for edit in edits:
            r = duplicate(base)
            edit(r)
            corpus.append(r)
        for rec in corpus:
            def verdict_of(fn):
                try:
                    fn(rec)
                    return True
                except ValueError:
                    return False

            self.assertEqual(verdict_of(lp.validate_record), verdict_of(vast_report.validate_llm_parity), json.dumps(rec)[:240])
        rejected = 0
        for rec in corpus[1:]:
            try:
                lp.validate_record(rec)
            except ValueError:
                rejected += 1
        self.assertEqual(rejected, len(edits))

    def test_a_failed_record_must_name_its_reason_in_both_validators(self):
        r = lp.build_record(SHA, None, None, reason="unpinned")
        del r["reason"]
        with self.assertRaises(ValueError):
            lp.validate_record(r)
        with self.assertRaises(ValueError):
            vast_report.validate_llm_parity(r)

    def test_a_record_with_an_unknown_field_is_refused_by_both_validators(self):
        r = self.passing()
        r["note"] = "x"
        with self.assertRaises(ValueError):
            lp.validate_record(r)
        with self.assertRaises(ValueError):
            vast_report.validate_llm_parity(r)

    def test_a_failed_verdict_with_a_named_reason_is_valid_in_both(self):
        r = self.passing()
        r.update(verdict="FAIL", reason="text_disagree")
        r["agreement"]["equal"] = False
        lp.validate_record(r)
        vast_report.validate_llm_parity(r)

    def test_a_pass_record_with_a_failed_agreement_or_invalid_ratio_is_refused(self):
        for edit in (lambda r: r["agreement"].update(equal=False),
                     lambda r: r["ratio"].update(state="INVALID", value=None, meets_min=None, meets_preferred=None)):
            r = self.passing()
            edit(r)
            with self.assertRaises(ValueError):
                lp.validate_record(r)
            with self.assertRaises(ValueError):
                vast_report.validate_llm_parity(r)

    def test_the_gate_flag_must_be_false_in_both_validators(self):
        r = self.passing()
        r["ratio"]["gate"] = True
        with self.assertRaises(ValueError):
            lp.validate_record(r)
        with self.assertRaises(ValueError):
            vast_report.validate_llm_parity(r)

    def test_every_reason_the_module_emits_is_a_named_one(self):
        import re

        source = (HERE / "llm_parity.py").read_text()
        used = set(re.findall(r'StageError\("([a-z0-9_]+)"', source)) | set(re.findall(r'reason="([a-z0-9_]+)"', source))
        used |= {r + "_arm_failed" for r in ("host", "guest")}
        self.assertLessEqual(used - {"x"}, lp.REASONS | {"usage"})
        for reason in lp.REASONS:
            self.assertRegex(reason, lp.REASON_RE)


class ArmEndToEnd(unittest.TestCase):
    """The real arm script against a fake `ollama` that serves the same four calls."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(os.path.realpath(self.tmp.name))
        self.tar = self.root / "ollama.tgz"
        make_tar(self.tar)
        self.model = self.root / "model.gguf"
        self.model.write_bytes(b"GGUF" + b"\0" * 5000)
        self.port = free_port()

    def tearDown(self):
        self.tmp.cleanup()

    def run_arm(self, label="host", cfg=None, tar=None, model=None, port=None, extra=(), name=None):
        work = self.root / ("work_" + (name or label))
        work.mkdir(exist_ok=True)
        (work / "fake.json").write_text(json.dumps(cfg or {}))
        return subprocess.run(
            [sys.executable, "-I", str(HERE / "llm_parity_arm.py"), "--label", label, "--tarball", str(tar or self.tar),
             "--model", str(model or self.model), "--work", str(work / "run"), "--port", str(port or self.port), "--timeout", "60", *extra],
            capture_output=True, text=True, timeout=120)

    def test_the_arm_prints_one_valid_result_and_the_share_throughput_lines(self):
        done = self.run_arm(cfg={"eval_ns": 8 * 10**9})
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        doc = lp.parse_arm_output(done.stdout, "host")
        self.assertEqual([r["eval_duration_ns"] for r in doc["runs"]], [8 * 10**9] * 3)
        self.assertEqual(doc["inputs"]["model"]["bytes"], 5004)
        self.assertEqual(doc["inputs"]["model"]["sha256"], __import__("hashlib").sha256(self.model.read_bytes()).hexdigest())
        self.assertEqual(doc["ps"], {"size": 1000, "size_vram": 1000})
        lines = [x for x in done.stdout.splitlines() if x.startswith("KF_LLM_SHARE_READ ")]
        self.assertEqual(len(lines), 2)
        self.assertRegex(lines[0], r"^KF_LLM_SHARE_READ label=host file=model bytes=5004 secs=[0-9.]+ MBps=[0-9.]+$")

    def test_the_server_is_gone_afterwards_and_the_port_is_free(self):
        self.assertEqual(self.run_arm().returncode, 0)
        with socket.socket() as s:
            self.assertNotEqual(s.connect_ex(("127.0.0.1", self.port)), 0)

    def test_the_server_is_stopped_even_when_a_later_step_fails(self):
        done = self.run_arm(cfg={"create_fails": True})
        self.assertEqual(done.returncode, 1)
        self.assertIn("KF_LLM_ARM_FAIL reason=create_failed", done.stdout)
        with socket.socket() as s:
            self.assertNotEqual(s.connect_ex(("127.0.0.1", self.port)), 0)

    def test_it_refuses_to_measure_somebody_elses_server(self):
        with socket.socket() as s:
            s.bind(("127.0.0.1", self.port))
            s.listen(1)
            done = self.run_arm()
        self.assertEqual(done.returncode, 1)
        self.assertIn("KF_LLM_ARM_FAIL reason=port_busy", done.stdout)
        self.assertNotIn("KF_LLM_ARM_RESULT", done.stdout)

    def test_a_listener_that_speaks_http_is_not_mistaken_for_our_server(self):
        import http.server
        import threading

        class Other(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def reply(self, doc):
                body = json.dumps(doc).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                self.reply({"version": "0.0.1"} if self.path == "/api/version" else {"models": [{"size": 5, "size_vram": 5}]})

            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                self.reply({"response": "other server", "eval_count": 1, "eval_duration": 1, "prompt_eval_count": 1, "total_duration": 1, "load_duration": 1})

        server = http.server.HTTPServer(("127.0.0.1", self.port), Other)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            done = self.run_arm()
        finally:
            server.shutdown()
            server.server_close()
        self.assertNotEqual(done.returncode, 0)
        self.assertNotIn("KF_LLM_ARM_RESULT", done.stdout)

    def test_no_secret_in_the_callers_environment_reaches_the_server(self):
        work = self.root / "work_host"
        work.mkdir()
        (work / "fake.json").write_text("{}")
        env = dict(os.environ, AWS_SECRET_ACCESS_KEY="top-secret", KF_RUN_TOKEN="tok", OLLAMA_HOST="0.0.0.0:9")
        done = subprocess.run([sys.executable, "-I", str(HERE / "llm_parity_arm.py"), "--label", "host", "--tarball", str(self.tar), "--model", str(self.model),
                               "--work", str(work / "run"), "--port", str(self.port), "--timeout", "60"], capture_output=True, text=True, env=env)
        self.assertEqual(done.returncode, 0, done.stdout)
        seen = json.loads((work / "env.json").read_text())
        self.assertNotIn("AWS_SECRET_ACCESS_KEY", seen)
        self.assertNotIn("KF_RUN_TOKEN", seen)
        self.assertEqual(seen["OLLAMA_HOST"], f"127.0.0.1:{self.port}")

    def test_a_symlink_that_leaves_the_directory_is_refused(self):
        make_tar(self.root / "s.tgz", extra=[(tinfo("bin/l", tarfile.SYMTYPE, "../../../../etc/passwd"), None)])
        self.assertIn("reason=tar_unsafe", self.run_arm(tar=self.root / "s.tgz", name="s").stdout)

    def test_a_hard_link_is_refused(self):
        make_tar(self.root / "h.tgz", extra=[(tinfo("bin/h", tarfile.LNKTYPE, "bin/ollama"), None)])
        self.assertIn("reason=tar_unsafe", self.run_arm(tar=self.root / "h.tgz", name="h").stdout)

    def test_a_device_node_is_refused(self):
        make_tar(self.root / "d.tgz", extra=[(tinfo("dev/zero", tarfile.CHRTYPE), None)])
        self.assertIn("reason=tar_unsafe", self.run_arm(tar=self.root / "d.tgz", name="d").stdout)

    def test_a_stale_binary_from_an_earlier_run_is_never_executed(self):
        self.assertEqual(self.run_arm().returncode, 0)
        make_tar(self.root / "nobin.tgz", binary=None, extra=[(tinfo("lib/x", size=1), b"x")])
        done = self.run_arm(tar=self.root / "nobin.tgz")
        self.assertNotEqual(done.returncode, 0)
        self.assertNotIn("KF_LLM_ARM_RESULT", done.stdout)

    def test_a_server_that_exits_is_named(self):
        done = self.run_arm(cfg={"die": True})
        self.assertIn("KF_LLM_ARM_FAIL reason=serve_exited", done.stdout)

    def test_a_missing_input_is_named(self):
        done = self.run_arm(model=self.root / "absent.gguf")
        self.assertIn("KF_LLM_ARM_FAIL reason=input_unreadable", done.stdout)

    def test_a_tarball_without_the_binary_is_refused(self):
        make_tar(self.root / "nobin.tgz", binary=None, extra=[(tinfo("lib/x", size=1), b"x")])
        self.assertIn("reason=tar_no_binary", self.run_arm(tar=self.root / "nobin.tgz").stdout)

    def test_hostile_tar_members_are_refused_before_anything_is_written(self):
        cases = {
            "absolute": [(tinfo("/etc/evil", size=1), b"x")],
            "dotdot": [(tinfo("a/../../evil", size=1), b"x")],
            "symlink_out": [(tinfo("bin/link", tarfile.SYMTYPE, "../../../etc/passwd"), None)],
            "symlink_abs": [(tinfo("bin/link2", tarfile.SYMTYPE, "/etc/passwd"), None)],
            "hardlink": [(tinfo("bin/hl", tarfile.LNKTYPE, "bin/ollama"), None)],
            "device": [(tinfo("dev/null", tarfile.CHRTYPE), None)],
            "fifo": [(tinfo("dev/fifo", tarfile.FIFOTYPE), None)],
        }
        for name, extra in cases.items():
            make_tar(self.root / (name + ".tgz"), extra=extra)
            done = self.run_arm(tar=self.root / (name + ".tgz"), name=name)
            self.assertIn("KF_LLM_ARM_FAIL reason=tar_unsafe", done.stdout, name)
            self.assertFalse((self.root / "etc").exists())
            self.assertFalse((self.root / ("work_" + name) / "run" / "ollama" / "bin").exists(), name)

    def test_an_arm_work_directory_that_already_exists_is_refused_not_reused(self):
        self.assertEqual(self.run_arm().returncode, 0)
        again = self.run_arm()
        self.assertIn("KF_LLM_ARM_FAIL reason=work_exists", again.stdout)

    def test_a_tar_that_is_not_a_tar_is_named(self):
        (self.root / "junk.tgz").write_bytes(b"not a tar" * 100)
        self.assertIn("reason=tar_unreadable", self.run_arm(tar=self.root / "junk.tgz").stdout)

    def test_bad_arguments_are_a_usage_error(self):
        for extra in (["--repeats", "0"], ["--repeats", "99"]):
            self.assertEqual(self.run_arm(extra=extra).returncode, 2)
        done = subprocess.run([sys.executable, "-I", str(HERE / "llm_parity_arm.py"), "--label", "host", "--tarball", "/a b", "--model", "/m",
                               "--work", "/w"], capture_output=True, text=True)
        self.assertEqual(done.returncode, 2)
        done = subprocess.run([sys.executable, "-I", str(HERE / "llm_parity_arm.py"), "--label", "other", "--tarball", "/a", "--model", "/m",
                               "--work", "/w"], capture_output=True, text=True)
        self.assertEqual(done.returncode, 2)

    def test_the_prompt_and_parameters_have_one_source_for_both_arms(self):
        self.assertEqual(arm.prompt_sha256(), __import__("hashlib").sha256(arm.PROMPT.encode()).hexdigest())
        source = (HERE / "llm_parity.py").read_text()
        self.assertNotIn("PROMPT", source)
        self.assertNotIn("num_predict", source.replace('"num_predict"', ""))

    def test_the_server_environment_is_an_allowlist_and_loopback_only(self):
        with patch.dict(os.environ, {"AWS_SECRET_ACCESS_KEY": "s", "KF_RUN_TOKEN": "t", "PATH": "/usr/bin", "OLLAMA_HOST": "0.0.0.0:1"}):
            env = arm.server_env("/w", 4242)
        self.assertEqual(env["OLLAMA_HOST"], "127.0.0.1:4242")
        self.assertNotIn("AWS_SECRET_ACCESS_KEY", env)
        self.assertNotIn("KF_RUN_TOKEN", env)
        self.assertEqual(env["HOME"], "/w/home")


class Orchestration(unittest.TestCase):
    """cmd_run with the box-bound steps replaced by the real arm against the fake ollama."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(os.path.realpath(self.tmp.name))
        os.chmod(self.root, 0o755)
        self.src = self.root / "src"
        self.src.mkdir()
        make_tar(self.src / "ollama.tgz")
        (self.src / "model.gguf").write_bytes(b"GGUF" + b"\1" * 3000)
        import hashlib

        self.sha = {n: hashlib.sha256((self.src / n).read_bytes()).hexdigest() for n in ("ollama.tgz", "model.gguf")}
        self.pins = pinned(ollama_version="0.0.1")
        self.pins["items"]["model"].update(sha256=self.sha["model.gguf"], size=3004)
        self.pins["items"]["ollama"].update(sha256=self.sha["ollama.tgz"], size=(self.src / "ollama.tgz").stat().st_size)
        self.pins_path = self.root / "pins.json"
        self.pins_path.write_text(json.dumps(self.pins))
        self.out = self.root / "out.json"
        self.calls = []
        self.guest_cfg = {"eval_ns": 10**10}
        self.host_cfg = {"eval_ns": 10**10}
        self.patches = [patch.object(lp, "MIN_FREE_BYTES", 1)]
        for p in self.patches:
            p.start()

    def tearDown(self):
        for p in self.patches:
            p.stop()
        self.tmp.cleanup()

    def args(self, **over):
        ns = dict(sha=SHA, checkout=str(self.root), work=str(self.root / "work"), out=str(self.out), pins=str(self.pins_path),
                  transport="auto", pin_dir=[], urls=None, s3_config=None, vfsd_uid=65534, limit_bandwidth=None)
        ns.update(over)
        return type("Args", (), ns)()

    def fetch(self, manifest, dest, args, say):
        self.calls.append("fetch")
        (Path(dest) / "llm").mkdir(parents=True, exist_ok=True)
        shutil.copy(self.src / "model.gguf", Path(dest) / "llm/model.gguf")
        shutil.copy(self.src / "ollama.tgz", Path(dest) / "llm/ollama.tgz")

    def run_arm_as(self, label, cfg):
        def step(share, work, say=None):
            w = Path(work) / ("arm_" + label)
            shutil.rmtree(w, ignore_errors=True)  # the real guest hook removes the guest's scratch the same way
            w.mkdir()
            (w / "fake.json").write_text(json.dumps(cfg))
            done = subprocess.run([sys.executable, "-I", str(HERE / "llm_parity_arm.py"), "--label", label, "--tarball", f"{share}/ollama.tgz",
                                   "--model", f"{share}/model.gguf", "--work", str(w / "run"), "--port", str(free_port()), "--timeout", "60"],
                                  capture_output=True, text=True, timeout=120)
            self.calls.append(label)
            return done.stdout
        return step

    def steps(self, **over):
        steps = {
            "preflight": lambda a, p: "virtiofs", "fetch": self.fetch,
            "host_arm": lambda share, work, say: self.run_arm_as("host", self.host_cfg)(share, work),
            "guest_arm": lambda a, t, work, share, pins: self.run_arm_as("guest", self.guest_cfg)(share, work),
        }
        steps.update(over)
        return steps

    def go(self, **over):
        buf = []
        with patch.object(lp, "say", lambda line: buf.append(line)):
            rc = lp.cmd_run(self.args(), self.steps(**over))
        return rc, buf, json.loads(self.out.read_text())

    def test_both_arms_agree_and_the_ratio_is_recorded(self):
        self.guest_cfg = {"eval_ns": 12 * 10**9}
        rc, lines, rec = self.go()
        self.assertEqual((rc, rec["verdict"]), (0, "PASS"), rec)
        self.assertEqual(rec["ratio"]["value"], 0.8333)
        self.assertEqual(rec["arms"]["host"]["eval_duration_ns"], [10**10] * 3)
        self.assertEqual(rec["arms"]["guest"]["eval_duration_ns"], [12 * 10**9] * 3)
        self.assertEqual(self.calls, ["fetch", "host", "guest"])
        self.assertEqual(len([x for x in lines if x.startswith("KF_LLM_PARITY_RESULT ")]), 1)
        self.assertTrue(any(x.startswith("KF_LLM_PARITY_VERDICT PASS ") for x in lines))
        # the host arm's lines are printed by run_host_arm, which these tests replace; the guest arm's are printed by cmd_run
        self.assertEqual(len([x for x in lines if x.startswith("KF_LLM_SHARE_READ ")]), 2)

    def test_a_slow_guest_is_a_pass_with_the_ratio_recorded(self):
        self.guest_cfg = {"eval_ns": 10**11}
        rc, _, rec = self.go()
        self.assertEqual((rc, rec["verdict"], rec["ratio"]["value"], rec["ratio"]["meets_min"]), (0, "PASS", 0.1, False))

    def test_a_guest_with_different_text_fails_with_the_ratio_still_recorded(self):
        self.guest_cfg = {"text": "different"}
        rc, _, rec = self.go()
        self.assertEqual((rc, rec["verdict"], rec["reason"]), (1, "FAIL", "text_disagree"))
        self.assertEqual(rec["ratio"]["value"], 1.0)

    def test_a_guest_off_the_gpu_fails(self):
        self.guest_cfg = {"vram": 0}
        self.assertEqual(self.go()[2]["reason"], "not_on_gpu")

    def test_a_guest_with_a_zero_duration_has_no_ratio_and_fails_on_timing(self):
        self.guest_cfg = {"eval_ns": 0}
        rec = self.go()[2]
        self.assertEqual((rec["reason"], rec["ratio"]["state"], rec["ratio"]["value"]), ("timing_invalid", "INVALID", None))

    def test_a_guest_that_generated_another_number_of_tokens_fails(self):
        self.guest_cfg = {"eval_count": 127}
        self.assertEqual(self.go()[2]["reason"], "text_disagree")

    def test_a_model_of_another_size_than_pinned_fails(self):
        pins = json.loads(self.pins_path.read_text())
        pins["items"]["model"]["size"] = 3005
        self.pins_path.write_text(json.dumps(pins))
        self.assertEqual(self.go()[2]["reason"], "inputs_mismatch")

    def test_no_file_in_the_share_is_writable_by_anyone(self):
        self.go()
        for p in (self.root / "work" / "share").iterdir():
            self.assertEqual(p.stat().st_mode & 0o222, 0, p.name)

    def test_a_work_directory_that_is_not_searchable_refuses_before_any_download(self):
        work = self.root / "work"
        work.mkdir()
        os.chmod(work, 0o700)
        self.go()
        self.assertEqual(self.calls, [])

    def test_a_guest_with_part_of_the_model_in_system_memory_fails(self):
        self.guest_cfg = {"vram": 999}
        self.assertEqual(self.go()[2]["reason"], "not_on_gpu")

    def test_different_token_ids_fail_even_with_the_same_text(self):
        self.guest_cfg = {"context_ids": [9, 9]}
        self.assertEqual(self.go()[2]["reason"], "text_disagree")

    def test_pins_that_do_not_match_what_the_arms_read_fail(self):
        pins = json.loads(self.pins_path.read_text())
        pins["items"]["model"]["sha256"] = "9" * 64
        self.pins_path.write_text(json.dumps(pins))
        rc, _, rec = self.go()
        self.assertEqual((rc, rec["reason"]), (1, "inputs_mismatch"))

    def test_a_guest_running_another_ollama_version_fails(self):
        self.guest_cfg = {"version": "9.9.9"}
        self.assertEqual(self.go()[2]["reason"], "version_mismatch")

    def test_a_guest_that_reports_other_parameters_fails(self):
        doc = arm_doc("guest")
        doc["params"]["num_predict"] = 64
        doc["inputs"]["model"].update(sha256=self.sha["model.gguf"], bytes=3004)
        doc["inputs"]["ollama"].update(sha256=self.sha["ollama.tgz"], bytes=self.pins["items"]["ollama"]["size"])
        doc["ollama_version"] = "0.0.1"
        rc, _, rec = self.go(guest_arm=lambda a, t, w, s, p: arm_text(doc))
        self.assertEqual(rec["reason"], "params_differ")

    def test_a_guest_that_prints_two_result_lines_is_refused(self):
        host = self.run_arm_as("guest", self.guest_cfg)
        rc, _, rec = self.go(guest_arm=lambda a, t, w, s, p: (lambda text: text + text)(host(s, w)))
        self.assertEqual(rec["reason"], "guest_arm_failed")

    def test_a_previous_share_is_replaced_not_reused(self):
        (self.root / "work" / "share").mkdir(parents=True)
        (self.root / "work" / "share" / "stale").write_text("x")
        self.assertEqual(self.go()[2]["verdict"], "PASS")
        self.assertEqual(sorted(p.name for p in (self.root / "work" / "share").iterdir()), ["model.gguf", "ollama.tgz"])

    def test_the_share_directory_is_not_writable_by_anyone(self):
        self.go()
        self.assertEqual((self.root / "work" / "share").stat().st_mode & 0o222, 0)

    def test_a_guest_arm_failure_keeps_the_host_numbers(self):
        rc, _, rec = self.go(guest_arm=lambda a, t, w, s, p: "KF_LLM_ARM_FAIL reason=guest_mount\n")
        self.assertEqual((rc, rec["reason"], rec["detail"], list(rec["arms"])), (1, "guest_arm_failed", "guest_mount", ["host"]))

    def test_a_host_arm_failure_stops_before_the_guest_boots(self):
        self.host_cfg = {"die": True}
        rc, _, rec = self.go()
        self.assertEqual((rec["reason"], rec["arms"]), ("host_arm_failed", {}))
        self.assertNotIn("guest", self.calls)

    def test_unpinned_items_refuse_before_anything_is_fetched(self):
        pins = json.loads(self.pins_path.read_text())
        pins["items"]["model"].update(sha256="UNSET", size="UNSET")
        self.pins_path.write_text(json.dumps(pins))
        rc, lines, rec = self.go()
        self.assertEqual((rc, rec["verdict"], rec["reason"], rec["detail"]), (1, "FAIL", "unpinned", "model"))
        self.assertEqual(self.calls, [])
        self.assertIn("KF_LLM_PARITY_REFUSED reason=unpinned", lines)

    def test_the_virtiofsd_pin_is_only_needed_for_the_virtiofs_transport(self):
        pins = json.loads(self.pins_path.read_text())
        pins["items"]["virtiofsd"].update(sha256="UNSET", size="UNSET", sources=[])
        self.pins_path.write_text(json.dumps(pins))
        rc, _, rec = self.go(preflight=lambda a, p: "virtiofs")
        self.assertEqual((rec["reason"], self.calls), ("unpinned", []))
        rc, _, rec = self.go(preflight=lambda a, p: "9p")
        self.assertEqual(rec["verdict"], "PASS")

    def test_the_repository_pins_refuse_the_real_stage_without_touching_the_box(self):
        out = self.root / "real.json"
        done = subprocess.run([sys.executable, str(HERE / "llm_parity.py"), "run", "--sha", SHA, "--checkout", str(self.root),
                               "--work", str(self.root / "w2"), "--out", str(out)], capture_output=True, text=True)
        self.assertEqual(done.returncode, 1)
        self.assertIn("KF_LLM_PARITY_REFUSED reason=unpinned", done.stdout)
        self.assertFalse((self.root / "w2").exists())
        rec = json.loads(out.read_text())
        self.assertEqual((rec["verdict"], rec["reason"]), ("FAIL", "unpinned"))
        vast_report.validate_llm_parity(rec)

    def test_a_bug_in_a_step_still_leaves_one_named_record(self):
        def boom(*a):
            raise RuntimeError("secret detail that must not be echoed beyond the type")

        rc, lines, rec = self.go(host_arm=boom)
        self.assertEqual((rc, rec["verdict"], rec["reason"], rec["detail"]), (1, "FAIL", "internal_error", "RuntimeError"))
        self.assertEqual(len([x for x in lines if x.startswith("KF_LLM_PARITY_RESULT ")]), 1)
        vast_report.validate_llm_parity(rec)

    def test_a_timeout_in_a_box_step_is_a_named_failure_not_a_traceback(self):
        expired = lambda *a, **k: (_ for _ in ()).throw(lp.subprocess.TimeoutExpired("x", 1))  # noqa: E731
        with patch.object(lp.subprocess, "run", expired):
            with self.assertRaises(lp.StageError) as cm:
                lp.run_host_arm(self.root, self.root, lambda line: None)
            self.assertEqual((cm.exception.reason, cm.exception.detail), ("host_arm_failed", "timeout"))
            manifest = lp.build_manifest(pinned(), ("model", "ollama"))
            with self.assertRaises(lp.StageError) as cm:
                lp.fetch_items(manifest, self.root / "w" / "dl", type("A", (), dict(pin_dir=[], urls=None, s3_config=None, limit_bandwidth=None))(), lambda line: None)
            self.assertEqual((cm.exception.reason, cm.exception.detail), ("download_failed", "timeout"))
            with self.assertRaises(lp.StageError) as cm:
                lp.kf3_binary(self.root)
            self.assertEqual(cm.exception.reason, "no_kf3_binary")
            with self.assertRaises(lp.StageError):
                lp.device_list(self.root / "qemu")

    def test_a_bad_sha_is_a_usage_refusal_not_a_run(self):
        buf = []
        with patch.object(lp, "say", lambda line: buf.append(line)):
            rc = lp.cmd_run(self.args(sha="zz"), self.steps())
        self.assertEqual(rc, 1)
        self.assertEqual(self.calls, [])

    def test_the_share_holds_only_the_model_and_the_tarball_readonly(self):
        self.go()
        share = self.root / "work" / "share"
        self.assertEqual(sorted(p.name for p in share.iterdir()), ["model.gguf", "ollama.tgz"])
        for p in share.iterdir():
            self.assertEqual(stat.S_IMODE(p.stat().st_mode), 0o444)
        self.assertEqual(stat.S_IMODE(share.stat().st_mode), 0o555)

    def test_a_rerun_over_a_previous_share_works(self):
        self.go()
        self.calls.clear()
        rc, _, rec = self.go()
        self.assertEqual(rec["verdict"], "PASS", rec)

    def test_a_work_directory_nobody_else_can_search_is_refused(self):
        work = self.root / "work"
        work.mkdir()
        os.chmod(work, 0o700)
        os.chmod(self.root, 0o700)
        try:
            rc, _, rec = self.go()
        finally:
            os.chmod(self.root, 0o755)
        self.assertEqual(rec["reason"], "share_not_traversable")


class GuestSide(unittest.TestCase):
    """The hook and the boot plumbing, with a stub `gssh_nv` and a stub boot_capture.sh."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(os.path.realpath(self.tmp.name))
        os.chmod(self.root, 0o755)
        make_tar(self.root / "ollama.tgz")
        (self.root / "model.gguf").write_bytes(b"GGUF" + b"\2" * 2000)
        (self.root / "work").mkdir()
        (self.root / "work" / "fake.json").write_text("{}")
        self.stub = self.root / "gssh"
        self.stub.write_text(textwrap.dedent(f'''\
            #!{sys.executable}
            import os, shlex, subprocess, sys
            cmd = sys.argv[1]
            mode = os.environ.get("STUB_MODE", "ok")
            log = open({str(self.root / "gssh.log")!r}, "a"); log.write(cmd + "\\n"); log.close()
            if mode == "down": sys.exit(255)
            if cmd.startswith("findmnt"):
                fs = "ext4" if mode == "wrongfs" else "virtiofs"
                opts = "rw,relatime" if mode == "rw" else "ro,relatime"
                print(fs, opts); print("WRITABLE" if mode == "writable" else "READONLY")
            elif cmd.startswith("mkdir -p"): print("1" + "0" * (3 if mode == "smalldisk" else 12))
            elif cmd.startswith("python3 -I -"):
                argv = shlex.split(cmd)[3:]
                sys.exit(subprocess.run([{sys.executable!r}, "-I", "-"] + argv, stdin=sys.stdin).returncode)
            '''))
        self.stub.chmod(0o755)
        self.cfg = {"transport": "virtiofs", "tag": "kfllm", "port": free_port(), "out": str(self.root / "guest.out"),
                    "guest_work": str(self.root / "gw"), "tarball": str(self.root / "ollama.tgz"), "model": str(self.root / "model.gguf"),
                    "arm": str(HERE / "llm_parity_arm.py"), "gssh": str(self.stub)}
        self.cfg_path = self.root / "cfg.json"
        self.cfg_path.write_text(json.dumps(self.cfg))
        (self.root / "gw").mkdir()
        self.patch = patch.object(lp, "MIN_FREE_BYTES", 10**6)
        self.patch.start()

    def tearDown(self):
        self.patch.stop()
        self.tmp.cleanup()

    def hook(self, mode="ok"):
        with patch.dict(os.environ, {"STUB_MODE": mode}):
            # the arm's scratch is <guest_work>/run and the fake's config sits beside it
            (self.root / "gw" / "fake.json").write_text("{}")
            self.assertEqual(lp.guest_hook(self.cfg_path), 0)
        return Path(self.cfg["out"]).read_text()

    def test_the_hook_mounts_proves_readonly_and_returns_the_arms_output(self):
        text = self.hook()
        self.assertIn("KF_LLM_SHARE_MOUNT transport=virtiofs fstype=virtiofs ro=1 writable_probe=no", text)
        self.assertIn("KF_LLM_GUEST_DISK avail_bytes=", text)
        doc = lp.parse_arm_output(text, "guest")
        self.assertEqual(doc["label"], "guest")
        log = (self.root / "gssh.log").read_text()
        self.assertIn("sudo mount -t virtiofs -o ro kfllm /mnt/kf-llm", log)

    def test_an_unreachable_guest_is_named(self):
        self.assertIn("KF_LLM_ARM_FAIL reason=guest_unreachable", self.hook("down"))

    def test_an_unreachable_guest_is_asked_once_and_nothing_else(self):
        self.hook("down")
        self.assertEqual((self.root / "gssh.log").read_text().splitlines(), ["true"])

    def test_a_mount_that_is_the_wrong_type_writable_or_not_readonly_is_refused(self):
        for mode in ("wrongfs", "rw", "writable"):
            text = self.hook(mode)
            self.assertIn("KF_LLM_ARM_FAIL reason=guest_mount", text, mode)
            self.assertNotIn("KF_LLM_ARM_RESULT", text)

    def test_too_little_guest_disk_is_named(self):
        self.assertIn("KF_LLM_ARM_FAIL reason=guest_disk", self.hook("smalldisk"))

    def test_a_guest_whose_df_prints_nothing_counts_as_having_no_space(self):
        self.stub.write_text(self.stub.read_text().replace('print("1" + "0" * (3 if mode == "smalldisk" else 12))', 'print("")'))
        self.assertIn("KF_LLM_ARM_FAIL reason=guest_disk", self.hook())

    def test_a_wrong_filesystem_type_is_refused_on_its_own(self):
        self.assertIn("KF_LLM_ARM_FAIL reason=guest_mount", self.hook("wrongfs"))

    def test_a_mount_without_the_ro_option_is_refused_on_its_own(self):
        self.assertIn("KF_LLM_ARM_FAIL reason=guest_mount", self.hook("rw"))

    def test_a_share_the_guest_can_write_to_is_refused_on_its_own(self):
        self.assertIn("KF_LLM_ARM_FAIL reason=guest_mount", self.hook("writable"))

    def test_the_arm_is_sent_to_an_isolated_interpreter_on_stdin(self):
        self.hook()
        self.assertTrue(any(l.startswith("python3 -I - --label guest ") for l in (self.root / "gssh.log").read_text().splitlines()))

    def boot_stub(self, body):
        root = self.root / "co"
        (root / "scripts/bench").mkdir(parents=True)
        (root / "scripts/bench/gssh_nv").write_text("#!/bin/sh\n")
        stub = root / "scripts/bench/boot_capture.sh"
        stub.write_text("#!/usr/bin/env bash\n" + body)
        return root

    def run_guest(self, root, transport="9p", **kw):
        share = self.root / "share"
        share.mkdir(exist_ok=True)
        os.chmod(share, 0o755)
        return lp.run_guest_arm(root, self.root / "work", share, transport, kw.get("vfsd"), 65534, "lptest", lambda line: None, boot_timeout=60)

    def test_the_boot_gets_the_share_arguments_after_the_double_dash_and_the_hook_in_the_environment(self):
        record = self.root / "boot.rec"
        root = self.boot_stub(f'printf "%s\\n" "$@" > {record}; echo "POST=$POST_CAPTURE_HOOK DEV=$KF_DEVICE CFG=$LLM_PARITY_HOOK_CFG" >> {record}; '
                              f'echo "KF_LLM_ARM_FAIL reason=x" > {self.root / "work" / "guest_arm.out"}\n')
        self.run_guest(root)
        lines = record.read_text().splitlines()
        self.assertEqual(lines[:2], ["lptest", "--"])
        self.assertEqual(lines[2], "-fsdev")
        self.assertIn("security_model=none,readonly=on", lines[3])
        self.assertIn("llm_parity_hook.sh", lines[-1])
        self.assertIn("DEV=kf3", lines[-1])
        self.assertIn("work/guest_cfg.json", lines[-1])

    def test_a_boot_that_fails_without_a_result_is_named(self):
        root = self.boot_stub("exit 3\n")
        with self.assertRaises(lp.StageError) as cm:
            self.run_guest(root)
        self.assertEqual(cm.exception.reason, "boot_failed")

    def test_a_boot_that_never_ends_is_named_and_the_daemon_stopped(self):
        root = self.boot_stub("exec sleep 30\n")
        share = self.root / "share"
        share.mkdir(exist_ok=True)
        os.chmod(share, 0o755)
        with self.patched_argv(self.fake_daemon()), self.assertRaises(lp.StageError) as cm:
            lp.run_guest_arm(root, self.root / "work", share, "virtiofs", "/opt/virtiofsd", 65534, "lptest", lambda line: None, boot_timeout=1)
        self.assertEqual((cm.exception.reason, cm.exception.detail), ("boot_failed", "timeout"))
        pid = int((self.root / "work/vfs/kfllm.sock.pid").read_text())
        with self.assertRaises(ProcessLookupError):
            for _ in range(50):
                os.kill(pid, 0)
                __import__("time").sleep(0.1)

    def test_a_boot_that_finishes_without_the_hooks_file_is_named(self):
        root = self.boot_stub("exit 0\n")
        with self.assertRaises(lp.StageError) as cm:
            self.run_guest(root)
        self.assertEqual(cm.exception.reason, "guest_no_result")

    def fake_daemon(self, mode="ok"):
        daemon = self.root / "fakevfsd.py"
        daemon.write_text(textwrap.dedent(f'''\
            import os, socket, sys, time
            sock = [a.split("=", 1)[1] for a in sys.argv if a.startswith("--socket-path=")][0]
            if {mode!r} == "dies": sys.exit(7)
            if {mode!r} == "slow": time.sleep(100)
            s = socket.socket(socket.AF_UNIX); s.bind(sock); s.listen(1)
            open(sock + ".pid", "w").write(str(os.getpid()))
            time.sleep(100)
            '''))
        return daemon

    def patched_argv(self, daemon):
        def argv(binary, socket_path, shared_dir, uid, gid=None):
            return [sys.executable, str(daemon), f"--socket-path={socket_path}", f"--shared-dir={shared_dir}"]
        return patch.object(lp, "virtiofsd_argv", argv)

    def default_guest_step(self, binary_mode):
        work = self.root / "work"
        binary = work / "dl" / "llm" / "virtiofsd"
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_text("#!/bin/sh\n")
        os.chmod(binary.parent, binary_mode)
        pins = pinned()
        pins["items"]["virtiofsd"]["dest"] = "llm/virtiofsd"
        calls = []
        args = type("A", (), dict(checkout=str(self.root), vfsd_uid=65534))()
        with patch.object(lp, "run_guest_arm", lambda *a, **k: calls.append(a) or "ok"):
            try:
                return lp.run_guest_arm_default(args, "virtiofs", work, self.root / "share", pins), calls
            finally:
                os.chmod(binary.parent, 0o755)

    def test_the_daemons_own_binary_must_be_reachable_by_its_uid(self):
        os.chmod(self.root / "work", 0o755)
        with self.assertRaises(lp.StageError) as cm:
            self.default_guest_step(0o700)
        self.assertEqual(cm.exception.reason, "share_not_traversable")

    def test_a_reachable_daemon_binary_is_made_executable_and_handed_on(self):
        os.chmod(self.root / "work", 0o755)
        result, calls = self.default_guest_step(0o755)
        self.assertEqual(result, "ok")
        self.assertEqual(stat.S_IMODE((self.root / "work/dl/llm/virtiofsd").stat().st_mode), 0o755)
        self.assertEqual(calls[0][4], str(self.root / "work/dl/llm/virtiofsd"))

    def test_virtiofsd_is_started_before_the_boot_and_stopped_after_it(self):
        record = self.root / "boot.rec"
        root = self.boot_stub(f'printf "%s\\n" "$@" > {record}; ls {self.root / "work/vfs"} >> {record}; echo "KF_LLM_ARM_FAIL reason=x" > {self.root / "work" / "guest_arm.out"}\n')
        with self.patched_argv(self.fake_daemon()):
            self.run_guest(root, "virtiofs", vfsd="/opt/virtiofsd")
        text = record.read_text()
        self.assertIn("vhost-user-fs-pci,chardev=kfllmc,tag=kfllm", text)
        self.assertIn("kfllm.sock", text)
        self.assertEqual(text.splitlines()[1], "--")
        self.assertEqual(stat.S_IMODE((self.root / "work/vfs").stat().st_mode), 0o700)
        pid = int((self.root / "work/vfs/kfllm.sock.pid").read_text())
        with self.assertRaises(ProcessLookupError):
            for _ in range(50):
                os.kill(pid, 0)
                __import__("time").sleep(0.1)

    def test_virtiofsd_is_stopped_when_the_boot_fails(self):
        root = self.boot_stub("exit 9\n")
        with self.patched_argv(self.fake_daemon()), self.assertRaises(lp.StageError) as cm:
            self.run_guest(root, "virtiofs", vfsd="/opt/virtiofsd")
        self.assertEqual(cm.exception.reason, "boot_failed")
        pid = int((self.root / "work/vfs/kfllm.sock.pid").read_text())
        with self.assertRaises(ProcessLookupError):
            for _ in range(50):
                os.kill(pid, 0)
                __import__("time").sleep(0.1)

    def test_a_daemon_that_dies_stops_the_stage_before_any_boot(self):
        record = self.root / "boot.rec"
        root = self.boot_stub(f"touch {record}\n")
        with self.patched_argv(self.fake_daemon("dies")), self.assertRaises(lp.StageError) as cm:
            self.run_guest(root, "virtiofs", vfsd="/opt/virtiofsd")
        self.assertEqual((cm.exception.reason, cm.exception.detail.split()[0]), ("virtiofsd_failed", "exited"))
        self.assertFalse(record.exists())

    def test_a_dead_daemon_is_noticed_at_once_not_after_the_timeout(self):
        import time

        root = self.boot_stub("true\n")
        t0 = time.monotonic()
        with self.patched_argv(self.fake_daemon("dies")), self.assertRaises(lp.StageError):
            self.run_guest(root, "virtiofs", vfsd="/opt/virtiofsd")
        self.assertLess(time.monotonic() - t0, 5)

    def test_a_daemon_that_never_makes_its_socket_is_named_after_the_wait(self):
        root = self.boot_stub("true\n")
        with patch.object(lp, "VFSD_SOCKET_WAIT_S", 1), self.patched_argv(self.fake_daemon("slow")), self.assertRaises(lp.StageError) as cm:
            self.run_guest(root, "virtiofs", vfsd="/opt/virtiofsd")
        self.assertEqual((cm.exception.reason, cm.exception.detail), ("virtiofsd_failed", "no socket"))

    def test_the_socket_directory_is_private(self):
        root = self.boot_stub(f'echo "KF_LLM_ARM_FAIL reason=x" > {self.root / "work" / "guest_arm.out"}\n')
        with self.patched_argv(self.fake_daemon()):
            self.run_guest(root, "virtiofs", vfsd="/opt/virtiofsd")
        self.assertEqual(stat.S_IMODE((self.root / "work/vfs").stat().st_mode), 0o700)


class FetchStep(unittest.TestCase):
    """fetch_items runs the real box downloader as a subprocess on the manifest the pins produce."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(os.path.realpath(self.tmp.name))
        import hashlib

        self.files = {"model": b"GGUF" + b"3" * 500, "ollama": b"TGZ" + b"4" * 700}
        self.pins = pinned(ollama_version="0.0.1")
        for n, data in self.files.items():
            self.pins["items"][n].update(sha256=hashlib.sha256(data).hexdigest(), size=len(data),
                                         sources=[{"kind": "mirror", "url": "https://127.0.0.1:1/" + n}])
        self.pins["items"]["virtiofsd"].update(sha256="UNSET", size="UNSET", sources=[])
        self.pin_dir = self.root / "pins"
        self.pin_dir.mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def args(self):
        return type("A", (), dict(pin_dir=[str(self.pin_dir)], urls=None, s3_config=None, limit_bandwidth=None))()

    def test_a_local_pin_that_matches_is_used_and_nothing_is_fetched_from_the_network(self):
        for n, data in self.files.items():
            (self.pin_dir / n).write_bytes(data)  # the downloader looks a local pin up by the item's name
        manifest = lp.build_manifest(self.pins, ("model", "ollama"))
        lines = []
        lp.fetch_items(manifest, self.root / "work" / "dl", self.args(), lines.append)
        self.assertEqual((self.root / "work/dl/llm/model.gguf").read_bytes(), self.files["model"])
        self.assertEqual((self.root / "work/dl/llm/ollama.tgz").read_bytes(), self.files["ollama"])
        self.assertTrue((self.root / "work/llm_parity_manifest.json").exists())

    def test_a_download_that_cannot_complete_is_named_and_leaves_no_verified_file(self):
        manifest = lp.build_manifest(self.pins, ("model", "ollama"))
        with self.assertRaises(lp.StageError) as cm:
            lp.fetch_items(manifest, self.root / "work" / "dl", type("A", (), dict(pin_dir=[], urls=None, s3_config=None, limit_bandwidth=None))(), lambda line: None)
        self.assertEqual(cm.exception.reason, "download_failed")
        self.assertFalse((self.root / "work/dl/llm/model.gguf").exists())

    def test_a_nonzero_downloader_exit_is_a_download_failure(self):
        manifest = lp.build_manifest(self.pins, ("model", "ollama"))
        with patch.object(lp.subprocess, "run", lambda cmd, **kw: type("R", (), {"returncode": 1})()):
            with self.assertRaises(lp.StageError) as cm:
                lp.fetch_items(manifest, self.root / "work" / "dl", self.args(), lambda line: None)
        self.assertEqual((cm.exception.reason, cm.exception.detail), ("download_failed", "rc=1"))

    def test_the_downloader_runs_in_isolated_mode(self):
        manifest = lp.build_manifest(self.pins, ("model", "ollama"))
        seen = []
        with patch.object(lp.subprocess, "run", lambda cmd, **kw: seen.append(cmd) or type("R", (), {"returncode": 0})()):
            lp.fetch_items(manifest, self.root / "work" / "dl", self.args(), lambda line: None)
        self.assertEqual(seen[0][1], "-I")

    def test_the_downloader_command_line_carries_no_url_or_credential(self):
        manifest = lp.build_manifest(self.pins, ("model", "ollama"))
        seen = []
        with patch.object(lp.subprocess, "run", lambda cmd, **kw: seen.append(cmd) or type("R", (), {"returncode": 0})()):
            lp.fetch_items(manifest, self.root / "work" / "dl", self.args(), lambda line: None)
        text = " ".join(seen[0])
        self.assertNotIn("https://", text)
        self.assertIn("box_downloader.py", text)
        self.assertEqual(seen[0][:3], [sys.executable, "-I", str(HERE / "box_downloader.py")])


class PayloadIntegration(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("vast_payload_lp", HERE / "vast_payload.py")
        self.m = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.m)

    def inst(self, **over):
        doc = dict(tier="large", windows=False, windows_runs=1, stages=(), spec=None, winapps=None, linux_apps=None, rehearsal=False)
        doc.update(over)
        return pi.Instruction(**doc)

    def test_the_stage_is_not_in_any_default_plan(self):
        m = self.m
        self.assertFalse(m.LLM_PARITY_IN_LARGE)
        for tier in ("small", "middle", "large", "adhoc"):
            self.assertNotIn("llm_parity", m.plan_stage_names(self.inst(tier=tier)))
            self.assertNotIn("llm_parity", m.plan_stage_names(self.inst(tier=tier, windows=tier == "large", linux_apps="middle" if tier != "small" else None)))

    def test_only_selects_it_in_the_large_tier_and_nowhere_else(self):
        m = self.m
        self.assertEqual(m.plan_stage_names(self.inst(), ("llm_parity",))[-1], "llm_parity")
        for tier in ("small", "middle", "adhoc"):
            self.assertNotIn("llm_parity", m.plan_stage_names(self.inst(tier=tier), ("llm_parity",)))

    def test_it_runs_after_the_linux_guests_and_before_the_windows_gate(self):
        names = self.m.plan_stage_names(self.inst(windows=True, linux_apps="middle"), ("llm_parity",))
        self.assertEqual(names[names.index("llm_parity") - 1], "apps")
        self.assertEqual(names[names.index("llm_parity") + 1], "windows_gate")

    def test_the_flag_puts_it_in_the_large_plan_only(self):
        with patch.object(self.m, "LLM_PARITY_IN_LARGE", True):
            self.assertIn("llm_parity", self.m.plan_stage_names(self.inst()))
            self.assertNotIn("llm_parity", self.m.plan_stage_names(self.inst(tier="middle")))

    def test_select_stages_refuses_it_when_it_is_not_planned(self):
        m = self.m
        self.assertEqual(m.select_stages(m.plan_stage_names(self.inst(), ("llm_parity",)), ["llm_parity"], []), ["llm_parity"])
        with self.assertRaises(m.EvidenceError):
            m.select_stages(m.plan_stage_names(self.inst()), ["llm_parity"], [])

    def test_kf_stages_still_refuses_the_stage(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.tier_requested({"KF_TIER": "large", "KF_STAGES": "llm_parity"}, False)

    def test_the_workers_stage_slot_is_still_design_only(self):
        m = self.m
        self.assertEqual(m.STAGE_SLOTS["llm_parity"], "DESIGN_ONLY")
        defaults = json.loads((ROOT / "ci/vast-broker/defaults.json").read_text())
        self.assertEqual(defaults["stageSlots"]["llm_parity"]["state"], "DESIGN_ONLY")

    def test_the_stage_needs_an_hour_left(self):
        self.assertGreaterEqual(self.m.LLM_PARITY_MIN_REMAINING_MS, 3600 * 1000)

    def test_dry_run_prints_the_plan_and_touches_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "i.json"
            pi.write(self.inst(), str(path))
            env = {k: v for k, v in os.environ.items() if not k.startswith("KF_")}
            cmd = [sys.executable, str(HERE / "vast_payload.py"), "--dry-run", "--instruction", str(path)]
            ok = subprocess.run([*cmd, "--only", "llm_parity"], env=env, capture_output=True, text=True, cwd=d)
            self.assertEqual(ok.returncode, 0, ok.stderr)
            self.assertIn("KF_PAYLOAD_PLAN stages=llm_parity planned=provision:cuda_host:merge_bar:cuda_guest:llm_parity", ok.stdout)
            plain = subprocess.run(cmd, env=env, capture_output=True, text=True, cwd=d)
            self.assertIn("planned=provision:cuda_host:merge_bar:cuda_guest\n", plain.stdout)

    def line(self, **over):
        rec = lp.build_record(SHA, "virtiofs", pinned(), host=arm_doc("host"), guest=arm_doc("guest"))
        rec.update(over)
        return "KF_LLM_PARITY_RESULT " + json.dumps(rec) + "\n"

    def test_rows_accept_a_green_record_and_keep_it(self):
        result = {}
        rows = self.m.llm_parity_rows(self.line(), 0, SHA, result)
        self.assertEqual(rows, [{"name": "llm_parity", "pass": 1, "fail": 0, "skip": 0}])
        self.assertEqual(result["llm_parity"]["ratio"]["value"], 1.0)

    def test_rows_never_read_the_ratio_value(self):
        for ratio in (0.01, 0.5, 5.0):
            rec = lp.build_record(SHA, "virtiofs", pinned(), host=arm_doc("host"), guest=arm_doc("guest"))
            rec["ratio"]["value"] = ratio
            rec["ratio"]["meets_min"] = ratio >= 0.9
            self.m.llm_parity_rows("KF_LLM_PARITY_RESULT " + json.dumps(rec) + "\n", 0, SHA, {})

    def refuses(self, text, rc, sha):
        with self.assertRaises(self.m.EvidenceError):
            self.m.llm_parity_rows(text, rc, sha, {})

    def test_rows_refuse_a_red_verdict(self):
        self.refuses(self.line(verdict="FAIL"), 0, SHA)

    def test_rows_refuse_a_nonzero_exit_with_a_green_record(self):
        self.refuses(self.line(), 1, SHA)

    def test_rows_refuse_a_record_of_another_commit(self):
        self.refuses(self.line(), 0, "b" * 40)

    def test_rows_refuse_two_result_lines_even_when_both_are_green(self):
        self.refuses(self.line() + self.line(), 0, SHA)

    def test_rows_refuse_a_missing_or_unparseable_line(self):
        self.refuses("", 0, SHA)
        self.refuses("KF_LLM_PARITY_RESULT {not json}\n", 0, SHA)

    def test_rows_refuse_a_red_record_then_a_green_one(self):
        self.refuses(self.line(verdict="FAIL") + self.line(), 0, SHA)

    def test_rows_refuse_every_bad_shape_in_one_sweep(self):
        m = self.m
        bad = [(self.line(verdict="FAIL"), 0, SHA), (self.line(), 1, SHA), (self.line(), 0, "b" * 40), (self.line() + self.line(), 0, SHA),
               ("", 0, SHA), ("KF_LLM_PARITY_RESULT {not json}\n", 0, SHA), (self.line(verdict="FAIL") + self.line(), 0, SHA)]
        for text, rc, sha in bad:
            with self.assertRaises(m.EvidenceError):
                m.llm_parity_rows(text, rc, sha, {})

    def test_rows_keep_a_red_record_in_the_result(self):
        result = {}
        with self.assertRaises(self.m.EvidenceError):
            self.m.llm_parity_rows(self.line(verdict="FAIL", reason="text_disagree"), 1, SHA, result)
        self.assertEqual(result["llm_parity"]["reason"], "text_disagree")

    def test_rows_accept_a_record_whose_ratio_is_far_below_the_target(self):
        guest = arm_doc("guest")
        for r in guest["runs"]:
            r["eval_duration_ns"] = 10**11
        rec = lp.build_record(SHA, "virtiofs", pinned(), host=arm_doc("host"), guest=guest)
        self.assertEqual((rec["verdict"], rec["ratio"]["value"]), ("PASS", 0.1))
        result = {}
        self.m.llm_parity_rows("KF_LLM_PARITY_RESULT " + json.dumps(rec) + "\n", 0, SHA, result)
        self.assertEqual(result["llm_parity"]["ratio"]["meets_min"], False)

    def test_the_runner_the_payload_names_accepts_the_arguments_the_payload_passes(self):
        import re

        source = (HERE / "vast_payload.py").read_text()
        found = re.search(r'"python3", str\(root / "(scripts/ci/[a-z_]+\.py)"\), "run", "--sha", sha, "--checkout", str\(root\),\s+"--work", LLM_PARITY_WORK, "--out", str\(evidence / "llm_parity.json"\)', source)
        self.assertIsNotNone(found)
        self.assertEqual(found.group(1), "scripts/ci/llm_parity.py")
        args = lp.build_parser().parse_args(["run", "--sha", SHA, "--checkout", "/c", "--work", self.m.LLM_PARITY_WORK, "--out", "/o"])
        self.assertEqual(args.work, self.m.LLM_PARITY_WORK)

    def test_the_reporter_reads_a_record_the_stage_built(self):
        rec = lp.build_record(SHA, "virtiofs", pinned(), host=arm_doc("host"), guest=arm_doc("guest"))
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "r.json"
            path.write_text(json.dumps({"verdict": "PASS", "suites": [{"name": "a", "pass": 1, "fail": 0, "skip": 0}], "llm_parity": rec}))
            self.assertEqual(vast_report.read_result_file(path)["llm_parity"], rec)

    def test_the_reporter_refuses_a_malformed_record_in_a_result_file(self):
        rec = lp.build_record(SHA, "virtiofs", pinned(), host=arm_doc("host"), guest=arm_doc("guest"))
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "r.json"
            rec["extra"] = 1
            path.write_text(json.dumps({"verdict": "PASS", "suites": [{"name": "a", "pass": 1, "fail": 0, "skip": 0}], "llm_parity": rec}))
            with self.assertRaises(vast_report.ResultError):
                vast_report.read_result_file(path)

    def test_the_reporter_refuses_a_gate_flag_in_a_result_file(self):
        rec = lp.build_record(SHA, "virtiofs", pinned(), host=arm_doc("host"), guest=arm_doc("guest"))
        rec["ratio"]["gate"] = True
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "r.json"
            path.write_text(json.dumps({"verdict": "PASS", "suites": [{"name": "a", "pass": 1, "fail": 0, "skip": 0}], "llm_parity": rec}))
            with self.assertRaises(vast_report.ResultError):
                vast_report.read_result_file(path)

    def test_the_stage_is_built_by_main_with_the_runner_and_its_arguments(self):
        source = (HERE / "vast_payload.py").read_text()
        self.assertIn('str(root / "scripts/ci/llm_parity.py"), "run", "--sha", sha', source)
        self.assertIn('stage.update(min_remaining_ms=LLM_PARITY_MIN_REMAINING_MS, short_row="llm_parity_retry_time")', source)


class Budget(unittest.TestCase):
    """scripts/ci/profile_resources.py: a manual-only stage needs no budget row, and only while it is out of every default plan."""

    def setUp(self):
        import profile_resources

        self.pr = profile_resources
        self.data = json.loads((ROOT / "ci/vast-broker/defaults.json").read_text())
        self.source = (HERE / "vast_payload.py").read_text()

    def test_the_live_payload_and_the_live_table_agree(self):
        self.assertEqual(self.pr.validate(self.data, self.source), [])
        self.assertNotIn("llm_parity", self.data["payloadStageResources"])

    def test_the_manual_only_table_names_the_flag_and_the_flag_is_false(self):
        self.assertEqual(self.pr.manual_only(self.source), ({"llm_parity": "LLM_PARITY_IN_LARGE"}, {"LLM_PARITY_IN_LARGE": False}))

    def test_the_flag_being_true_while_listed_manual_only_is_refused(self):
        errors = self.pr.validate(self.data, self.source.replace("LLM_PARITY_IN_LARGE = False", "LLM_PARITY_IN_LARGE = True"))
        self.assertIn("manual-only stage is in a default plan: llm_parity", errors)

    def test_a_stage_that_is_neither_budgeted_nor_manual_only_is_refused(self):
        errors = self.pr.validate(self.data, self.source.replace('MANUAL_ONLY_STAGES = {"llm_parity": "LLM_PARITY_IN_LARGE"}', "MANUAL_ONLY_STAGES = {}"))
        self.assertEqual(errors, ["unbudgeted payload stage: llm_parity"])

    def test_a_manual_only_stage_that_also_has_a_row_is_refused(self):
        data = json.loads(json.dumps(self.data))
        data["payloadStageResources"]["llm_parity"] = dict(data["payloadStageResources"]["specviewperf"], profiles=[])
        self.assertIn("manual-only stage has a budget row: llm_parity", self.pr.validate(data, self.source))


if __name__ == "__main__":
    unittest.main()
