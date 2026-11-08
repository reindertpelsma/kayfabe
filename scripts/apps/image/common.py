# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Shared stdlib-only pin, archive and inventory contracts (host and guest)."""

import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import tarfile
import urllib.parse
import urllib.request
import zipfile

SHA = re.compile(r"[0-9a-f]{64}\Z")
ID = re.compile(r"[a-zA-Z0-9][a-zA-Z0-9._+-]*\Z")
AGENT = "kayfabe-apps-image/1.0 (+https://github.com/reindertpelsma/kayfabe)"
MAX_RESULT = 16 * 1024 * 1024


def canonical(value):
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=True
    ).encode()


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def relative_path(path):
    p = PurePosixPath(path)
    if not path or p.is_absolute() or ".." in p.parts or "\\" in path or "\x00" in path:
        raise ValueError("unsafe relative path")
    return p


def absolute_path(path):
    p = PurePosixPath(path)
    if not p.is_absolute() or ".." in p.parts or "\\" in path or "\x00" in path:
        raise ValueError("unsafe absolute path")
    return p


def check_argv(argv):
    if (
        not isinstance(argv, list)
        or not argv
        or any(not isinstance(x, str) or not x or "\x00" in x for x in argv)
    ):
        raise ValueError("command must be a nonempty argv list")
    if PurePosixPath(argv[0]).name in {
        "sh",
        "bash",
        "dash",
        "zsh",
        "fish",
        "csh",
        "env",
        "sudo",
    }:
        raise ValueError("shell/wrapper commands are forbidden")
    if any(
        x in {";", "&&", "||", "|", ">", ">>", "<"} or "$(" in x or "`" in x
        for x in argv
    ):
        raise ValueError("shell syntax is forbidden")


def validate_pin(pin):
    if not ID.fullmatch(pin.get("id", "")) or not SHA.fullmatch(pin.get("sha256", "")):
        raise ValueError("input needs an id and SHA256 pin")
    if type(pin.get("size")) is not int or pin["size"] <= 0:
        raise ValueError("input needs a positive pinned size")
    u = urllib.parse.urlsplit(pin.get("url", ""))
    if u.scheme != "https" or not u.hostname or u.username or u.password or u.fragment:
        raise ValueError("input URL must be HTTPS without credentials or fragments")


