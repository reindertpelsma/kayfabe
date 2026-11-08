# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Deterministic licence inventory and source mirrors; no application extraction.

Only licence/notice members are read from already hash-verified archives. Their
original paths and parent archive hashes are retained in LICENSES/index.json.
No vendor code is executed and no archive member is extracted onto the host.
Distribution classes are engineering policy records, not legal clearance.
"""
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import tarfile
import urllib.parse
import zipfile

MAX_TEXT = 4 * 1024**2
MAX_TOTAL = 64 * 1024**2
CLASSES = frozenset(("public-ok", "private-only"))
REPO_LICENSE = [dict(package="kayfabe", id="Apache-2.0 OR GPL-2.0-or-later",
                     upstream="https://github.com/reindertpelsma/kayfabe",
                     distribution="public-ok", conditions=[], sources=[])]
TOOL_LICENSE = REPO_LICENSE + [dict(package="zig-runtime", version="0.13.0",
    id="LicenseRef-Zig-0.13.0-Linked-Runtime-Notices",
    upstream="https://ziglang.org/download/0.13.0/", distribution="public-ok", conditions=[], sources=[],
    notices="LICENSES/zig/ (exact pinned compiler release notices)")]


def source_key(pin, bundle_class="public"):
    if not re.fullmatch(r"[0-9a-f]{64}", pin["sha256"]): raise ValueError("invalid source digest")
    if bundle_class != "public": raise ValueError("only public source mirrors are permitted")
    return "winapps-open/source-" + pin["sha256"] + ".archive"


def inputs(packages):
    """Extra pinned downloads, including exact source archives, never credentials."""
    for p in packages:
        c = p["compliance"]
        for pin in c["license_inputs"] + c["sources"]:
            yield p["id"], pin, Path("_compliance") / p["id"] / pin["name"]


def https(url):
    u = urllib.parse.urlsplit(url)
    return u.scheme == "https" and bool(u.hostname) and not (u.username or u.password or u.fragment)


def validate(packages, validate_pin, public_only=False):
    for p in packages:
        pid = p["id"]
        c = p.get("compliance", {})
        if not c.get("license_id") or not https(c.get("upstream", "")) or not p.get("version"):
            raise ValueError(f"package {pid}: missing compliance licence/upstream/exact version")
        if c.get("distribution") not in CLASSES:
            raise ValueError(f"package {pid}: invalid distribution class")
        conditions = c.get("conditions")
        if not isinstance(conditions, list) or any(not isinstance(x, str) or not x.strip() for x in conditions):
            raise ValueError(f"package {pid}: invalid distribution conditions")
        if c["distribution"] != "public-ok" and not conditions:
            raise ValueError(f"package {pid}: distribution conditions must be listed")
        if public_only and c["distribution"] != "public-ok":
            raise ValueError(f"package {pid}: --public-only refuses {c['distribution']}")
        names = set()
        for pin in c.get("license_inputs", []) + c.get("sources", []):
            validate_pin(pid, pin)
            if pin["name"].casefold() in names: raise ValueError(f"package {pid}: duplicate compliance input")
            names.add(pin["name"].casefold())
        if any(pin["size"] > MAX_TEXT for pin in c.get("license_inputs", [])):
            raise ValueError(f"package {pid}: upstream licence text exceeds limit")
        sources = c.get("sources")
        if not isinstance(sources, list) or any(s.get("version") != p["version"] for s in sources):
            raise ValueError(f"package {pid}: source version must exactly match binary version")
        if re.search(r"(?:^|\W)(?:GPL|LGPL|AGPL)-", c["license_id"]) and not sources:
            raise ValueError(f"package {pid}: copyleft requires pinned mirrored source")
        rules = c.get("license_rules", {})
        if set(rules) != {f["name"] for f in p["files"]}:
            raise ValueError(f"package {pid}: licence coverage must match every binary input")
        license_names = {f["name"] for f in c.get("license_inputs", [])}
        source_names = {f["name"] for f in sources}
        package_names = {f["name"] for f in p["files"]}
        for name, rule in rules.items():
            kind = rule.get("kind")
            if kind not in ("zip", "upstream", "source", "file"):
                raise ValueError(f"package {pid}/{name}: unknown licence rule")
            if kind == "upstream" and rule.get("input") not in license_names:
                raise ValueError(f"package {pid}/{name}: missing pinned upstream licence")
            if kind == "source" and rule.get("input") not in source_names:
                raise ValueError(f"package {pid}/{name}: missing pinned source licence")
            if kind == "file" and (rule.get("file") not in package_names or
                    not legal_name(rule["file"])):
                raise ValueError(f"package {pid}/{name}: licence file must name pinned legal text")
            if rule.get("fallback") and rule["fallback"] not in license_names:
                raise ValueError(f"package {pid}/{name}: unpinned fallback licence")
        if not isinstance(c.get("license_inputs"), list): raise ValueError(f"package {pid}: missing licence inputs")
        if any(f["name"].endswith(".whl") for f in p["files"]):
            components = c.get("components", {})
            if set(components) != {f["name"] for f in p["files"]}:
                raise ValueError(f"package {pid}: missing exact wheel components")
            for name, component in components.items():
                if not component.get("version") or not component.get("license_id") or not https(component.get("upstream", "")):
                    raise ValueError(f"package {pid}/{name}: incomplete wheel compliance")
            if c["distribution"] == "public-ok":
                import bundle_wheels
                for fl in p["files"]:
                    if not bundle_wheels.permissive(components[fl["name"]]["license_id"]) or fl.get("public_audit", {}).get("archive_sha256") != fl["sha256"]:
                        raise ValueError(f"package {pid}: public wheel needs a reviewed permissive/content audit")


def legal_name(name):
    # Include bundled third-party notices, exceptions and copyrights. Directories
    # such as Blender's license/ also carry texts with project-specific basenames.
    parts = PurePosixPath(name.replace("\\", "/")).parts
    base = parts[-1].lower() if parts else ""
    return bool(re.search(r"licen[cs]e|copying|copyright|notice|eula|third.party|acknowledg", base) or
                any(x.lower() in ("license", "licenses", "licence", "licences") for x in parts[:-1]))


def _archive_legal_members(path, kind):
    if kind == "zip":
        with zipfile.ZipFile(path) as z:
            entries = z.infolist()
            if len(entries) > 200000: raise ValueError("too many archive members")
            for entry in sorted(entries, key=lambda x: x.filename):
                if entry.is_dir() or not legal_name(entry.filename): continue
                if (entry.external_attr >> 16) & 0o170000 == 0o120000: raise ValueError("licence member is a link")
                if not 0 < entry.file_size <= MAX_TEXT: raise ValueError("invalid licence text size")
                yield entry.filename, z.read(entry)
    else:
        # Only source archives, never application payloads; streaming avoids a
        # tar member table proportional to an untrusted archive's entry count.
        with tarfile.open(name=None if hasattr(path, "read") else path,
                          fileobj=path if hasattr(path, "read") else None,
                          mode="r|*", encoding="utf-8", errors="surrogateescape") as t:
            for count, entry in enumerate(t):
                if count >= 200000: raise ValueError("too many source members")
                if not legal_name(entry.name) or entry.isdir(): continue
                if not entry.isfile(): raise ValueError("source licence member is not a regular file")
                if not 0 < entry.size <= MAX_TEXT: raise ValueError("invalid licence text size")
                with t.extractfile(entry) as f: yield entry.name, f.read(MAX_TEXT + 1)


def legal_members(path, kind):
    try:
        yield from _archive_legal_members(path, kind)
    except (OSError, EOFError, ValueError, zipfile.BadZipFile, tarfile.TarError, RuntimeError, NotImplementedError) as error:
        path = Path(getattr(path, "display_path", path))
        raise ValueError(f"package {path.parent.name}/{path.name}: invalid licence archive ({type(error).__name__})") from None


def read_text(path):
    with (path if hasattr(path, "open") else Path(path)).open("rb") as source:
        return source.read(MAX_TEXT + 1)


def licenses(p, filename=None):
    c = p["compliance"]; component = c.get("components", {}).get(filename, {})
    return [dict(package=p["id"], component=filename, version=component.get("version", p["version"]),
                 id=component.get("license_id", c["license_id"]), upstream=component.get("upstream", c["upstream"]),
                 distribution=c["distribution"], conditions=c["conditions"],
                 sources=[dict(version=s["version"], url=s["url"], sha256=s["sha256"], key=source_key(s, "public" if c["distribution"] == "public-ok" else "private")) for s in c["sources"]])]


def stage(packages, downloads, *, image=None, records=None):
    """Derive legal files from pinned bytes, from a cache or a bounded ISO view.

    ISO verification replays the same recipe without extracting app archives or
    trusting the producer's per-file receipt. records comes from Reader.records.
    """
    import package_bundle as B
    validate(packages, B.validate_pin)
    def checked(owner, pin, relative, iso_name):
        if image is None:
            path = Path(downloads) / relative; B.verify_file(owner, pin, path)
            return path
        if records.get(iso_name) != {k: pin[k] for k in ("sha256", "size")}:
            raise B.InputDrift(f"INPUT_DRIFT package {owner}/{pin['name']}: ISO bytes differ from pin")
        return image.member(iso_name)

    files, meta, index, sources = {}, {}, [], []
    notices = ["Kayfabe Windows app bundle: THIRD PARTY NOTICES", "",
        "Distributor: Reindert Pelsma / kayfabe", "Contact: https://github.com/reindertpelsma/kayfabe/issues", "",
        "Written source offer: we offer anyone possessing the covered object code",
        "a copy of its Corresponding Source at no charge by network download, or on",
        "a durable physical medium for no more than the reasonable cost of conveying it.",
        "This offer is valid for at least three years after our last distribution",
        "and for as long as we offer support or spare parts for a covered product.",
        "It is OUR obligation to keep the mirrored source available as long as we",
        "distribute the binary, and to honor the source offer for its full term.",
        "Source archives are on this ISO in SOURCE/ and offered with equivalent",
        "access at no extra charge in the SAME bucket/prefix as the ISO, at the",
        "content-addressed keys listed below. Upstream links are supplemental.",
        "Third-party GPL/LGPL app binaries are copied byte-for-byte from upstream; no patches.",
        "See LICENSES/index.json for original paths, archive pins and text hashes.", ""]
    for p in sorted(packages, key=lambda x: x["id"]):
        pid, c = p["id"], p["compliance"]
        original = {f["name"]: f for f in p["files"]}
        cached = {}
        for owner, pin, relative in inputs([p]):
            target = (f"SOURCE/{pid}/{pin['name']}" if pin in c["sources"]
                      else f"LICENSES/{pid}/{pin['sha256']}")
            cached[pin["name"]] = checked(owner, pin, relative, target)
        package_total = 0
        def add(name, body, origin, parent_pin, covered, pinned_text=False):
            nonlocal package_total
            if not 0 < len(body) <= MAX_TEXT: raise ValueError(f"package {pid}: invalid licence text size")
            package_total += len(body)
            if package_total > MAX_TOTAL: raise ValueError(f"package {pid}: licence texts exceed limit")
            # Archive member paths never become host/ISO paths. Keep originals in
            # the index and use full SHA identifiers (Joliet's 64-char maximum).
            digest = hashlib.sha256(body).hexdigest()
            if pinned_text and (digest != parent_pin["sha256"] or len(body) != parent_pin["size"]):
                raise ValueError(f"package {pid}: licence bytes changed after verification")
            target = f"LICENSES/{pid}/{digest}"
            files[target] = body; meta[target] = licenses(p)
            index.append(dict(path=target, original=name, origin=origin, input_sha256=parent_pin["sha256"],
                              sha256=digest, size=len(body), covers=covered, package=pid))
        binaries = {}
        for fl in p["files"]:
            path = checked(pid, fl, Path(pid) / fl["name"], f"pkg/{pid}/{fl['name']}")
            binaries[fl["name"]] = path
            if fl["name"].endswith(".whl") and c["distribution"] == "public-ok":
                import bundle_wheels
                bundle_wheels.verify(path, fl, c["components"][fl["name"]])
            meta[f"pkg/{pid}/{fl['name']}"] = licenses(p, fl["name"])
        for fl in c["sources"]:
            path = cached[fl["name"]]; target = f"SOURCE/{pid}/{fl['name']}"
            files[target] = path; meta[target] = licenses(p)
            sources.append(dict(package=pid, path=target, key=source_key(fl, "public" if c["distribution"] == "public-ok" else "private"), **fl))
        for name, rule in sorted(c["license_rules"].items()):
            kind = rule["kind"]; fl = original[name]; count = 0
            if kind == "zip":
                for member, body in legal_members(binaries[name], "zip"):
                    add(member, body, fl["url"], fl, name); count += 1
            elif kind == "source":
                src = next(s for s in c["sources"] if s["name"] == rule["input"])
                for member, body in legal_members(cached[src["name"]], "tar"):
                    add(member, body, src["url"], src, name); count += 1
            elif kind == "file":
                lp = original[rule["file"]]
                add(lp["name"], read_text(binaries[lp["name"]]), lp["url"], lp, name, pinned_text=True); count = 1
            # Always retain an explicitly pinned upstream text as well as any
            # bundled notices: a dependency's MIT notice cannot replace the
            # main program's GPL/Apache licence merely by being present.
            if kind == "upstream" or rule.get("fallback"):
                lp = next(s for s in c["license_inputs"] if s["name"] == rule.get("input", rule.get("fallback")))
                add(lp["name"], read_text(cached[lp["name"]]), lp["url"], lp, name, pinned_text=True); count += 1
            if not count: raise ValueError(f"package {pid}/{name}: no licence text found; pin the as-shipped upstream text")
        notices.extend([f"Package: {pid}", f"Version: {p['version']}", f"Licence: {c['license_id']}",
                        f"Upstream: {c['upstream']}", f"Distribution: {c['distribution']}"])
        notices.extend("Condition: " + s for s in c["conditions"])
        for name, component in sorted(c.get("components", {}).items()):
            notices.append(f"Component: {name} | {component['version']} | {component['license_id']} | {component['upstream']}")
        for src in c["sources"]:
            notices.extend([f"SOURCE exact version: {src['version']}", "SOURCE upstream: " + src["url"],
                "SOURCE sha256: " + src["sha256"], f"SOURCE on ISO: SOURCE/{pid}/{src['name']}",
                "SOURCE mirror next to ISO: " + source_key(src, "public" if c["distribution"] == "public-ok" else "private")])
        notices.append("")
    files["LICENSES/index.json"] = B.canonical(sorted(index, key=lambda x: (x["package"], x["covers"], x["original"])))
    files["THIRD_PARTY_NOTICES.txt"] = ("\n".join(notices) + "\n").encode("utf-8")
    aggregate = [entry for p in sorted(packages, key=lambda x: x["id"]) for entry in licenses(p)]
    for name in ("LICENSES/index.json", "THIRD_PARTY_NOTICES.txt"): meta[name] = aggregate
    return files, meta, sources
