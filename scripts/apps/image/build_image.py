#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Open-only apps image; official NVIDIA inputs belong to transient provisioning.

Host external programs: QEMU only. NVIDIA downloads use J4's stdlib kf_fetch API.
No hardware is used by plan, URL checks or the GPU-free fixtures.
"""

import argparse
import base64
from contextlib import contextmanager
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import secrets
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse

from common import (
    MAX_RESULT,
    canonical,
    copied_input_hashes,
    download,
    read_recipe,
    requirements,
    relative_path,
    sha256_file,
    validate_inventory,
    inventory_parts,
    inventory_observation,
    validate_recipe,
)
from iso9660 import make_iso
from channels import Channels
from apt_mirror import AptMirror
import official
from export_check import validate_export
import input_disk

HERE = Path(__file__).resolve().parent
RUNTIME = ("guest.py", "common.py", "elf.py", "scan_image.py", "input_disk.py")
TOOLS = RUNTIME + (
    "build_image.py",
    "iso9660.py",
    "channels.py",
    "apt_mirror.py",
    "official.py",
    "export_check.py",
)
MIN_FREE = 60 * 1024**3
MAX_LOG = 64 * 1024**2
MAX_EXPORT = 2 * 1024**3


def tooling(root=HERE):
    return {name: sha256_file(root / name) for name in TOOLS}


def local_files(recipe, root):
    for item in recipe["local_files"]:
        path = root.parents[2] / item["path"]
        if sha256_file(path) != item["sha256"]:
            raise ValueError("local input changed: " + item["path"])
        yield item, path


def cloud_config(
    recipe,
    root,
    nonce,
    mode,
    server_url,
    keys,
    allowed_packages=(),
    export_pin=None,
    official_device=False,
):
    install = mode in ("builder", "assemble")
    config = {
        "users": [
            {
                "name": recipe["guest"]["user"],
                "uid": recipe["guest"].get("uid", 1000),
                "groups": ["video", "render"],
                "lock_passwd": True,
                "sudo": ["ALL=(ALL) NOPASSWD:ALL"],
                "ssh_authorized_keys": list(keys),
            }
        ],
        "groups": ["video", "render"],
        "ssh_pwauth": False,
        "disable_root": True,
        "package_update": install,
        "package_upgrade": False,
        "packages": recipe["apt"]["packages"] if install else [],
        "growpart": {
            "mode": "auto",
            "devices": ["/"],
            "ignore_growroot_disabled": False,
        },
        "resize_rootfs": True,
        "cloud_final_modules": [
            "package-update-upgrade-install",
            "scripts-per-instance",
            "final-message",
        ],
        "write_files": [],
    }
    if install:
        config["apt"] = {
            "preserve_sources_list": False,
            "sources_list": "Types: deb\nURIs: "
            + server_url
            + "/ubuntu/\nSuites: "
            + " ".join(recipe["apt"]["suites"])
            + "\nComponents: main universe\nArchitectures: amd64\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\nCheck-Valid-Until: no\n",
            "conf": 'APT::Install-Recommends "false";\nAPT::Install-Suggests "false";\nAcquire::Languages "none";\nAcquire::IndexTargets::deb::DEP-11::DefaultEnabled "false";\n',
        }

    def write(path, data, permissions="0644"):
        config["write_files"].append(
            {"path": path, "content": data, "permissions": permissions}
        )

    for name in RUNTIME:
        write("/opt/apps/image/" + name, (root / name).read_text())
    write("/opt/apps/image/recipe.json", canonical(recipe).decode())
    write(
        "/opt/apps/image/allowed_packages.json",
        canonical(sorted(allowed_packages)).decode(),
    )
    session = {
        "nonce": nonce,
        "mode": mode,
        "recipe_sha256": hashlib.sha256(canonical(recipe)).hexdigest(),
        "server_url": server_url,
    }
    if official_device:
        session["official_disk"] = input_disk.DEVICE
        session["official_ids"] = [
            a["id"] for a in official_pins(recipe, builder=mode == "builder")
        ]
    if export_pin:
        session["export_pin"] = export_pin
    write("/opt/apps/image/session.json", canonical(session).decode(), "0600")
    for layer, filename in [
        ("open", "requirements.lock"),
        ("runtime", "requirements-runtime.lock"),
    ]:
        write("/opt/apps/image/" + filename, requirements(recipe, layer))
    write(
        "/var/lib/cloud/scripts/per-instance/kayfabe-apps-image",
        '#!/usr/bin/python3\nimport sys\nsys.path.insert(0,"/opt/apps/image")\nimport guest\nguest.main()\n',
        "0755",
    )
    if mode in ("builder", "provision"):
        for item, path in local_files(recipe, root):
            config["write_files"].append(
                {
                    "path": item["target"],
                    "encoding": "b64",
                    "content": base64.b64encode(path.read_bytes()).decode(),
                    "permissions": item.get("permissions", "0644"),
                }
            )
    if mode == "assemble":
        config["write_files"].extend(recipe["write_files"])
    if install:
        write(
            "/etc/apt/apt.conf.d/99-apps-no-periodic",
            'APT::Periodic::Enable "0";\nAPT::Periodic::Unattended-Upgrade "0";\n',
        )
    return config


def input_port(server_url):
    """Only the generated Slirp endpoint can forward to the host input server."""
    u = urllib.parse.urlsplit(server_url)
    if (
        u.scheme != "http"
        or u.hostname != "10.0.2.100"
        or u.port is None
        or u.username
        or u.password
        or u.path
        or u.query
        or u.fragment
    ):
        raise ValueError("invalid pinned input forward endpoint")
    return u.port


def seed(
    recipe,
    directory,
    nonce,
    mode,
    server_url="",
    keys=(),
    root=HERE,
    allowed_packages=(),
    export_pin=None,
    official_disk=None,
):
    disk_transport = Path(directory) / "official-disk.json"
    if official_disk is not None:
        disk_transport.write_bytes(
            canonical({"path": str(Path(official_disk).resolve())})
        )
    else:
        disk_transport.unlink(missing_ok=True)
    transport = Path(directory) / "input-server.json"
    if server_url:
        input_port(server_url)
        transport.write_bytes(canonical({"server_url": server_url}))
    else:
        transport.unlink(missing_ok=True)
    data = cloud_config(
        recipe,
        root,
        nonce,
        mode,
        server_url,
        keys,
        allowed_packages,
        export_pin,
        official_disk is not None,
    )
    files = {
        "user-data": b"#cloud-config\n" + canonical(data) + b"\n",
        "meta-data": canonical(
            {"instance-id": "kayfabe-apps-" + nonce, "local-hostname": "kayfabe-apps"}
        )
        + b"\n",
        "network-config": canonical(
            recipe["guest"].get(
                "network",
                {
                    "version": 2,
                    "ethernets": {"apps": {"match": {"name": "en*"}, "dhcp4": True}},
                },
            )
        )
        + b"\n",
    }
    out = Path(directory) / "seed.iso"
    out.write_bytes(make_iso(files))
    return out


@contextmanager
def serve_inputs(paths, apt=None):
    failures = []

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def parse_request(self):
            result = super().parse_request()
            if result:
                # A guestfwd chardev is one stream for QEMU's lifetime, even
                # across sequential guest TCP connections. Every GET has an
                # explicit Content-Length, so retain the backend stream when
                # urllib asks to close the guest side of a connection.
                self.close_connection = False
            return result

        def setup(self):
            self.request.settimeout(30)
            super().setup()

        def do_GET(self):
            parsed = urllib.parse.urlsplit(self.path)
            name = urllib.parse.unquote(parsed.path).removeprefix("/")
            if parsed.query or parsed.fragment:
                self.send_error(404)
                return
            try:
                path = paths.get(name)
                if path is None and apt is not None and name.startswith("ubuntu/"):
                    path = apt.get(name.removeprefix("ubuntu/"))
                if path is None:
                    self.send_error(404)
                    return
                self.send_response(200)
                self.send_header("Content-Length", str(path.stat().st_size))
                self.end_headers()
                with path.open("rb") as source:
                    shutil.copyfileobj(source, self.wfile)
            except (BrokenPipeError, ConnectionResetError, TimeoutError):
                pass
            except Exception as error:
                failures.append(error)
                self.send_error(503)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://10.0.2.100:{server.server_port}"
        if failures:
            raise official.MediaDrift(
                "MEDIA_DRIFT: pinned guest input could not be served"
            ) from failures[0]
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def qemu_option(path):
    return str(Path(path).resolve()).replace(",", ",,")


def run_qemu_tool(argv, timeout=1800):
    subprocess.run(
        argv,
        check=True,
        timeout=timeout,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def qemu_argv(recipe, disk, seed_path, channels, executable):
    netdev = "user,id=n0,restrict=on"
    forwarded = []
    transport = Path(seed_path).with_name("input-server.json")
    if transport.exists():
        port = input_port(json.loads(transport.read_text())["server_url"])
        netdev += ",guestfwd=tcp:10.0.2.100:" + str(port) + "-chardev:pin-http"
        forwarded = [
            "-chardev",
            "socket,id=pin-http,host=127.0.0.1,port=" + str(port) + ",reconnect-ms=500",
        ]
    argv = [
        executable,
        *forwarded,
        "-machine",
        "accel=kvm",
        "-cpu",
        "host",
        "-m",
        str(recipe["guest"]["memory_mib"]),
        "-smp",
        str(recipe["guest"]["cpus"]),
        "-display",
        "none",
        "-monitor",
        "none",
        "-no-reboot",
        "-chardev",
        "socket,id=console,path=" + qemu_option(channels.path("serial")),
        "-serial",
        "chardev:console",
        "-chardev",
        "socket,id=inventory,path=" + qemu_option(channels.path("inventory")),
        "-device",
        "virtio-serial-pci",
        "-device",
        "virtserialport,chardev=inventory,name=org.kayfabe.image",
        "-drive",
        "if=virtio,file="
        + qemu_option(disk)
        + ",format=qcow2,discard=unmap,detect-zeroes=unmap",
        "-drive",
        "file=" + qemu_option(seed_path) + ",media=cdrom,readonly=on,format=raw",
        "-netdev",
        netdev,
        "-device",
        "virtio-net-pci,netdev=n0",
    ]
    if "export" in channels.limits:
        argv += [
            "-chardev",
            "socket,id=export,path=" + qemu_option(channels.path("export")),
            "-device",
            "virtserialport,chardev=export,name=org.kayfabe.export",
        ]
    official_transport = Path(seed_path).with_name("official-disk.json")
    if official_transport.exists():
        official_path = Path(json.loads(official_transport.read_text())["path"])
        argv += [
            "-drive",
            "if=none,id=official,file="
            + qemu_option(official_path)
            + ",format=raw,readonly=on",
            "-device",
            "virtio-blk-pci,drive=official,serial=KF-OFFICIAL",
        ]
    return argv


def parse_result(result_path, serial_path, nonce, mode, recipe):
    if not result_path.exists() or result_path.stat().st_size > MAX_RESULT:
        raise ValueError("missing/oversized guest inventory")
    if not serial_path.exists() or serial_path.stat().st_size > MAX_LOG:
        raise ValueError("missing/oversized serial log")
    report = json.loads(result_path.read_text())
    if (
        report.get("status") != "complete"
        or report.get("nonce") != nonce
        or report.get("mode") != mode
        or report.get("recipe_sha256") != hashlib.sha256(canonical(recipe)).hexdigest()
    ):
        raise ValueError("guest session/recipe did not complete")
    inv = report.get("inventory")
    if mode == "builder":
        if (
            not isinstance(inv, dict)
            or inv.get("layer") != "builder"
            or not inv.get("files")
        ):
            raise ValueError("missing builder inventory")
        for path, sha in copied_input_hashes(recipe, "open").items():
            if path.startswith("/opt/apps/bundle/") and inv["files"].get(path) != sha:
                raise ValueError("builder copied source differs: " + path)
        for b in recipe["builds"]:
            if b["requires"] == "open-layer":
                for output in b["outputs"]:
                    if output["path"] not in inv["files"]:
                        raise ValueError("missing builder output")
                for path in b.get("shared_runtime_outputs", {}):
                    if path not in inv.get("shared_runtime", {}):
                        raise ValueError("missing shared-runtime audit")
    else:
        validate_inventory(inv, recipe)
    marker = (
        f"KF_APPS_IMAGE_COMPLETE {nonce} {hashlib.sha256(canonical(inv)).hexdigest()}"
    )
    markers = [
        line.strip()
        for line in serial_path.read_text(errors="replace").splitlines()
        if line.strip().startswith("KF_APPS_IMAGE_COMPLETE ")
    ]
    if markers != [marker]:
        raise ValueError("missing/duplicate/mismatched completion marker")
    return inv


def boot(
    recipe,
    disk,
    seed_path,
    directory,
    nonce,
    mode,
    qemu="qemu-system-x86_64",
    timeout=14400,
):
    directory = Path(directory)
    proc = None
    limits = {"serial": MAX_LOG, "inventory": MAX_RESULT}
    if mode == "builder":
        limits["export"] = MAX_EXPORT
    try:
        with Channels(directory, limits) as channels:
            proc = subprocess.Popen(
                qemu_argv(recipe, disk, seed_path, channels, qemu),
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            channels.add_pipe(proc.stdout, "qemu", MAX_LOG)
            deadline = time.monotonic() + timeout
            while proc.poll() is None:
                if time.monotonic() >= deadline:
                    raise TimeoutError("guest completion/shutdown timeout")
                channels.pump(0.1)
            channels.drain()
            if proc.returncode != 0:
                raise ValueError("QEMU failed with status " + str(proc.returncode))
        return parse_result(
            directory / "inventory.json", directory / "serial.log", nonce, mode, recipe
        )
    finally:
        if proc is not None and proc.poll() is None:
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                proc.wait(timeout=5)


def compare_inventory(expected, observed):
    if canonical(expected["pinned"]) != canonical(observed["pinned"]):
        fields = sorted(
            key
            for key in set(expected["pinned"]) | set(observed["pinned"])
            if expected["pinned"].get(key) != observed["pinned"].get(key)
        )
        raise ValueError("inventory differs (PINNED): " + ", ".join(fields))
    present = set(observed["built"]["files"]) | set(observed["pinned"]["files"])
    if set(expected["built"]["required_apps"]) - present:
        raise ValueError("missing inventory app")


def preflight(path, build=True, required=None):
    if not sys.platform.startswith("linux") or not os.access(
        "/dev/kvm", os.R_OK | os.W_OK
    ):
        raise ValueError("Linux and KVM access required")
    minimum = required if required is not None else (MIN_FREE if build else 6 * 1024**3)
    if shutil.disk_usage(path).free < minimum:
        raise ValueError("insufficient free disk")


def fetch_inputs(recipe, cache):
    return {
        a["sha256"]: download(a, cache) for a in [recipe["base"]] + recipe["artifacts"]
    }


def official_pins(recipe, builder=False):
    pins = [
        a
        for a in recipe["official_fetch"]
        if not builder or a["kind"] in ("run", "archive")
    ]
    pins = [
        a
        for a in pins
        if a["kind"] != "run" or a["version"] == recipe["guest"]["nvidia_version"]
    ]
    return pins


def fetch_official(recipe, directory, builder=False):
    return {
        a["sha256"]: official.fetch(a, directory)
        for a in official_pins(recipe, builder)
    }


def official_input_disk(recipe, paths, path, builder=False):
    pins = official_pins(recipe, builder)
    if not pins:
        return None
    try:
        return input_disk.write(path, pins, paths)
    except ValueError as error:
        raise official.MediaDrift(
            "MEDIA_DRIFT: official input disk assembly failure"
        ) from error


def keep_failure(work, out):
    target = Path(tempfile.mkdtemp(prefix="apps-image-failure-", dir=out))
    for name, limit in [
        ("serial.log", MAX_LOG),
        ("inventory.json", MAX_RESULT),
        ("qemu.log", MAX_LOG),
    ]:
        path = work / name
        if path.exists():
            with path.open("rb") as src, (target / name).open("wb") as dst:
                (
                    shutil.copyfileobj(src, dst, length=65536)
                    if path.stat().st_size <= limit
                    else dst.write(src.read(limit))
                )


def boot_recorded(recipe, disk, seed_path, work, nonce, mode, out, qemu, timeout):
    try:
        return boot(recipe, disk, seed_path, work, nonce, mode, qemu, timeout)
    except BaseException:
        keep_failure(work, out)
        raise


def create_overlay(qemu_img, base, out, size=None):
    argv = [qemu_img, "create", "-f", "qcow2", "-F", "qcow2", "-b", str(base), str(out)]
    if size:
        argv.append(str(size) + "G")
    run_qemu_tool(argv)


def build(
    recipe,
    out,
    cache,
    keys=(),
    qemu="qemu-system-x86_64",
    qemu_img="qemu-img",
    timeout=14400,
    root=HERE,
):
    validate_recipe(recipe)
    out = Path(out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    image = out / "apps-image.qcow2"
    manifest_path = out / "apps-image.manifest.json"
    bundle_path = out / "apps-open-bundle.tar.gz"
    if image.exists() or manifest_path.exists() or bundle_path.exists():
        raise ValueError("output already exists")
    preflight(out)
    cache = Path(cache).resolve()
    cache.mkdir(parents=True, exist_ok=True)
    preflight(cache)
    list(local_files(recipe, root))
    paths = fetch_inputs(recipe, cache)
    apt = AptMirror(recipe, cache / "apt")
    base = paths[recipe["base"]["sha256"]]
    with tempfile.TemporaryDirectory(prefix=".apps-build-", dir=out) as temporary:
        work = Path(temporary)
        builder = work / "builder"
        builder.mkdir()
        builder_disk = builder / "scratch.qcow2"
        create_overlay(qemu_img, base, builder_disk, recipe["guest"]["disk_gib"])
        # These files and the scratch guest are never promoted into the open image.
        transient = fetch_official(recipe, work / "official", builder=True)
        official_data = official_input_disk(
            recipe, transient, builder / "official-inputs.raw", builder=True
        )
        with serve_inputs(paths, apt) as server:
            nonce = secrets.token_hex(16)
            seed_path = seed(
                recipe,
                builder,
                nonce,
                "builder",
                server,
                root=root,
                allowed_packages=apt.names,
                official_disk=official_data,
            )
            build_inv = boot_recorded(
                recipe,
                builder_disk,
                seed_path,
                builder,
                nonce,
                "builder",
                out,
                qemu,
                timeout,
            )
        export = builder / "export.tar.gz"
        validate_export(export, recipe, build_inv)
        export_pin = {
            "id": "open-build-export",
            "url": "https://local.invalid/open-build-export",
            "sha256": sha256_file(export),
            "size": export.stat().st_size,
        }
        # Delete the entire NVIDIA scratch disk, not just files from its filesystem.
        builder_disk.unlink()
        if official_data is not None:
            official_data.unlink()
        shutil.rmtree(work / "official")
        final = work / "open"
        final.mkdir()
        disk = final / "open.qcow2"
        create_overlay(qemu_img, base, disk, recipe["guest"]["disk_gib"])
        with serve_inputs({**paths, export_pin["sha256"]: export}, apt) as server:
            nonce = secrets.token_hex(16)
            seed_path = seed(
                recipe,
                final,
                nonce,
                "assemble",
                server,
                keys,
                root,
                apt.names,
                export_pin,
            )
            inv = boot_recorded(
                recipe, disk, seed_path, final, nonce, "assemble", out, qemu, timeout
            )
        if inv["layer"] != "open" or not inv["nvidia_scan"]["clean"]:
            raise ValueError("open image NVIDIA-file scan failed")
        for path, sha in build_inv["files"].items():
            if inv["files"].get(path) != sha:
                raise ValueError("assembled app differs from audited builder export")
        converted = work / "converted.qcow2"
        run_qemu_tool(
            [
                qemu_img,
                "convert",
                "-f",
                "qcow2",
                "-O",
                "qcow2",
                "-c",
                str(disk),
                str(converted),
            ]
        )
        run_qemu_tool([qemu_img, "check", "-f", "qcow2", str(converted)])
        manifest = {
            "schema_version": 3,
            "layer": "open",
            "no_nvidia_proprietary_files": True,
            "claim": "PINNED inputs identical; BUILT hashes informational",
            "recipe": recipe,
            "recipe_sha256": hashlib.sha256(canonical(recipe)).hexdigest(),
            "base": recipe["base"],
            "apt_snapshot": recipe["apt"]["snapshot"],
            "main_universe_packages": sorted(apt.names),
            "apt_inputs": list(apt.used.values()),
            **inventory_parts(inv, recipe, tooling(root)),
        }
        manifest["built"].update(
            {
                "builder_inventory": build_inv,
                "open_bundle": {
                    "filename": bundle_path.name,
                    "sha256": export_pin["sha256"],
                    "size": export_pin["size"],
                },
                "image": {
                    "filename": image.name,
                    "sha256": sha256_file(converted),
                    "size": converted.stat().st_size,
                },
            }
        )
        pending = work / "manifest.json"
        pending.write_bytes(canonical(manifest) + b"\n")
        shutil.copyfile(export, bundle_path)
        converted.replace(image)
        pending.replace(manifest_path)
    return manifest


def load_manifest(path, root):
    manifest = json.loads(Path(path).read_text())
    recipe = validate_recipe(manifest["recipe"])
    if manifest["schema_version"] != 3 or manifest["pinned"].get("tooling") != tooling(
        root
    ):
        raise ValueError("manifest schema/tooling differs; use original checkout")
    if manifest["recipe_sha256"] != hashlib.sha256(canonical(recipe)).hexdigest():
        raise ValueError("manifest recipe digest differs")
    inv = inventory_observation(manifest)
    validate_inventory(inv, recipe)
    if manifest["pinned"] != inventory_parts(inv, recipe, tooling(root))["pinned"]:
        raise ValueError("manifest pinned inventory differs from recipe/observation")
    if (
        manifest["built"]["required_apps"]
        != inventory_parts(inv, recipe, tooling(root))["built"]["required_apps"]
    ):
        raise ValueError("manifest required apps differ")
    if manifest["layer"] != inv["layer"]:
        raise ValueError("manifest/inventory layer differs")
    return manifest, recipe


def verify(
    image,
    manifest_path,
    out,
    qemu="qemu-system-x86_64",
    qemu_img="qemu-img",
    timeout=900,
    root=HERE,
):
    image = Path(image).resolve()
    manifest, recipe = load_manifest(manifest_path, root)
    layer = manifest["layer"]
    out = Path(out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    target = out / "apps-image.verify.json"
    if target.exists():
        raise ValueError("verification output exists")
    preflight(out, build=False)
    before = sha256_file(image)
    run_qemu_tool([qemu_img, "check", "-f", "qcow2", str(image)])
    with tempfile.TemporaryDirectory(prefix=".apps-verify-", dir=out) as temporary:
        work = Path(temporary)
        disk = work / "verify.qcow2"
        create_overlay(qemu_img, image, disk)
        nonce = secrets.token_hex(16)
        mode = "verify-" + layer
        seed_path = seed(
            recipe,
            work,
            nonce,
            mode,
            root=root,
            allowed_packages=manifest.get("main_universe_packages", ()),
        )
        inv = boot_recorded(
            recipe, disk, seed_path, work, nonce, mode, out, qemu, timeout
        )
        observed = inventory_parts(inv, recipe, tooling(root))
        compare_inventory(manifest, observed)
        if sha256_file(image) != before:
            raise ValueError("verification changed input image")
        report = {
            "status": "pinned-inputs-identical",
            "layer": layer,
            "pinned": observed["pinned"],
            "pinned_inventory_sha256": hashlib.sha256(
                canonical(observed["pinned"])
            ).hexdigest(),
            "built": {
                **observed["built"],
                "image": {"sha256": before, "size": image.stat().st_size},
                "reference_image_sha256": manifest["built"]["image"]["sha256"],
            },
        }
        pending = work / "report.json"
        pending.write_bytes(canonical(report) + b"\n")
        pending.replace(target)
    return report


@contextmanager
def provisioned(
    image,
    manifest_path,
    out,
    cache,
    qemu="qemu-system-x86_64",
    qemu_img="qemu-img",
    timeout=14400,
    root=HERE,
    driver_version=None,
):
    """Yield a per-run overlay and runtime manifest. Always delete NVIDIA files on exit.

    The coordinator runs the existing gate while this context is alive. Never
    upload or distribute the overlay, official downloads, or a disk made from it.
    """
    manifest, recipe = load_manifest(manifest_path, root)
    if manifest["layer"] != "open":
        raise ValueError("provisioning requires an open-layer manifest")
    if driver_version is not None:
        if driver_version not in recipe.get("driver_versions", {}):
            raise official.MediaDrift("MEDIA_DRIFT: release driver has no official pin")
        recipe = json.loads(canonical(recipe))
        recipe["guest"]["nvidia_version"] = driver_version
    out = Path(out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    preflight(
        out,
        build=False,
        required=2 * sum(a["size"] for a in official_pins(recipe)) + 12 * 1024**3,
    )
    with tempfile.TemporaryDirectory(prefix=".apps-runtime-", dir=out) as temporary:
        work = Path(temporary)
        verify(
            image,
            manifest_path,
            work / "verified-open",
            qemu,
            qemu_img,
            min(timeout, 900),
            root,
        )
        disk = work / "runtime.qcow2"
        create_overlay(qemu_img, Path(image).resolve(), disk)
        paths = fetch_inputs(recipe, cache)
        official_paths = fetch_official(recipe, work / "official")
        official_data = official_input_disk(
            recipe, official_paths, work / "official-inputs.raw"
        )
        with serve_inputs(paths) as server:
            nonce = secrets.token_hex(16)
            seed_path = seed(
                recipe,
                work,
                nonce,
                "provision",
                server,
                root=root,
                official_disk=official_data,
            )
            inv = boot_recorded(
                recipe, disk, seed_path, work, nonce, "provision", out, qemu, timeout
            )
        runtime = {
            **manifest,
            "recipe": recipe,
            "recipe_sha256": hashlib.sha256(canonical(recipe)).hexdigest(),
            "layer": "runtime",
            "no_nvidia_proprietary_files": False,
            **inventory_parts(inv, recipe, tooling(root)),
        }
        runtime["built"].update(
            {
                "open_image_sha256": manifest["built"]["image"]["sha256"],
                "image": {
                    "filename": disk.name,
                    "sha256": sha256_file(disk),
                    "size": disk.stat().st_size,
                },
            }
        )
        receipt = work / (disk.name + ".STAGED")
        receipt.write_text(
            "version="
            + inv["guest"]["nvidia_module_version"]
            + "\nimage="
            + str(disk)
            + "\nstaged=inventory-verified\n"
        )
        runtime_path = work / "runtime.manifest.json"
        runtime_path.write_bytes(canonical(runtime) + b"\n")
        yield disk, runtime_path, runtime


def public_key(path):
    value = Path(path).read_text().strip()
    if "\n" in value or not value.startswith(
        ("ssh-ed25519 ", "ssh-rsa ", "ecdsa-sha2-")
    ):
        raise ValueError("supply one SSH public key")
    try:
        base64.b64decode(value.split()[1], validate=True)
    except (ValueError, IndexError) as error:
        raise ValueError("invalid SSH public key") from error
    return value


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "action",
        choices=[
            "plan",
            "inputs",
            "build",
            "verify",
            "verify-official",
            "provision-check",
        ],
    )
    p.add_argument("--recipe", type=Path, default=HERE / "recipe.json")
    p.add_argument("--cache", type=Path, default=Path("apps-image-cache"))
    p.add_argument("--out", type=Path, default=Path("out"))
    p.add_argument("--image", type=Path)
    p.add_argument("--manifest", type=Path)
    p.add_argument("--ssh-authorized-key", type=Path, action="append", default=[])
    p.add_argument("--qemu", default="qemu-system-x86_64")
    p.add_argument("--qemu-img", default="qemu-img")
    p.add_argument("--timeout", type=int)
    p.add_argument("--driver-version")
    args = p.parse_args(argv)
    if args.timeout is not None and args.timeout <= 0:
        p.error("timeout must be positive")
    try:
        if args.action in ("verify", "provision-check"):
            if args.image is None or args.manifest is None:
                p.error("image and manifest required")
            if args.action == "verify":
                result = verify(
                    args.image,
                    args.manifest,
                    args.out,
                    args.qemu,
                    args.qemu_img,
                    args.timeout or 900,
                )
            else:
                with provisioned(
                    args.image,
                    args.manifest,
                    args.out,
                    args.cache,
                    args.qemu,
                    args.qemu_img,
                    args.timeout or 14400,
                    driver_version=args.driver_version,
                ) as (disk, path, runtime):
                    result = verify(
                        disk,
                        path,
                        args.out / "verified-runtime",
                        args.qemu,
                        args.qemu_img,
                        args.timeout or 900,
                    )
                    # Inventory data can be retained; never retain the NVIDIA overlay or installers.
                    (args.out / "apps-runtime.inventory.json").write_bytes(
                        canonical(
                            {"pinned": runtime["pinned"], "built": runtime["built"]}
                        )
                        + b"\n"
                    )
        else:
            recipe = read_recipe(args.recipe)
            list(local_files(recipe, HERE))
            for layer, file in [
                ("open", "requirements.lock"),
                ("runtime", "requirements-runtime.lock"),
            ]:
                if (HERE / file).read_text() != requirements(recipe, layer):
                    raise ValueError("requirements lock drift")
            if args.action == "verify-official":
                result = official.verify_urls(recipe["official_fetch"])
            elif args.action == "plan":
                result = {
                    "recipe_version": recipe["recipe_version"],
                    "open_inputs": len(recipe["artifacts"]) + 1,
                    "official_fetch": len(recipe["official_fetch"]),
                    "open_download_bytes": sum(
                        a["size"] for a in [recipe["base"]] + recipe["artifacts"]
                    ),
                    "static_runtime_builds": [
                        b["name"]
                        for b in recipe["builds"]
                        if b["requires"] == "nvidia-layer-static"
                    ],
                    "claim": "PINNED input equality; BUILT binary/image hashes informational",
                }
            elif args.action == "inputs":
                fetch_inputs(recipe, args.cache)
                result = {"status": "open inputs verified; no NVIDIA inputs fetched"}
            else:
                manifest = build(
                    recipe,
                    args.out,
                    args.cache,
                    [public_key(x) for x in args.ssh_authorized_key],
                    args.qemu,
                    args.qemu_img,
                    args.timeout or 14400,
                )
                result = {
                    "pinned": manifest["pinned"],
                    "built": manifest["built"],
                    "manifest": str(args.out / "apps-image.manifest.json"),
                    "layer": "open",
                }
        print(json.dumps(result, sort_keys=True))
        return 0
    except (ValueError, OSError, TimeoutError, subprocess.SubprocessError) as error:
        if getattr(error, "report", None):
            print(json.dumps(error.report, sort_keys=True))
        print("apps-image: " + str(error), file=sys.stderr)
        return 1


def interrupted(signum, frame):
    raise InterruptedError("image operation interrupted by signal " + str(signum))


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        sys.exit(130)
