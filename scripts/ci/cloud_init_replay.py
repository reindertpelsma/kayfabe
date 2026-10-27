#!/usr/bin/env python3
"""Disposable Ubuntu22 replay; GPU/VM/compiler commands are explicit fixtures.

Run only inside a fresh --network=none container with the reviewed blobs at /fixtures,
this checkout at /workspace/kayfabe, and KF_CLOUD_INIT_REPLAY=1. Never a GPU verdict.
The real entry/setup/provision_box/provision_full/driver/bench/CUDA/merge scripts run.
Kernel/VM/build leaf operations use the fixtures below; APT installs real cached debs.
"""

import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import time
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
STATE = Path("/run/replay")


def fixture_command(name, args):
    """Only explicitly listed hardware/build operations may be simulated."""
    with (STATE / "commands.jsonl").open("a") as log:
        log.write(json.dumps({"name": name, "args": args}) + "\n")
    driver = (STATE / "driver").read_text().strip()
    if name == "nvidia-smi":
        query = next((a for a in args if a.startswith("--query-gpu=")), "")
        if query == "--query-gpu=name,compute_cap,driver_version":
            print("NVIDIA GeForce RTX 3090, 8.6, " + driver)
        elif query == "--query-gpu=driver_version":
            print(driver)
        elif query == "--query-gpu=memory.used":
            print("0")
        else:
            print("NVIDIA GeForce RTX 3090, " + driver)
    elif name in ("cat", "grep") and "/proc/driver/nvidia/version" in args:
        text = "NVRM version: NVIDIA UNIX Open Kernel Module " + driver + " fixture\n"
        if name == "cat":
            print(text, end="")
        else:
            return subprocess.run(
                ["/usr/bin/grep", *[a for a in args if a != "/proc/driver/nvidia/version"]],
                input=text,
                text=True,
            ).returncode
    elif name in ("cat", "grep"):
        os.execv("/usr/bin/" + name, [name, *args])
    elif name == "dmesg":
        print("[0.0] replay kernel fixture\n[0.1] replay ready")
    elif name in (
        "modprobe",
        "rmmod",
        "depmod",
        "nvidia-modprobe",
        "ip",
        "iptables",
        "sysctl",
        "scp",
    ):
        return 0
    elif name == "lsmod":
        print("Module Size Used by")
    elif name == "modinfo":
        print("version: " + driver + "\nlicense: Dual MIT/GPL")
    elif name == "cargo":
        assert os.environ.get("HOME") == "/root", "HOME missing in real provisioner"
        assert not os.environ.get("USER") and not os.environ.get("LOGNAME")
        assert not os.isatty(0)
        assert os.environ.get("CARGO_NET_OFFLINE") == "true"
        assert Path("/root/.cargo/config.toml").is_file(), "vendor config absent"
        target = Path(os.environ["CARGO_TARGET_DIR"])
        assert target == ROOT / "target", "Cargo output path drift"
        if "--target" in args and (STATE / "scenario").read_text() == "swap-failure":
            return 66
        if args[0] == "test":
            print("test result: ok. 123 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out")
        else:
            if "--target" in args:
                artifacts = ["x86_64-unknown-linux-musl/release/kayfabe-rm-ladder"]
            elif "kayfabe-rm-ladder" in args:
                artifacts = ["release/kayfabe-rm-ladder"]
            else:
                artifacts = ["release/libkf_qemu.a"]
            for relative in artifacts:
                dest = target / relative
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_text("#!/bin/sh\nexit 0\n")
                dest.chmod(0o755)
    elif name == "rustup":
        if args[:2] == ["target", "list"]:
            print("x86_64-unknown-linux-musl\nx86_64-unknown-uefi")
    elif name == "curl":
        if "--config" in args:
            config = sys.stdin.read()
            assert "Authorization: Bearer " in config
            asset = args[-1].rsplit("/", 1)[1]
            lock = json.loads((ROOT / "scripts/ci/setup-cache.lock.json").read_text())
            meta = next(a for a in lock["assets"] if a["name"] == asset)
            print(
                json.dumps(
                    {
                        "url": "https://release-assets.githubusercontent.com/replay/" + asset,
                        "size": meta["size"],
                        "name": asset,
                    }
                )
            )
        elif "--write-out" in args:
            print("206 8388608 0.2")
        elif "-o" in args:
            dest = Path(args[args.index("-o") + 1])
            dest.parent.mkdir(parents=True, exist_ok=True)
            if ".run" in args[-1]:
                dest.write_text(
                    (
                        "#!/bin/sh\necho 580.159.04 > /run/replay/driver\n"
                        "echo fixture > /var/log/nvidia-installer.log\n"
                    )
                )
            elif "qemu-" in args[-1]:
                with tarfile.open(dest, "w:xz") as tar:
                    for suffix, data in [
                        ("VERSION", b"10.2.4"),
                        ("hw/misc/meson.build", b""),
                        ("hw/misc/Kconfig", b""),
                    ]:
                        item = tarfile.TarInfo("qemu-10.2.4/" + suffix)
                        item.size = len(data)
                        tar.addfile(item, io.BytesIO(data))
            elif "cloudimg" in args[-1]:
                dest.write_bytes(b"guest image fixture")
            else:
                raise AssertionError("unexpected curl URL")
        else:
            raise AssertionError("unexpected curl operation")
    elif name == "ssh":
        cmd = args[-1]
        if "cloud-init status" in cmd:
            print("status: done")
        elif "MIRROR" in cmd:
            print("MIRROR archive.ubuntu.com/ubuntu 2000000")
        elif "CC_PRESENT" in cmd:
            print("CC_PRESENT")
        elif "grep -c CUresult" in cmd:
            print("601")
        elif "modinfo nvidia" in cmd:
            print("version: 580.159.04 license: Dual MIT/GPL vermagic: replay")
    elif name in ("qemu-img", "cloud-localds"):
        if name == "cloud-localds":
            Path(args[0]).write_bytes(b"fixture seed")
        elif args[0] == "convert":
            Path(args[-1]).write_bytes(b"fixture qcow")
    elif name == "qemu-system-x86_64":
        # Real bench script only needs to own a process it can terminate after guest shutdown.
        if "-pidfile" in args:
            child = subprocess.Popen(["/bin/sleep", "30"])
            Path(args[args.index("-pidfile") + 1]).write_text(str(child.pid))
    elif name == "gcc":
        if "-o" not in args:
            os.execv("/usr/bin/gcc", ["gcc", *args])
        dest = Path(args[args.index("-o") + 1])
        dest.parent.mkdir(parents=True, exist_ok=True)
        lines = {
            "cup2": "CE rv=0xabcd1234 want=0xabcd1234 -> PASS",
            "cup3": "KERNEL rv=43 fixture",
            "cup8": "CUP8 RESULT bad=0 maxerr=0 fixture",
            "cup8bench": "BENCH_TOTAL_BAD=0\nBENCH_VERDICT: PASS",
        }
        dest.write_text("#!/bin/sh\ncat <<'EOF'\n" + lines[dest.name] + "\nEOF\n")
        dest.chmod(0o755)
    elif name == "bash":
        script = Path(args[0]).name if args else ""
        if script == "build_kf3.sh":
            dest = Path(args[2]) / "qemu-system-x86_64"
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text("#!/bin/sh\nexit 0\n")
            dest.chmod(0o755)
            print("KF3_BUILT " + str(dest) + " rev=fixture")
        elif script == "build_fast_guest.sh":
            dest = Path(args[2])
            dest.mkdir(parents=True, exist_ok=True)
            (dest / "initrd.cpio.gz").write_bytes(b"fixture initrd")
        elif script == "v3_gates.sh":
            Path(args[1]).write_text("V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS\n")
        elif script == "bare_metal_suite.sh":
            print("BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30")
        elif script == "fast_suite.sh":
            Path("/workspace/bench/" + args[1] + "_suite.out").write_text(
                ("FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 " "FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30\n")
            )
        elif script == "birth_census.sh":
            if args[1] == "--selftest":
                print("BIRTH_CENSUS_SELFTEST_OK")
            else:
                print(
                    (
                        "BIRTH_CENSUS_SUITE fixture\nBIRTH_CENSUS_GATES fixture\n"
                        "BIRTH_CENSUS_OK arms=30 fixture"
                    )
                )
        elif script == "boot_capture.sh":
            bench = Path("/workspace/bench")
            tag = args[1]
            (bench / ("run_" + tag + "_probe.log")).write_text(
                (
                    "CE rv=0xabcd1234 want=0xabcd1234 -> PASS\nCUP3_VAL=43\n"
                    "CUP8_BAD=0\nCUP8_MAXERR=0\nGUEST_BENCH_TOTAL_BAD=0\n"
                    "GUEST_BENCH_VERDICT=PASS\n"
                )
            )
            (bench / ("run_" + tag + "_qemu.log")).write_text("fixture\n")
        else:
            os.execv("/bin/bash", ["bash", *args])
    else:
        raise AssertionError("unrecorded command " + name)
    return 0


