#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Shell-free open Windows bundle. Run with python3 -I package_bundle.py --help.

Only urllib downloads and the pinned Zig executable are used on the packaging
host. App payloads are NEVER extracted here; only licence texts are read from
verified archives into LICENSES/. No upload capability.
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
import bundle_compliance
import bundle_repro

OPEN_PACKAGES = frozenset("cuda_demo_suite python_embed pywheels llama_cuda llama_vulkan qwen_gguf vulkan_rt vkpeak blender clpeak_ocl clpeak_cuda hashcat sevenzip cuda_cudart cuda_cccl vc_redist hf_wheels hf_model cuda_nvrtc hf_wheels_extra".split())
PUBLIC_PACKAGES = frozenset("python_embed llama_vulkan qwen_gguf hf_model vkpeak blender clpeak_ocl hashcat sevenzip hf_wheels".split())
STAGE_MODES = frozenset(("unzip", "python_embed", "wheels", "copy", "7z", "msi_admin", "installer"))
UA = "kayfabe-bundle/1 (+https://github.com/reindertpelsma/kayfabe)"
BUILDER_VERSION = 6


class InputDrift(ValueError):
    """Pinned upstream bytes changed; never treat this as reproduction noise."""


def guard(env=None, allow_ci=False, bundle_class="public"):
    env = os.environ if env is None else env
    if bundle_class != "public": raise ValueError("permanent bundles are public-only; official media is transient")
    if env.get("GITHUB_ACTIONS") == "true" and not allow_ci:
        raise ValueError("REFUSED: CI packaging requires explicit --ci-build")
    for key in ("CI", "KF_RUN_TOKEN"):
        if key in env and not (key == "CI" and allow_ci):
            raise ValueError(f"REFUSED: {key} is set; packaging is a person's local operation")
    for key in ("AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN", "R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY", "BUNDLE_UPLOAD_URLS"):
        if key in env:
            raise ValueError(f"REFUSED: build environment contains upload credential/capability {key}")


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


def validate_pin(package, fl, image=False):
    try:
        name = fl["name"]
        if len(bundle_iso.valid_path(name)) != 1:
            raise ValueError("filename must be one component")
        if not re.fullmatch(r"[0-9a-f]{64}", fl.get("sha256") or ""):
            raise ValueError("missing/invalid sha256 pin")
        if type(fl.get("size")) is not int or not 0 < fl["size"] <= (1 << 50 if image else bundle_iso.MAX_FILE):
            raise ValueError("missing/invalid size pin")
        url = urllib.parse.urlsplit(fl.get("url", ""))
        if url.scheme != "https" or not url.hostname or url.username or url.password or url.fragment:
            raise ValueError("input URL must be pinned HTTPS without credentials/fragment")
        if "latest" in url.path.lower().split("/"):
            raise ValueError("moving latest URLs are forbidden")
    except (KeyError, TypeError, ValueError) as e:
        raise ValueError(f"package {package}: {e}") from e


def select_packages(man, profile, only=None, public_only=False, bundle_class="public"):
    if bundle_class != "public": raise ValueError("only public packages may enter a release bundle")
    allowed = profile.get("open_packages", []) + profile.get("official_packages", [])
    if not allowed or len(set(allowed)) != len(allowed):
        raise ValueError("open_packages must be nonempty and unique")
    if set(allowed) - OPEN_PACKAGES:
        raise ValueError("forbidden open_packages: " + ",".join(sorted(set(allowed) - OPEN_PACKAGES)))
    target = "public-ok"
    defaults = profile.get("open_packages", [])
    want = list(defaults) if only is None else list(only)
    if not want:
        raise ValueError("empty package selection")
    byid = {}
    for p in man.get("packages", []):
        if p["id"] in byid:
            raise ValueError("duplicate package: " + p["id"])
        byid[p["id"]] = p
    selected, active, seen = [], set(), set()
    def visit(pid, dependency=False):
        if pid not in allowed or pid not in byid:
            raise ValueError(f"package {pid}: outside open_packages or absent from manifest")
        if pid in active:
            raise ValueError(f"package {pid}: dependency cycle")
        if pid in seen:
            return
        active.add(pid)
        p = byid[pid]
        if bundle_class == "public" and pid not in PUBLIC_PACKAGES:
            raise ValueError(f"package {pid}: forbidden on public media")
        if p.get("compliance", {}).get("distribution") != target:
            raise ValueError(f"package {pid}: refused by {bundle_class} bundle class")
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
            visit(dep, dependency=True)
        active.remove(pid); seen.add(pid); selected.append(p)
    if man.get("schema") != 1:
        raise ValueError("unsupported manifest schema")
    for pid in sorted(want):
        visit(pid)
    bundle_compliance.validate(selected, validate_pin, bundle_class == "public")
    return selected