def validate_recipe(recipe):
    try:
        if recipe["schema_version"] != 2:
            raise ValueError("unsupported recipe schema")
        if not recipe["recipe_version"]:
            raise ValueError("missing recipe version")
        validate_pin(recipe["base"])
        apt = recipe["apt"]
        if not re.fullmatch(r"20\d{6}T\d{6}Z", apt["snapshot"]):
            raise ValueError("apt snapshot must be immutable")
        if not apt["packages"] or any(
            not re.fullmatch(r"[a-z0-9][a-z0-9+.:-]*(?:=[a-zA-Z0-9+.~:-]+)?", p)
            for p in apt["packages"]
        ):
            raise ValueError("invalid package list")
        if apt["suites"] != ["noble", "noble-updates", "noble-security"]:
            raise ValueError("unexpected apt suites")
        guest = recipe["guest"]
        if not re.fullmatch(r"[a-z_][a-z0-9_-]*", guest["user"]):
            raise ValueError("invalid guest user")
        for field, minimum in [("disk_gib", 60), ("memory_mib", 4096), ("cpus", 1)]:
            if type(guest[field]) is not int or guest[field] < minimum:
                raise ValueError("guest resource too small")
        if recipe["image_policy"]["nvidia_notice_policy"] != "documented-oss-only":
            raise ValueError("unsupported NVIDIA scan policy")
        if apt["components"] != ["main", "universe"]:
            raise ValueError("open image permits only main/universe")
        seen = {recipe["base"]["id"]}
        for a in recipe["artifacts"] + recipe["official_fetch"]:

            validate_pin(a)
            if a["id"] in seen:
                raise ValueError("duplicate input id")
            seen.add(a["id"])
            if a["kind"] not in {"file", "archive", "wheel", "deb", "run"}:
                raise ValueError("invalid input kind")
            if a["kind"] == "wheel":
                if not a["filename"].endswith(".whl") or "/" in a["filename"]:
                    raise ValueError("invalid wheel filename")
                if not a["package"] or not a["version"]:
                    raise ValueError("wheel needs exact version")
            elif a["kind"] == "deb":
                if not a["filename"].endswith(".deb") or "/" in a["filename"]:
                    raise ValueError("invalid deb filename")
            elif a["kind"] == "run":
                if a not in recipe["official_fetch"]:
                    raise ValueError("installer must be official_fetch only")
            else:
                absolute_path(a["target"])
        for a in recipe["official_fetch"]:
            if not a.get("licence") or not a.get("install_argv"):
                raise ValueError("official input needs licence and install argv")
            check_argv(a["install_argv"])
        for a in recipe["artifacts"]:
            vendor = (urllib.parse.urlsplit(a["url"]).hostname or "").endswith(
                "nvidia.com"
            )
            package = a.get("package", "").lower()
            if (
                vendor
                or package.startswith(("nvidia-", "libnvidia-", "cuda-", "libcudnn"))
                or package in {"triton", "cupy-cuda12x"}
                or (
                    package in {"torch", "torchvision"}
                    and "+cu" in a.get("version", "")
                )
            ):
                raise ValueError("NVIDIA input cannot be part of open artifacts")
        targets = set()
        for f in recipe["local_files"]:
            relative_path(f["path"])
            absolute_path(f["target"])
            if not SHA.fullmatch(f.get("sha256", "")):
                raise ValueError("local file needs a SHA256 pin")
            if f["target"] in targets:
                raise ValueError("duplicate local target")
            targets.add(f["target"])
        names = set()
        for b in recipe["builds"]:
            if not ID.fullmatch(b["name"]) or b["name"] in names:
                raise ValueError("invalid/duplicate build name")
            names.add(b["name"])
            if b.get("requires") not in {"open-layer", "nvidia-layer-static"}:
                raise ValueError("build layer must be explicit")
            if b["source"] != "local" and b["source"] not in seen:
                raise ValueError("build source is not pinned")
            absolute_path(b["cwd"])
            if not b["commands"] or not b["outputs"]:
                raise ValueError("build needs commands and outputs")
            for cmd in b["commands"]:
                check_argv(cmd)
            for o in b["outputs"]:
                absolute_path(o["path"])
                if o["sha256"] != "inventory-only" and not SHA.fullmatch(o["sha256"]):
                    raise ValueError(
                        "build output must be pinned or explicitly inventory-only"
                    )
        for f in recipe["write_files"]:
            absolute_path(f["path"])
        for path in (
            recipe["inventory"]["required_paths"] + recipe["inventory"]["trees"]
        ):
            absolute_path(path)
    except (KeyError, TypeError, AttributeError) as e:
        raise ValueError("incomplete recipe") from e
    return recipe


def read_recipe(path):
    return validate_recipe(json.loads(Path(path).read_text()))


def verify_file(path, pin):
    if Path(path).stat().st_size != pin["size"] or sha256_file(path) != pin["sha256"]:
        raise ValueError(f"size/SHA256 mismatch for {pin['id']}")


def download(pin, cache, opener=None):
    """Resume transport failures; corrupt content is deleted and never falls back to a mirror."""
    validate_pin(pin)
    cache = Path(cache)
    cache.mkdir(parents=True, exist_ok=True)
    target = cache / pin["sha256"]
    if target.exists():
        verify_file(target, pin)
        return target
    part = target.with_suffix(".part")
    offset = part.stat().st_size if part.exists() else 0
    if offset > pin["size"]:
        part.unlink()
        raise ValueError("oversized partial download")
    headers = {"User-Agent": AGENT}
    if offset:
        headers["Range"] = f"bytes={offset}-"
    try:
        if offset < pin["size"]:
            request = urllib.request.Request(pin["url"], headers=headers)
            with (opener or urllib.request.urlopen)(request, timeout=60) as response:
                if offset and getattr(response, "status", None) != 206:
                    offset = 0
                if offset and not response.headers.get("Content-Range", "").startswith(
                    f"bytes {offset}-"
                ):
                    raise ValueError("invalid resume range")
                with open(part, "ab" if offset else "wb") as f:
                    size = offset
                    for block in iter(lambda: response.read(1024 * 1024), b""):
                        size += len(block)
                        if size > pin["size"]:
                            raise ValueError("oversized download")
                        f.write(block)
        verify_file(part, pin)
        part.replace(target)
    except ValueError:
        part.unlink(missing_ok=True)
        raise
    return target