def python_entry(args):
    target = next((a for a in args if a.endswith(".py")), "")
    if target.endswith("vast_setup.py") and "restore" in args:
        sys.path.insert(0, str(ROOT / "scripts/ci"))
        import vast_setup as setup

        class Files:
            def open(self, request, timeout):
                assert request.full_url.startswith(
                    "https://release-assets.githubusercontent.com/replay/"
                )
                assert not request.has_header("Authorization")
                response = open("/fixtures/" + request.full_url.rsplit("/", 1)[1], "rb")
                response.status = 200
                return response

        sys.argv = args[args.index(target) :]
        with patch.object(setup.urllib.request, "build_opener", return_value=Files()):
            return setup.main()
    if target.endswith("vast_payload.py") and "--grade-xid" not in args:
        sys.path.insert(0, str(ROOT / "scripts/ci"))
        import vast_payload as payload

        sys.argv = args[args.index(target) :]
        disk = shutil.disk_usage("/")
        with patch.object(
            payload.shutil, "disk_usage", return_value=disk._replace(free=80_000_000_000)
        ):
            return payload.main()
    os.execv("/usr/bin/python3", ["python3", *args])


def replay():
    assert os.geteuid() == 0 and Path("/.dockerenv").exists()
    assert ROOT == Path("/workspace/kayfabe"), "requires isolated container checkout"
    assert os.environ.get("KF_CLOUD_INIT_REPLAY") == "1"
    assert not Path("/run/systemd/system").exists()
    assert not any(
        line.split()[1] == "00000000"
        for line in Path("/proc/net/route").read_text().splitlines()[1:]
    )
    scenario = sys.argv[1] if len(sys.argv) == 2 else "pass"
    assert scenario in ("pass", "swap-failure")
    STATE.mkdir(parents=True, exist_ok=False)
    (STATE / "scenario").write_text(scenario)
    (STATE / "driver").write_text("575.51.03\n")
    for asset in json.loads((ROOT / "scripts/ci/setup-cache.lock.json").read_text())["assets"]:
        data = Path("/fixtures", asset["name"]).read_bytes()
        assert len(data) == asset["size"] and hashlib.sha256(data).hexdigest() == asset["sha256"]
    # Fixtures live on a separate path from installed package tools; no mocked APT.
    for name in (
        "python3",
        "bash",
        "curl",
        "cargo",
        "rustup",
        "nvidia-smi",
        "cat",
        "grep",
        "dmesg",
        "modprobe",
        "rmmod",
        "depmod",
        "nvidia-modprobe",
        "lsmod",
        "modinfo",
        "ip",
        "iptables",
        "sysctl",
        "scp",
        "ssh",
        "qemu-img",
        "cloud-localds",
        "qemu-system-x86_64",
        "gcc",
    ):
        dest = Path("/usr/local/bin", name)
        dest.write_text(
            "#!/usr/bin/python3\nimport sys\nsys.path.insert(0, "
            + repr(str(ROOT / "scripts/ci"))
            + ")\nimport cloud_init_replay as r\nraise SystemExit(r.python_entry(sys.argv[1:]) if "
            + repr(name)
            + ' == "python3" else r.fixture_command('
            + repr(name)
            + ",sys.argv[1:]))\n"
        )
        dest.chmod(0o755)
    cargo = Path("/root/.cargo")
    cargo.mkdir(exist_ok=True)
    (cargo / "env").write_text('export PATH="$HOME/.cargo/bin:$PATH"\n')
    Path("/dev/kvm").touch()
    for node in ("nvidiactl", "nvidia0", "nvidia-uvm"):
        Path("/dev", node).touch()
    bench = Path("/workspace/bench")
    bench.mkdir()
    (bench / "cuda.h").write_text("CUresult\n" * 601)
    subprocess.run(["git", "init", "-q", str(ROOT)], check=True)
    subprocess.run(["git", "-C", str(ROOT), "config", "user.name", "Fixture"], check=True)
    subprocess.run(
        ["git", "-C", str(ROOT), "config", "user.email", "fixture@example.invalid"], check=True
    )
    subprocess.run(["git", "-C", str(ROOT), "add", "."], check=True)
    subprocess.run(
        ["git", "-C", str(ROOT), "commit", "-qm", "recorded fixture checkout"], check=True
    )
    subprocess.run(["git", "-C", str(ROOT), "remote", "add", "origin", str(ROOT)], check=True)
    sha = subprocess.check_output(["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True).strip()
    sys.path.insert(0, str(ROOT / "scripts/ci"))
    import test_vast_box as box

    box.setUpModule()
    broker = box.Broker()
    try:
        env = box.good_env(broker.url)
        env["KF_SHA"] = sha
        Path("/etc/environment").write_text("\n".join(k + "=" + v for k, v in env.items()) + "\n")
        start_env = {
            "PATH": "/usr/local/bin:/usr/bin:/bin",
            "LC_ALL": "C.UTF-8",
            "SSL_CERT_FILE": box.CERT["cert"],
            "CURL_CA_BUNDLE": box.CERT["cert"],
            "KF_CI_REPORTER_ARGS": " ".join(box.FAST),
        }
        started = time.monotonic()
        with (STATE / "entry.log").open("w") as log:
            result = subprocess.run(
                ["/bin/bash", str(ROOT / "scripts/ci/vast_onstart.sh")],
                env=start_env,
                cwd="/",
                stdin=subprocess.DEVNULL,
                stdout=log,
                stderr=log,
                timeout=180,
            )
        doc = json.loads(Path("/workspace/kf-result.json").read_text())
        posts = broker.results()
        evidence = {
            "run": "v8-cloud-init-replay-20261023",
            "scenario": scenario,
            "seconds": round(time.monotonic() - started, 3),
            "entry_rc": result.returncode,
            "result": doc,
            "posted_results": posts,
            "host_environment": sorted(start_env),
            "hardware_and_builds": "recorded fixtures, not GPU evidence",
            "cache_sha_verified": True,
            "no_external_network": True,
        }
        Path("/output/replay.json").write_text(json.dumps(evidence, indent=2) + "\n")
        for path in [
            STATE / "entry.log",
            Path("/workspace/kf-ci-output.log"),
            *Path("/root/prov").glob("*.log"),
        ]:
            shutil.copyfile(path, Path("/output") / path.name)
            assert "command not found" not in path.read_text(errors="replace"), path
        assert result.returncode == (0 if scenario == "pass" else 1), (result.returncode, doc)
        assert posts and posts[-1]["verdict"] == ("PASS" if scenario == "pass" else "FAIL"), posts
        assert doc["hardware"]["driver_version"] == "580.159.04", doc
        expected = (
            [
                "provision",
                "cuda_host",
                "gates",
                "kf3_build",
                "raw_suite",
                "thin_suite",
                "birth_census",
                "cuda_fat",
            ]
            if scenario == "pass"
            else ["provision"]
        )
        assert [r["name"] for r in doc["suites"]] == expected, doc
        if scenario == "swap-failure":
            assert doc["suites"][0]["fail"] == 1
            assert (
                "KF_PAYLOAD_STAGE_START name=cuda"
                not in Path("/workspace/kf-ci-output.log").read_text()
            )
        print(json.dumps(evidence, indent=2))
    finally:
        broker.stop()
        box.tearDownModule()


if __name__ == "__main__":
    replay()