def verify_file(package, fl, path):
    path = Path(path)
    if path.is_symlink() or not path.is_file() or path.stat().st_size != fl["size"] or sha256(path) != fl["sha256"]:
        raise InputDrift(f"INPUT_DRIFT package {package}/{fl['name']}: missing file or size/sha256 mismatch")


class _HTTPSRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if urllib.parse.urlsplit(newurl).scheme != "https":
            raise ValueError("non-HTTPS redirect refused")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def fetch_file(package, fl, destination, retries=4, image=False, private=False):
    """Resume a .part using validated Range responses; promote only verified bytes.

    Existing bad cache entries fail closed. Transport/truncation retries use the
    same URL; hash mismatch is terminal. Never rewrite a pin or choose a mirror.
    """
    validate_pin(package, fl, image=image)
    dest = Path(destination)
    if dest.exists() or dest.is_symlink():
        verify_file(package, fl, dest)
        print(f"CACHE_OK package={package} bytes={fl['size']} sha256={fl['sha256']}" if private else f"CACHE_OK {package}/{fl['name']}", flush=True)
        return
    print(f"FETCH package={package} bytes={fl['size']} sha256={fl['sha256']}" if private else f"FETCH {package}/{fl['name']} bytes={fl['size']}", flush=True)
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
        except urllib.error.HTTPError as e:
            if e.code in (404, 410):
                raise InputDrift(f"INPUT_DRIFT package {package}/{fl['name']}: HTTP {e.code}") from None
            error = "HTTP " + str(e.code)
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
        raise InputDrift(f"INPUT_DRIFT package {package}/{fl['name']}: sha256 mismatch (partial deleted)")
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
        files["LICENSES/kayfabe/" + name] = (root.parents[3] / name).read_bytes().replace(b"\r\n", b"\n")
    return files


def identity(records):
    # Identity hashes the payload; KFAPPS.ID is excluded to avoid a self-reference.
    records = {k: dict(v) for k, v in records.items() if k != "KFAPPS.ID"}
    for name, record in records.items():
        if (name in {"tools/" + p for p in bundle_repro.PROGRAMS}
                and record.get("reproducibility", {}).get("mode") == "inventory-only"):
            # This marker must not reintroduce the explicitly waived byte check.
            record.update(sha256="inventory-only", size="inventory-only")
    return hashlib.sha256(json.dumps(records, sort_keys=True).encode("utf-8")).hexdigest()


def iso_input_hash(packages, toolchain, open_packages, root=HERE):
    """Hash the complete ISO recipe, excluding clocks, host paths and receipts."""
    import bundle_tools
    root = Path(root)
    contents = repo_files(root)
    for name in ("package_bundle.py", "bundle_iso.py", "bundle_tools.py", "bundle_compliance.py", "bundle_wheels.py", "bundle_repro.py"):
        contents["recipe/" + name] = (root / name).read_bytes().replace(b"\r\n", b"\n")
    for name in bundle_tools.SOURCES:
        contents["source/" + name] = (root.parents[3] / name).read_bytes().replace(b"\r\n", b"\n")
    record = dict(version=BUILDER_VERSION, packages=packages, toolchain=toolchain,
                  open_packages=sorted(open_packages),
                  files={k: hashlib.sha256(v).hexdigest() for k, v in sorted(contents.items())})
    return hashlib.sha256(canonical(record)).hexdigest()


def input_hash(man, profile, root=HERE, bundle_class="public"):
    import bundle_tools
    packages = select_packages(man, profile, bundle_class=bundle_class)
    toolchain = bundle_tools.check_toolchain(man.get("bundle_toolchain", {}))
    return iso_input_hash(packages, toolchain, profile["open_packages"], root)


def pin_text(image_sha, inputs, bundle_class="public", inventory_sha=None):
    if bundle_class != "public": raise ValueError("transient media has no repository pin")
    filename = "winapps-open.iso"
    if not all(re.fullmatch(r"[0-9a-f]{64}", x) for x in (image_sha, inputs)):
        raise ValueError("invalid pin digest")
    if inventory_sha is not None and not re.fullmatch(r"[0-9a-f]{64}", inventory_sha): raise ValueError("invalid inventory digest")
    return (f"{filename} sha256={image_sha}\n# inputs={inputs}\n" +
            (f"# inventory={inventory_sha}\n" if inventory_sha is not None else "") +
            (f"# iso_key=winapps-open/kfapps-{image_sha}.iso\n"
             f"# manifest_key=winapps-open/manifest-{image_sha}.json\n"))


