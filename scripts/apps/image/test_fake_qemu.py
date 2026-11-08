#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Test-only executable. Uses the real bounded transport, never a VM/KVM."""

import hashlib
import io
import json
import os
from pathlib import Path
import socket
import sys
import tarfile
import time


def main():
    args = sys.argv[1:]
    scenario = json.loads(Path(os.environ["APPS_IMAGE_TEST_SCENARIO"]).read_text())
    with open(os.environ["APPS_IMAGE_TEST_COMMANDS"], "a") as f:
        f.write(json.dumps({"program": Path(sys.argv[0]).name, "argv": args}) + "\n")
    if Path(sys.argv[0]).name == "qemu-img":
        if args[0] == "create":
            Path(args[-2] if args[-1].endswith("G") else args[-1]).write_bytes(
                b"fake-qcow2"
            )
        if args[0] == "convert":
            Path(args[-1]).write_bytes(b"fake-converted-qcow2")
        return (
            int(scenario.get("qemu_img_rc", 0))
            if scenario.get("qemu_img_fail_on", args[0]) == args[0]
            else 0
        )
    if scenario.get("hang"):
        time.sleep(30)
        return 0
    drive = next(
        args[i + 1]
        for i, x in enumerate(args)
        if x == "-drive" and "media=cdrom" in args[i + 1]
    )
    seed = Path(drive.split("file=", 1)[1].split(",media=", 1)[0].replace(",,", ","))
    data = json.loads(
        seed.read_bytes().split(b"#cloud-config\n", 1)[1].split(b"\n", 1)[0]
    )
    written = {x["path"]: x["content"] for x in data["write_files"]}
    session = json.loads(written["/opt/apps/image/session.json"])
    recipe = json.loads(written["/opt/apps/image/recipe.json"])
    mode = session["mode"]
    if mode == "builder":
        inv = {"layer": "builder", "shared_runtime": {}, "files": {}}
        for b in recipe["builds"]:
            if b["requires"] != "open-layer":
                continue
            inv["files"].update(
                {o["path"]: hashlib.sha256(b"built").hexdigest() for o in b["outputs"]}
            )
            inv["shared_runtime"].update(
                {p: libs for p, libs in b.get("shared_runtime_outputs", {}).items()}
            )
        if scenario.get("extra_builder_file"):
            inv["files"][scenario["extra_builder_file"]] = hashlib.sha256(
                b"built"
            ).hexdigest()
        for path, content in scenario.get("builder_contents", {}).items():
            inv["files"][path] = hashlib.sha256(content.encode("utf-8")).hexdigest()
    else:
        inv = (
            scenario.get("runtime_inventory", scenario["inventory"])
            if mode in ("provision", "verify-runtime")
            else scenario["inventory"]
        )
    report = {k: session[k] for k in ("nonce", "mode", "recipe_sha256")}
    report.update(status="complete", inventory=inv)
    if scenario.get("stale"):
        report["nonce"] = "stale-session"
    if scenario.get("error"):
        report["status"] = "error"
    endpoints = {}
    for i, a in enumerate(args):
        if a == "-chardev":
            config = args[i + 1]
            if "path=" not in config:
                continue
            ident = config.split("id=", 1)[1].split(",", 1)[0]
            endpoints[ident] = config.split("path=", 1)[1].replace(",,", ",")

    def send(ident, content):
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.connect(endpoints[ident])
            s.sendall(content)

    if mode == "builder":
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
            for path in inv["files"]:
                content = (
                    scenario.get("builder_contents", {})
                    .get(path, "built")
                    .encode("utf-8")
                )
                info = tarfile.TarInfo(path.lstrip("/"))
                info.size = len(content)
                info.mode = 0o755
                archive.addfile(info, io.BytesIO(content))
        send("export", buffer.getvalue())
    if not scenario.get("no_inventory"):
        send("inventory", json.dumps(report).encode())
    digest = hashlib.sha256(
        json.dumps(inv, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()
    marker = f"KF_APPS_IMAGE_COMPLETE {session['nonce']} {digest}\n"
    if scenario.get("bad_digest"):
        marker = marker.replace(digest, "0" * 64)
    if scenario.get("no_marker"):
        marker = "cloud-init finished\n"
    if scenario.get("duplicate"):
        marker += marker
    send("console", b"fake guest boot transcript\n" + marker.encode())
    return int(scenario.get("qemu_rc", 0))


if __name__ == "__main__":
    sys.exit(main())