def extract(source, destination, strip_components=1):
    """Extract only regular files, directories and confined symlinks; never special files."""
    destination = Path(destination)
    destination.mkdir(parents=True, exist_ok=True)

    def stripped(name):
        parts = relative_path(name).parts
        if len(parts) <= strip_components:
            return None
        return str(PurePosixPath(*parts[strip_components:]))

    try:
        if zipfile.is_zipfile(source):
            with zipfile.ZipFile(source) as z:
                for info in z.infolist():
                    name = stripped(info.filename)
                    if name is None:
                        continue
                    mode = info.external_attr >> 16
                    if stat.S_ISLNK(mode):
                        raise ValueError("zip symlink refused")
                    out = destination / name
                    if not out.resolve().is_relative_to(destination.resolve()):
                        raise ValueError("zip escape")
                    if info.is_dir():
                        out.mkdir(parents=True, exist_ok=True)
                        continue
                    out.parent.mkdir(parents=True, exist_ok=True)
                    with z.open(info) as src, open(out, "wb") as dst:
                        shutil.copyfileobj(src, dst)
                    out.chmod(0o755 if mode & 0o111 else 0o644)
        else:
            with tarfile.open(source) as t:

                def data_filter(member, dest):
                    name = stripped(member.name)
                    if name is None:
                        return None
                    if not (member.isfile() or member.isdir() or member.issym()):
                        raise ValueError("tar special file/hardlink refused")
                    return tarfile.data_filter(member.replace(name=name), dest)

                t.extractall(destination, filter=data_filter)
    except (tarfile.TarError, zipfile.BadZipFile) as e:
        raise ValueError("unsafe/invalid archive") from e


def locked_wheels(recipe, layer="open"):
    rows = {
        a["package"]: (a["version"], a["sha256"])
        for a in recipe["artifacts"]
        if a["kind"] == "wheel"
    }
    if layer == "runtime":
        for a in recipe["official_fetch"]:
            if a["kind"] == "wheel":
                rows[a["package"]] = (a["version"], a["sha256"])
    return {f"{name}=={version}": sha for name, (version, sha) in rows.items()}


def requirements(recipe, layer="open"):
    return "".join(
        f"{name} --hash=sha256:{sha}\n"
        for name, sha in sorted(locked_wheels(recipe, layer).items())
    )


def validate_inventory(inv, recipe):
    """A syntactically valid empty report must never certify an incomplete install."""
    try:
        if inv["schema_version"] != 1 or not inv["dpkg"] or not inv["files"]:
            raise ValueError("empty/unsupported inventory")
        layer = inv["layer"]
        if layer not in {"open", "runtime"}:
            raise ValueError("unknown inventory layer")
        if inv["guest"]["kernel"] != recipe["guest"]["kernel"] or (
            layer == "runtime"
            and inv["guest"]["nvidia_module_version"]
            != recipe["guest"]["nvidia_version"]
        ):
            raise ValueError("guest kernel/NVIDIA module differs from pin")
        if layer == "open" and (
            not inv["nvidia_scan"]["clean"]
            or inv["nvidia_scan"]["findings"]
            or not inv["no_nvidia_proprietary_files"]
        ):
            raise ValueError("NVIDIA-file scan failed")
        if layer == "open" and inv["guest"].get("nvidia_module_version"):
            raise ValueError("NVIDIA module in open image")
        if layer == "runtime":
            receipts = {
                a["id"]: a["sha256"]
                for a in recipe["official_fetch"]
                if a["kind"] != "run"
                or a["version"] == recipe["guest"]["nvidia_version"]
            }
            if inv["official_receipts"] != receipts:
                raise ValueError("official input receipts differ")
        inventory_spec = (
            recipe["inventory"] if layer == "open" else recipe["runtime_inventory"]
        )
        packages = {}
        for row in inv["dpkg"]:
            status, name, architecture, version = row.split("\t")
            if not status.startswith("ii"):
                raise ValueError("non-installed dpkg inventory row")
            packages[name.split(":", 1)[0]] = version
        requested = list(recipe["apt"]["packages"])
        requested.extend(
            a["package"] + "=" + a["version"]
            for a in recipe["artifacts"]
            if a["kind"] == "deb"
        )
        for spec in requested:
            name, separator, version = spec.partition("=")
            if name not in packages or (separator and packages[name] != version):
                raise ValueError("missing or wrong-version apt package: " + name)
        wheels = locked_wheels(recipe, layer)

        def normalized(spec):
            name, version = spec.split("==", 1)
            return re.sub(r"[-_.]+", "-", name).lower() + "==" + version

        if inv["wheels"] != wheels or sorted(
            map(normalized, inv["pip_freeze"])
        ) != sorted(wheels):
            raise ValueError("pip freeze or wheel pins differ from recipe")
        expected_names = {spec.split("==")[0] for spec in wheels}
        if set(inv["pip_files"]) != expected_names or any(
            not SHA.fullmatch(h) for h in inv["pip_files"].values()
        ):
            raise ValueError("missing pip installed-file hashes")
        files = inv["files"]
        if any(not SHA.fullmatch(h) for h in files.values()):
            raise ValueError("invalid file SHA256")
        for p in inventory_spec["required_paths"]:
            if p not in files:
                raise ValueError(f"missing inventory file: {p}")
        for name in inventory_spec["dpkg_binaries"]:
            if not any(PurePosixPath(path).name == name for path in files):
                raise ValueError("missing installed app: " + name)
        for a in recipe["artifacts"]:
            if a["kind"] == "file" and files.get(a["target"]) != a["sha256"]:
                raise ValueError(f"file pin differs: {a['id']}")
        for b in recipe["builds"]:
            if layer == "open" and b["requires"] == "nvidia-layer-static":
                continue
            for o in b["outputs"]:
                if o["path"] not in files:
                    raise ValueError("missing build output")
    except (KeyError, TypeError, AttributeError) as e:
        raise ValueError("incomplete inventory") from e