def build_image(packages, downloads, tools, output, toolchain, root=HERE, probe_receipt=None, open_packages=None, bundle_class="public"):
    """Assemble already pinned inputs. Compiler/upload/network are outside this function."""
    if bundle_class != "public": raise ValueError("release image must be public; KFAPPSP is transient-only")
    if any(p["id"] in OPEN_PACKAGES - PUBLIC_PACKAGES for p in packages):
        raise ValueError("official inputs belong only in transient run media")
    bundle_compliance.validate(packages, validate_pin, bundle_class == "public")
    if any(p["compliance"]["distribution"] != "public-ok" for p in packages):
        raise ValueError("package crosses bundle class boundary")
    if bundle_class == "public" and any(p["id"] not in PUBLIC_PACKAGES for p in packages):
        raise ValueError("forbidden package on public media")
    basename = "winapps-open"
    label = "KFAPPS"
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    files = package_files(packages, downloads)
    expected = {}
    for p in packages:
        for fl in p["files"]:
            path = f"pkg/{p['id']}/{fl['name']}"
            verify_file(p["id"], fl, files[path])
            expected[path] = {k: fl[k] for k in ("sha256", "size")}
    compliance_files, license_records, sources = bundle_compliance.stage(packages, downloads)
    files.update(compliance_files)
    files.update(repo_files(root))
    files.update({("LICENSES/zig/" + name[len("licenses/zig/"):] if name.startswith("licenses/zig/") else "tools/" + name):
                  Path(path) for name, path in sorted(tools.items())})
    files["stage.json"] = canonical(stage_table(packages))
    # No wall clock, host paths, Python version or host-specific toolchain selection in ISO.
    inputs = iso_input_hash(packages, toolchain, open_packages if open_packages is not None else [p["id"] for p in packages], root)
    files["bundle.inputs.json"] = canonical(dict(schema=1, inputs=inputs, packages=packages, toolchain=toolchain, bundle_class=bundle_class))
    for name, source in files.items():
        if name not in expected:
            expected[name] = dict(sha256=hashlib.sha256(source).hexdigest(), size=len(source)) if isinstance(source, bytes) else dict(sha256=sha256(source), size=source.stat().st_size)
        own_licenses = bundle_compliance.TOOL_LICENSE if name.startswith(("tools/", "LICENSES/zig/")) else bundle_compliance.REPO_LICENSE
        expected[name]["licenses"] = license_records.get(name, own_licenses)
        expected[name]["reproducibility"] = bundle_repro.policy(name, toolchain)
    bundle_repro.inventory(dict(files=expected, toolchain=toolchain))
    ident = identity(expected)
    files["KFAPPS.ID"] = (ident + "\n").encode("ascii")
    expected["KFAPPS.ID"] = dict(sha256=hashlib.sha256(files["KFAPPS.ID"]).hexdigest(), size=len(files["KFAPPS.ID"]))
    expected["KFAPPS.ID"]["licenses"] = bundle_compliance.REPO_LICENSE
    expected["KFAPPS.ID"]["reproducibility"] = dict(mode="sha256")
    iso = output / (basename + ".iso")
    temp = output / (basename + ".pending.iso")
    try:
        actual = bundle_iso.write_image(files, temp, label=label)
        # Compare hashes of bytes actually copied, catching source races/corruption.
        if any(actual[k] != {field: v[field] for field in ("sha256", "size")} for k, v in expected.items()):
            raise ValueError("input changed during image assembly")
        receipt = dict(schema=1, inputs=inputs, packages=packages, files=expected, sources=sources, bundle_class=bundle_class,
            image=dict(file=iso.name, label=label, format="iso9660+joliet", identity=ident,
                       size=temp.stat().st_size, sha256=sha256(temp)),
            toolchain=toolchain, packaging_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
            python_version=sys.version, writer="bundle_iso.py/v1", guest_root="C:\\kfapps")
        receipt["inventory_sha256"] = bundle_repro.inventory_hash(receipt)
        with bundle_iso.Reader(temp) as image:
            receipt["container_metadata"] = image.metadata()
        receipt["recipe_sha256"] = {name: hashlib.sha256((HERE / name).read_bytes().replace(b"\r\n", b"\n")).hexdigest()
                                     for name in ("package_bundle.py", "bundle_iso.py", "bundle_tools.py", "bundle_compliance.py", "bundle_wheels.py", "bundle_repro.py")}
        if probe_receipt is not None:
            receipt["probe_build"] = probe_receipt
        verify_image(temp, receipt)
        os.replace(temp, iso)
    finally:
        temp.unlink(missing_ok=True)
    (output / (basename + ".manifest.json")).write_bytes(canonical(receipt))
    pin_line = pin_text(receipt["image"]["sha256"], inputs, bundle_class, receipt["inventory_sha256"]).encode("ascii")
    (output / (basename + ".pin.line")).write_bytes(pin_line)
    if bundle_class == "public": (output / "bundle.pin.line").write_bytes(pin_line)
    return receipt


