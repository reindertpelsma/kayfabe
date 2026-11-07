#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Shell-free open Windows bundle. Run with python3 -I package_bundle.py --help.

Only urllib downloads and the pinned Zig executable are used on the packaging
host. App archives/installers are NEVER extracted here. No upload capability.
The compiler release itself is unpacked by Python (bundle_tools.py).
"""
import argparse
import datetime
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))  # sibling modules also work under python -I
import bundle_iso

OPEN_PACKAGES = frozenset("cuda_demo_suite python_embed pywheels llama_cuda llama_vulkan qwen_gguf vulkan_rt vkpeak blender clpeak_ocl clpeak_cuda hashcat sevenzip cuda_cudart cuda_cccl vc_redist hf_wheels hf_model cuda_nvrtc".split())
STAGE_MODES = frozenset(("unzip", "python_embed", "wheels", "copy", "7z", "msi_admin", "installer"))
UA = "kayfabe-bundle/1 (+https://github.com/reindertpelsma/kayfabe)"


def guard(env=None):
    env = os.environ if env is None else env
    for key in ("CI", "KF_RUN_TOKEN"):
        if key in env:
            raise ValueError(f"REFUSED: {key} is set; packaging is a person's local operation")


def load(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def canonical(obj):
    return (json.dumps(obj, sort_keys=True, ensure_ascii=True, separators=(",", ":")) + "\n").encode("ascii")


def sha256(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
    return h.hexdigest()


def validate_pin(package, fl):
    try:
        name = fl["name"]
        if len(bundle_iso.valid_path(name)) != 1:
            raise ValueError("filename must be one component")
        if not re.fullmatch(r"[0-9a-f]{64}", fl.get("sha256") or ""):
            raise ValueError("missing/invalid sha256 pin")
        if type(fl.get("size")) is not int or not 0 < fl["size"] <= bundle_iso.MAX_FILE:
            raise ValueError("missing/invalid size pin")
        url = urllib.parse.urlsplit(fl.get("url", ""))
        if url.scheme != "https" or not url.hostname or url.username or url.password or url.fragment:
            raise ValueError("input URL must be pinned HTTPS without credentials/fragment")
    except (KeyError, TypeError, ValueError) as e:
        raise ValueError(f"package {package}: {e}") from e


def select_packages(man, profile, only=None):
    allowed = profile.get("open_packages", [])
    if not allowed or len(set(allowed)) != len(allowed):
        raise ValueError("open_packages must be nonempty and unique")
    if set(allowed) - OPEN_PACKAGES:
        raise ValueError("forbidden open_packages: " + ",".join(sorted(set(allowed) - OPEN_PACKAGES)))
    want = list(allowed) if only is None else list(only)
    if not want:
        raise ValueError("empty package selection")
    byid = {}
    for p in man.get("packages", []):
        if p["id"] in byid:
            raise ValueError("duplicate package: " + p["id"])
        byid[p["id"]] = p
    selected, active, seen = [], set(), set()
    def visit(pid):
        if pid not in allowed or pid not in byid:
            raise ValueError(f"package {pid}: outside open_packages or absent from manifest")
        if pid in active:
            raise ValueError(f"package {pid}: dependency cycle")
        if pid in seen:
            return
        active.add(pid)
        p = byid[pid]
        if not p.get("license") or not p.get("redistribution"):
            raise ValueError(f"package {pid}: missing licence/redistribution record")
        if p.get("stage", {}).get("mode") not in STAGE_MODES:
            raise ValueError(f"package {pid}: unsupported guest staging mode")
        if not p.get("files"):
            raise ValueError(f"package {pid}: no files")
        names = set()
        for fl in p["files"]:
            validate_pin(pid, fl)
            if fl["name"].casefold() in names:
                raise ValueError(f"package {pid}: duplicate/case-colliding file")
            names.add(fl["name"].casefold())
        for dep in sorted(p.get("needs", [])):
            visit(dep)
        active.remove(pid); seen.add(pid); selected.append(p)
    if man.get("schema") != 1:
        raise ValueError("unsupported manifest schema")
    for pid in sorted(want):
        visit(pid)
    return selected


def verify_file(package, fl, path):
    path = Path(path)
    if path.is_symlink() or not path.is_file() or path.stat().st_size != fl["size"] or sha256(path) != fl["sha256"]:
        raise ValueError(f"package {package}/{fl['name']}: missing file or size/sha256 mismatch")


class _HTTPSRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if urllib.parse.urlsplit(newurl).scheme != "https":
            raise ValueError("non-HTTPS redirect refused")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def fetch_file(package, fl, destination, retries=4):
    """Resume a .part using validated Range responses; promote only verified bytes.

    Existing bad cache entries fail closed. Transport/truncation retries use the
    same URL; hash mismatch is terminal. Never rewrite a pin or choose a mirror.
    """
    validate_pin(package, fl)
    dest = Path(destination)
    if dest.exists() or dest.is_symlink():
        verify_file(package, fl, dest)
        print(f"CACHE_OK {package}/{fl['name']}", flush=True)
        return
    print(f"FETCH {package}/{fl['name']} bytes={fl['size']}", flush=True)
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + ".part")
    if part.is_symlink():
        raise ValueError(f"package {package}/{fl['name']}: partial file is a link")
    error = "download failed"
    for attempt in range(retries):
        offset = part.stat().st_size if part.exists() else 0
        if offset > fl["size"]:
            raise ValueError(f"package {package}/{fl['name']}: partial file exceeds pin size")
        if offset == fl["size"]:
            break
        request = urllib.request.Request(fl["url"], headers={"User-Agent": UA, "Accept-Encoding": "identity"})
        if offset:
            request.add_header("Range", f"bytes={offset}-")
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                status = response.status
                if status == 206:
                    expected = f"bytes {offset}-{fl['size'] - 1}/{fl['size']}"
                    if response.headers.get("Content-Range") != expected:
                        raise ValueError("invalid resumed Content-Range")
                elif status == 200:
                    offset = 0  # server ignored Range; discard the old prefix.
                else:
                    raise ValueError(f"unexpected HTTP status {status}")
                with part.open("ab" if status == 206 and offset else "wb") as f:
                    while chunk := response.read(min(1 << 20, fl["size"] - offset + 1)):
                        offset += len(chunk)
                        if offset > fl["size"]:
                            raise ValueError("response exceeds pin size")
                        f.write(chunk)
            if offset == fl["size"]:
                break
            error = "truncated response"
        except (OSError, urllib.error.URLError, http.client.HTTPException) as e:
            # Do not print redirect/signed URLs or provider response bodies.
            error = type(e).__name__
        except ValueError as e:
            raise ValueError(f"package {package}/{fl['name']}: {e}") from e
        if attempt + 1 < retries:
            time.sleep(min(2 ** attempt, 8))
    if not part.exists() or part.stat().st_size != fl["size"]:
        raise ValueError(f"package {package}/{fl['name']}: {error} after {retries} attempts")
    if sha256(part) != fl["sha256"]:
        part.unlink()
        raise ValueError(f"package {package}/{fl['name']}: sha256 mismatch (partial deleted)")
    os.replace(part, dest)


def package_files(packages, downloads):
    return {f"pkg/{p['id']}/{f['name']}": Path(downloads) / p["id"] / f["name"]
            for p in packages for f in p["files"]}


def stage_table(packages):
    """Ignore legacy image_form=tree: every app archive is staged inside Windows."""
    return {p["id"]: dict(mode=p["stage"]["mode"], dest=p["stage"]["dest"].replace("/", "\\"),
             strip_top=bool(p["stage"].get("strip_top")), run=p["stage"].get("run", []),
             files=[f["name"] for f in p["files"]]) for p in packages}


def repo_files(root=HERE):
    root = Path(root)
    # Normalize checkout CRLF to LF. Only tracked textual input classes are used;
    # no binaries, pycache, local temporary files or arbitrary tool directory.
    files = {}
    for p in sorted((root / "guest").rglob("*")):
        if p.suffix not in (".ps1", ".py", ".html"):
            continue
        if p.is_symlink():
            raise ValueError("repository guest file is a link: " + p.name)
        files[p.relative_to(root).as_posix()] = p.read_bytes().replace(b"\r\n", b"\n")
    files["apps.json"] = canonical(load(root / "apps.json"))
    for name in ("LICENSE", "LICENSE-APACHE", "LICENSE-GPL", "THIRD_PARTY_NOTICES.md"):
        files["licenses/kayfabe/" + name] = (root.parents[3] / name).read_bytes().replace(b"\r\n", b"\n")
    return files


def identity(records):
    # Identity hashes the payload; KFAPPS.ID is excluded to avoid a self-reference.
    records = {k: v for k, v in records.items() if k != "KFAPPS.ID"}
    return hashlib.sha256(json.dumps(records, sort_keys=True).encode("utf-8")).hexdigest()


def build_image(packages, downloads, tools, output, toolchain, root=HERE, probe_receipt=None):
    """Assemble already pinned inputs. Compiler/upload/network are outside this function."""
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    files = package_files(packages, downloads)
    expected = {}
    for p in packages:
        for fl in p["files"]:
            path = f"pkg/{p['id']}/{fl['name']}"
            verify_file(p["id"], fl, files[path])
            expected[path] = {k: fl[k] for k in ("sha256", "size")}
    files.update(repo_files(root))
    files.update({"tools/" + name: Path(path) for name, path in sorted(tools.items())})
    files["stage.json"] = canonical(stage_table(packages))
    # No wall clock, host paths, Python version or host-specific toolchain selection in ISO.
    files["bundle.inputs.json"] = canonical(dict(schema=1, packages=packages, toolchain=toolchain))
    for name, source in files.items():
        if name not in expected:
            expected[name] = dict(sha256=hashlib.sha256(source).hexdigest(), size=len(source)) if isinstance(source, bytes) else dict(sha256=sha256(source), size=source.stat().st_size)
    ident = identity(expected)
    files["KFAPPS.ID"] = (ident + "\n").encode("ascii")
    expected["KFAPPS.ID"] = dict(sha256=hashlib.sha256(files["KFAPPS.ID"]).hexdigest(), size=len(files["KFAPPS.ID"]))
    iso = output / "winapps-open.iso"
    temp = output / "winapps-open.pending.iso"
    try:
        actual = bundle_iso.write_image(files, temp)
        # Compare hashes of bytes actually copied, catching source races/corruption.
        if any(actual[k] != v for k, v in expected.items()):
            raise ValueError("input changed during image assembly")
        receipt = dict(schema=1, packages=packages, files=expected,
            image=dict(file=iso.name, label="KFAPPS", format="iso9660+joliet", identity=ident,
                       size=temp.stat().st_size, sha256=sha256(temp)),
            toolchain=toolchain, packaging_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
            python_version=sys.version, writer="bundle_iso.py/v1", guest_root="C:\\kfapps")
        receipt["recipe_sha256"] = {name: hashlib.sha256((HERE / name).read_bytes().replace(b"\r\n", b"\n")).hexdigest()
                                     for name in ("package_bundle.py", "bundle_iso.py", "bundle_tools.py")}
        if probe_receipt is not None:
            receipt["probe_build"] = probe_receipt
        verify_image(temp, receipt)
        os.replace(temp, iso)
    finally:
        temp.unlink(missing_ok=True)
    (output / "winapps-open.manifest.json").write_bytes(canonical(receipt))
    (output / "bundle.pin.line").write_bytes(f"winapps-open.iso sha256={receipt['image']['sha256']}\n".encode("ascii"))
    return receipt


def verify_image(iso, receipt):
    if Path(iso).stat().st_size != receipt["image"]["size"] or sha256(iso) != receipt["image"]["sha256"]:
        raise ValueError("image size/sha256 mismatch")
    expected = receipt["files"]
    if not expected or receipt["image"]["identity"] != identity(expected):
        raise ValueError("image identity mismatch")
    with bundle_iso.Reader(iso) as r:
        if "KFAPPS.ID" not in expected or set(r.files) != set(expected):
            raise ValueError("image file list mismatch")
        got = r.records()
        for name, pin in expected.items():
            if got[name] != pin:
                raise ValueError("image file size/sha256 mismatch: " + name)
        if b"".join(r.chunks("KFAPPS.ID")) != (identity(expected) + "\n").encode("ascii"):
            raise ValueError("KFAPPS.ID mismatch")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("command", choices=("check", "fetch", "build", "verify", "verify-image", "layout"))
    ap.add_argument("--manifest", type=Path, default=HERE / "manifest.json")
    ap.add_argument("--profiles", type=Path, default=HERE / "gate_profiles.json")
    ap.add_argument("--dl", type=Path, default=Path("out/downloads"))
    ap.add_argument("--out", type=Path, default=Path("out"))
    ap.add_argument("--only", help="comma-separated open package IDs (dependencies included)")
    ap.add_argument("--iso", type=Path, help="image for verify-image")
    ap.add_argument("--receipt", type=Path, help="build receipt for verify-image")
    args = ap.parse_args(argv)
    try:
        guard()
        # A single HTTPS-only opener prevents redirect downgrade, while keeping
        # the urllib call easy to fake in offline tests.
        urllib.request.install_opener(urllib.request.build_opener(_HTTPSRedirect()))
        if args.command == "verify-image":
            verify_image(args.iso or args.out / "winapps-open.iso", load(args.receipt or args.out / "winapps-open.manifest.json"))
        else:
            man, profile = load(args.manifest), load(args.profiles)
            packages = select_packages(man, profile, args.only.split(",") if args.only is not None else None)
            import bundle_tools
            toolchain = bundle_tools.check_toolchain(man.get("bundle_toolchain", {}))
            if args.command == "layout":
                paths = set(package_files(packages, args.dl)) | set(repo_files()) | {"KFAPPS.ID", "stage.json", "bundle.inputs.json"}
                paths |= {"tools/" + n for n in bundle_tools.TOOL_NAMES + bundle_tools.NOTICE_NAMES}
                print("\n".join(sorted(paths)))
                host, pin = bundle_tools.host_pin(toolchain)
                print(f"DOWNLOAD_ONLY _toolchain/{pin['name']} host={host} (compiler; not on ISO)")
            elif args.command in ("fetch", "verify", "build"):
                for p in packages:
                    for fl in p["files"]:
                        dest = args.dl / p["id"] / fl["name"]
                        if args.command == "fetch":
                            fetch_file(p["id"], fl, dest)
                        else:
                            verify_file(p["id"], fl, dest)
                host, pin = bundle_tools.host_pin(toolchain)
                archive = args.dl / "_toolchain" / pin["name"]
                if args.command == "fetch":
                    fetch_file("zig-" + host, pin, archive)
                else:
                    verify_file("zig-" + host, pin, archive)
                if args.command == "build":
                    tools = bundle_tools.build_tools(toolchain, archive, args.out / "build")
                    receipt = build_image(packages, args.dl, tools, args.out, toolchain,
                                          probe_receipt=load(args.out / "build" / "tools.receipt.json"))
                    print("BUILD_DONE sha256=" + receipt["image"]["sha256"] + " identity=" + receipt["image"]["identity"])
        print(args.command.upper().replace("-", "_") + "_OK")
        return 0
    except (OSError, ValueError, KeyError, TypeError) as e:
        print("FAIL " + str(e), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