def inventory_input_pins(recipe, layer):
    """Declared inputs whose acquisition is hash-checked, never generated outputs."""
    official = [
        pin
        for pin in recipe["official_fetch"]
        if (layer == "runtime" or pin["kind"] != "wheel")
        and (
            pin["kind"] != "run" or pin["version"] == recipe["guest"]["nvidia_version"]
        )
    ]
    return sorted(
        [recipe["base"], *recipe["artifacts"], *recipe["apt"]["metadata"], *official],
        key=lambda pin: pin["id"],
    )


def source_refs(recipe):
    return {
        key: value
        for key, value in recipe.get("provenance", {}).items()
        if key.endswith(("_commit", "_tag", "_version"))
    }


def inventory_parts(inv, recipe, source_tooling=None):
    """Owner policy: compare PINNED inputs; record BUILT bytes for information."""
    validate_inventory(inv, recipe)
    input_paths = {a["target"] for a in recipe["artifacts"] if a["kind"] == "file"}
    pinned = {
        "layer": inv["layer"],
        "recipe_sha256": hashlib.sha256(canonical(recipe)).hexdigest(),
        "apt_snapshot": recipe["apt"]["snapshot"],
        "tooling": source_tooling or {},
        "dpkg": sorted(inv["dpkg"]),
        "pip_freeze": sorted(inv["pip_freeze"]),
        "wheels": inv["wheels"],
        "inputs": inventory_input_pins(recipe, inv["layer"]),
        "local_sources": recipe["local_files"],
        "source_refs": source_refs(recipe),
        "files": {
            path: sha for path, sha in inv["files"].items() if path in input_paths
        },
    }
    if inv["layer"] == "runtime":
        pinned["official_receipts"] = inv["official_receipts"]
    built = {
        key: value
        for key, value in inv.items()
        if key
        not in {
            "schema_version",
            "layer",
            "dpkg",
            "pip_freeze",
            "wheels",
            "files",
            "official_receipts",
        }
    }
    built["files"] = {
        path: sha for path, sha in inv["files"].items() if path not in input_paths
    }
    spec = (
        recipe["inventory"] if inv["layer"] == "open" else recipe["runtime_inventory"]
    )
    required = set(spec["required_paths"])
    for build in recipe["builds"]:
        if inv["layer"] == "runtime" or build["requires"] == "open-layer":
            required.update(output["path"] for output in build["outputs"])
    required.update(
        path
        for path in inv["files"]
        if PurePosixPath(path).name in spec["dpkg_binaries"]
    )
    built["required_apps"] = sorted(required)
    return {"pinned": pinned, "built": built}


def inventory_observation(parts):
    """Reconstruct the bounded guest observation for structural/policy validation."""
    try:
        pinned, built = parts["pinned"], parts["built"]
        if set(pinned["files"]) & set(built["files"]):
            raise ValueError("input and built file categories overlap")
        inv = {
            key: built[key]
            for key in (
                "guest",
                "pip_files",
                "nvidia_scan",
                "no_nvidia_proprietary_files",
                "symlinks",
            )
            if key in built
        }
        inv.update(
            schema_version=1,
            layer=pinned["layer"],
            dpkg=pinned["dpkg"],
            pip_freeze=pinned["pip_freeze"],
            wheels=pinned["wheels"],
            files={**pinned["files"], **built["files"]},
        )
        if "official_receipts" in pinned:
            inv["official_receipts"] = pinned["official_receipts"]
        return inv
    except (KeyError, TypeError, AttributeError) as error:
        raise ValueError("incomplete split inventory") from error