def verify_image(iso, receipt, public_only=False):
    if Path(iso).stat().st_size != receipt["image"]["size"] or sha256(iso) != receipt["image"]["sha256"]:
        raise ValueError("image size/sha256 mismatch")
    expected = receipt["files"]
    if not expected or receipt["image"]["identity"] != identity(expected):
        raise ValueError("image identity mismatch")
    bundle_class = receipt.get("bundle_class")
    if bundle_class != "public":
        raise ValueError("invalid or private bundle class")
    with bundle_iso.Reader(iso) as r:
        if r.label != receipt["image"]["label"] or r.label != "KFAPPS":
            raise ValueError("bundle class/label mismatch")
        if "KFAPPS.ID" not in expected or set(r.files) != set(expected):
            raise ValueError("image file list mismatch")
        got = r.records()
        for name, record in expected.items():
            pin = {field: record[field] for field in ("sha256", "size")}
            if got[name] != pin:
                raise ValueError("image file size/sha256 mismatch: " + name)
            if not record.get("licenses"):
                raise ValueError("missing per-file licence record: " + name)
        if b"".join(r.chunks("KFAPPS.ID")) != (identity(expected) + "\n").encode("ascii"):
            raise ValueError("KFAPPS.ID mismatch")
        embedded = json.loads(b"".join(r.chunks("bundle.inputs.json")))
        if receipt.get("inputs") != embedded.get("inputs"):
            raise ValueError("ISO input hash mismatch")
        if receipt["packages"] != embedded["packages"] or receipt["toolchain"] != embedded["toolchain"] or bundle_class != embedded.get("bundle_class"):
            raise ValueError("ISO package/toolchain record mismatch")
        bundle_compliance.validate(receipt["packages"], validate_pin, public_only or bundle_class == "public")
        if any(p["compliance"]["distribution"] != "public-ok" for p in receipt["packages"]):
            raise ValueError("receipt crosses bundle class boundary")
        source_records = [dict(package=p["id"], path=f"SOURCE/{p['id']}/{s['name']}", key=bundle_compliance.source_key(s, bundle_class), **s)
                          for p in sorted(embedded["packages"], key=lambda p: p["id"]) for s in p["compliance"]["sources"]]
        if receipt.get("sources") != source_records:
            raise ValueError("source mirror coverage mismatch")
        for source in receipt.get("sources", []):
            if (source["path"] not in expected or source["sha256"] != expected[source["path"]]["sha256"] or
                    source["size"] != expected[source["path"]]["size"] or source["key"] != bundle_compliance.source_key(source, bundle_class)):
                raise ValueError("source mirror receipt mismatch")

    inventory_sha = bundle_repro.inventory_hash(receipt)
    if receipt.get("inventory_sha256", inventory_sha) != inventory_sha:
        raise ValueError("per-file inventory digest mismatch")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("command", choices=("check", "fetch", "build", "verify", "verify-image", "compare-image", "layout", "input-hash"))
    ap.add_argument("--class", dest="bundle_class", choices=("public", "private"), default="public", help="public release media; the retired private class is refused")
    ap.add_argument("--ci-build", action="store_true", help="explicit credential-free public CI build (KF_RUN_TOKEN still forbidden)")
    ap.add_argument("--manifest", type=Path, default=HERE / "manifest.json")
    ap.add_argument("--profiles", type=Path, default=HERE / "gate_profiles.json")
    ap.add_argument("--dl", type=Path, default=Path("out/downloads"))
    ap.add_argument("--out", type=Path, default=Path("out"))
    ap.add_argument("--only", help="comma-separated open package IDs (dependencies included)")
    ap.add_argument("--public-only", action="store_true", help="refuse every selected package outside public-ok (never silently drop packages)")
    ap.add_argument("--iso", type=Path, help="image for verify-image")
    ap.add_argument("--receipt", type=Path, help="build receipt for verify-image")
    ap.add_argument("--candidate-receipt", type=Path, help="second image's own receipt for compare-image; defaults next to --iso")
    args = ap.parse_args(argv)
    package_id = "bundle"
    try:
        if args.public_only and args.bundle_class != "public":
            raise ValueError("--public-only is a compatibility alias for --class public")
        guard(allow_ci=args.ci_build or args.command == "input-hash", bundle_class=args.bundle_class)
        if args.command == "input-hash":
            man, profile = load(args.manifest), load(args.profiles)
            packages = select_packages(man, profile, args.only.split(",") if args.only is not None else None, args.public_only, args.bundle_class)
            import bundle_tools
            print("inputs=" + iso_input_hash(packages, bundle_tools.check_toolchain(man.get("bundle_toolchain", {})), profile["open_packages"]))
            return 0
        basename = "winapps-open"
        # A single HTTPS-only opener prevents redirect downgrade, while keeping
        # the urllib call easy to fake in offline tests.
        urllib.request.install_opener(urllib.request.build_opener(_HTTPSRedirect()))
        if args.command == "compare-image":
            iso = args.iso or args.out / (basename + ".iso")
            reference = load(args.receipt)
            candidate = load(args.candidate_receipt or iso.with_suffix(".manifest.json"))
            candidate = bundle_repro.read_candidate(iso, candidate)
            result = bundle_repro.compare(reference, candidate)
            bundle_repro.report(result)
            (args.out / "reproduction.json").parent.mkdir(parents=True, exist_ok=True)
            (args.out / "reproduction.json").write_bytes(canonical(result))
            return 0 if result["level1"] == "equal" else 1
        if args.command == "verify-image":
            receipt = load(args.receipt or args.out / (basename + ".manifest.json"))
            if receipt.get("bundle_class") != args.bundle_class: raise ValueError("requested/receipt class differs")
            verify_image(args.iso or args.out / (basename + ".iso"), receipt, args.public_only)
        else:
            man, profile = load(args.manifest), load(args.profiles)
            packages = select_packages(man, profile, args.only.split(",") if args.only is not None else None, args.public_only, args.bundle_class)
            import bundle_tools
            toolchain = bundle_tools.check_toolchain(man.get("bundle_toolchain", {}))
            if args.command == "layout":
                paths = set(package_files(packages, args.dl)) | set(repo_files()) | {"KFAPPS.ID", "stage.json", "bundle.inputs.json"}
                paths |= {"tools/" + n for n in bundle_tools.TOOL_NAMES}
                paths |= {"LICENSES/zig/" + n[len("licenses/zig/"):] for n in bundle_tools.NOTICE_NAMES}
                print("LAYOUT private file list withheld") if args.bundle_class == "private" else print("\n".join(sorted(paths)))
                print("COMPLIANCE LICENSES/<package>/<text-sha256>, LICENSES/index.json, THIRD_PARTY_NOTICES.txt")
                for pid, pin, relative in bundle_compliance.inputs(packages):
                    print(f"COMPLIANCE_INPUT package={pid} bytes={pin['size']} sha256={pin['sha256']}")
                host, pin = bundle_tools.host_pin(toolchain)
                print(f"DOWNLOAD_ONLY _toolchain/{pin['name']} host={host} (compiler; not on ISO)")
            elif args.command in ("fetch", "verify", "build"):
                for p in packages:
                    package_id = p["id"]
                    for fl in p["files"]:
                        dest = args.dl / p["id"] / fl["name"]
                        if args.command == "fetch":
                            fetch_file(p["id"], fl, dest, private=args.bundle_class == "private")
                        else:
                            verify_file(p["id"], fl, dest)
                for pid, fl, relative in bundle_compliance.inputs(packages):
                    if args.command == "fetch":
                        fetch_file(pid, fl, args.dl / relative, private=args.bundle_class == "private")
                    else:
                        verify_file(pid, fl, args.dl / relative)
                host, pin = bundle_tools.host_pin(toolchain)
                archive = args.dl / "_toolchain" / pin["name"]
                if args.command == "fetch":
                    fetch_file("zig-" + host, pin, archive, private=args.bundle_class == "private")
                else:
                    verify_file("zig-" + host, pin, archive)
                if args.command == "build":
                    tools = bundle_tools.build_tools(toolchain, archive, args.out / "build")
                    receipt = build_image(packages, args.dl, tools, args.out, toolchain,
                                          probe_receipt=load(args.out / "build" / "tools.receipt.json"),
                                          open_packages=profile["open_packages"], bundle_class=args.bundle_class)
                    print("BUILD_DONE sha256=" + receipt["image"]["sha256"] + " identity=" + receipt["image"]["identity"])
        print(args.command.upper().replace("-", "_") + "_OK")
        return 0
    except (OSError, ValueError, KeyError, TypeError) as e:
        print("REFUSED: permanent private bundles retired; use official run-time fetching" if args.bundle_class != "public" else "FAIL " + str(e), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
