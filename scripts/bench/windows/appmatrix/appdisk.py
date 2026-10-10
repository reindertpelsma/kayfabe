#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""appdisk.py -- build and verify the Windows app matrix's read-only application disk.

The image (kfapps.iso, volume label KFAPPS) holds portable installers/binaries of every package in
manifest.json so that a Windows guest needs NO internet at run time. It is attached to the VM
read-only. Binaries are never committed: manifest.json (URLs, sha256, sizes, licences) is.

  appdisk.py check   [--manifest M] [--apps A]     schema + cross-reference check (no network)
  appdisk.py fetch   [--manifest M] --dl DIR [--pipdl DIR] [--only ID,ID] [--max-tier N] [--no-write]
                                                   HEAD every URL (records http status + length),
                                                   download, sha256, fill/verify the manifest
  appdisk.py verify  [--manifest M] --dl DIR       re-hash everything in DIR against the manifest
  appdisk.py build   [--manifest M] --dl DIR --tools DIR --out DIR [--only ID,..] [--max-tier N]
                                                   assemble the staging tree and make the ISO;
                                                   writes OUT/manifest.json with the sha256 of
                                                   every file (downloads, tools, image)
  appdisk.py verify-image --iso ISO [--whole] [--mount]      read every file back out of the image and compare with manifest["files"]
  appdisk.py layout  [--manifest M]                print the ISO tree that `build` would make

Exit codes: 0 ok, 1 a check failed (named), 2 usage.
"""
import argparse
import datetime
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_MANIFEST = os.path.join(HERE, "manifest.json")
DEFAULT_APPS = os.path.join(HERE, "apps.json")
UA = "Mozilla/5.0 (kayfabe-appmatrix)"
STAGE_MODES = {"unzip", "python_embed", "wheels", "copy", "7z", "msi_admin", "installer"}
# guest-side files that ride on the ISO (a copy; the driver also pushes the repo's own copy by QGA)
GUEST_DIRS = ["guest", "tools/src"]


def utcnow():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def load(path):
    with open(path) as f:
        return json.load(f)


def save(path, obj):
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(obj, f, indent=1)
        f.write("\n")
    os.replace(tmp, path)


def sha256_file(path, bufsize=1 << 22):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            b = f.read(bufsize)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def pkg_by_id(man):
    return {p["id"]: p for p in man["packages"]}


# ------------------------------------------------------------------------------------- check
def check_manifest(man, apps=None):
    """Return a list of problems (empty = OK). Pure function: unit-tested."""
    bad = []
    if man.get("schema") != 1:
        bad.append("manifest schema != 1")
    ids = [p.get("id") for p in man.get("packages", [])]
    if len(ids) != len(set(ids)):
        bad.append("duplicate package ids")
    seen_names = set()
    for p in man.get("packages", []):
        pid = p.get("id", "?")
        for k in ("title", "version", "license", "redistribution", "stage", "files"):
            if k not in p:
                bad.append(f"package {pid}: missing {k}")
        st = p.get("stage", {})
        if st.get("mode") not in STAGE_MODES:
            bad.append(f"package {pid}: unknown stage mode {st.get('mode')!r}")
        if st.get("mode") == "installer" and not st.get("run"):
            bad.append(f"package {pid}: installer without a run command")
        for n in p.get("needs", []):
            if n not in ids:
                bad.append(f"package {pid}: needs unknown package {n}")
        if not p.get("files"):
            bad.append(f"package {pid}: no files")
        for fl in p.get("files", []):
            nm = fl.get("name", "")
            if not nm or "/" in nm or "\\" in nm:
                bad.append(f"package {pid}: bad file name {nm!r}")
            if not fl.get("url", "").startswith(("https://", "file://")):
                bad.append(f"package {pid}/{nm}: url must be https:// (or file:// in tests)")
            sh = fl.get("sha256")
            if sh is not None and (len(sh) != 64 or any(c not in "0123456789abcdef" for c in sh)):
                bad.append(f"package {pid}/{nm}: sha256 is not 64 hex digits")
            if (pid, nm) in seen_names:
                bad.append(f"package {pid}: duplicate file {nm}")
            seen_names.add((pid, nm))
    for d in man.get("dropped", []):
        if not d.get("reason"):
            bad.append(f"dropped {d.get('id')}: no reason")
    if apps is not None:
        for a in apps.get("apps", []):
            for pk in a.get("pkgs", []):
                if pk not in ids and pk != "tools" and pk != "guest":
                    bad.append(f"app {a.get('id')}: unknown package {pk}")
    return bad


def closure(man, want):
    """Packages needed for the ids in `want`, dependencies first."""
    byid = pkg_by_id(man)
    out, seen = [], set()

    def visit(i):
        if i in seen or i not in byid:
            return
        seen.add(i)
        for n in byid[i].get("needs", []):
            visit(n)
        out.append(i)

    for i in want:
        visit(i)
    return out


# ------------------------------------------------------------------------------------- fetch
def curl_head(url):
    """(http status, content-length or None) after redirects, or (0, None)."""
    try:
        r = subprocess.run(["curl", "-sIL", "-m", "40", "-A", UA, url], capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired):
        return 0, None
    status, length = 0, None
    for line in r.stdout.splitlines():
        low = line.lower().strip()
        if low.startswith("http/"):
            parts = low.split()
            status = int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else 0
            length = None
        elif low.startswith("content-length:"):
            try:
                length = int(low.split(":", 1)[1])
            except ValueError:
                pass
    if url.startswith("file://"):
        p = url[7:]
        return (200, os.path.getsize(p)) if os.path.exists(p) else (404, None)
    return status, length


def curl_get(url, dest):
    part = dest + ".part"
    r = subprocess.run(["curl", "-fL", "-C", "-", "--retry", "3", "-A", UA, "-o", part, url])
    if r.returncode != 0:
        return False
    os.replace(part, dest)
    return True


def cmd_fetch(a):
    man = load(a.manifest)
    bad = check_manifest(man)
    if bad:
        print("manifest problems:\n  " + "\n  ".join(bad))
        return 1
    only = set(a.only.split(",")) if a.only else None
    failures = 0
    for p in man["packages"]:
        if only and p["id"] not in only:
            continue
        if p.get("tier", 1) > a.max_tier:
            print(f"SKIP {p['id']}: tier {p['tier']} > {a.max_tier}")
            continue
        d = os.path.join(a.dl, p["id"])
        os.makedirs(d, exist_ok=True)
        for fl in p["files"]:
            dest = os.path.join(d, fl["name"])
            st, ln = curl_head(fl["url"])
            fl["http"] = st
            fl["head_length"] = ln
            fl["verified_utc"] = utcnow()
            ok_head = st == 200
            print(f"HEAD {p['id']}/{fl['name']}: http={st} length={ln}")
            if not os.path.exists(dest):
                seed = os.path.join(a.pipdl, fl["name"]) if a.pipdl else None
                if seed and os.path.exists(seed):
                    try:
                        os.link(seed, dest)
                    except OSError:
                        shutil.copy2(seed, dest)
                    print(f"SEED {fl['name']} from {a.pipdl}")
                elif ok_head:
                    print(f"GET  {fl['url']}")
                    if not curl_get(fl["url"], dest):
                        print(f"FAIL download {p['id']}/{fl['name']}")
                        failures += 1
                        continue
                else:
                    print(f"FAIL {p['id']}/{fl['name']}: url does not resolve (http {st})")
                    failures += 1
                    continue
            size = os.path.getsize(dest)
            sha = sha256_file(dest)
            if ln is not None and ln != size and ok_head:
                print(f"WARN {fl['name']}: content-length {ln} != downloaded {size}")
            if fl.get("sha256") and fl["sha256"] != sha:
                print(f"MISMATCH {p['id']}/{fl['name']}: manifest {fl['sha256']} actual {sha}")
                failures += 1
                continue
            fl["sha256"], fl["size"] = sha, size
            print(f"OK   {p['id']}/{fl['name']} size={size} sha256={sha}")
            if not a.no_write:
                save(a.manifest, man)
    if not a.no_write:
        save(a.manifest, man)
    print(f"FETCH_DONE failures={failures}")
    return 1 if failures else 0


def cmd_verify(a):
    man = load(a.manifest)
    n = bad = 0
    for p in man["packages"]:
        for fl in p["files"]:
            path = os.path.join(a.dl, p["id"], fl["name"])
            if not os.path.exists(path):
                if p.get("tier", 1) <= a.max_tier:
                    print(f"MISSING {p['id']}/{fl['name']}")
                    bad += 1
                continue
            sha = sha256_file(path)
            n += 1
            if not fl.get("sha256"):
                print(f"UNPINNED {p['id']}/{fl['name']} actual {sha}")
                bad += 1
            elif sha != fl["sha256"]:
                print(f"MISMATCH {p['id']}/{fl['name']}")
                bad += 1
            else:
                print(f"OK {p['id']}/{fl['name']}")
    print(f"VERIFY_DONE files={n} bad={bad}")
    return 1 if bad else 0


# ------------------------------------------------------------------------------------- build
def plan_layout(man, only=None, max_tier=9, tools=None, guest_root=HERE):
    """[(iso_path, source_path)] for the ISO tree; pure (does not touch the disk except to list `tools`)."""
    want = only or [p["id"] for p in man["packages"] if p.get("tier", 1) <= max_tier]
    ids = closure(man, want)
    byid = pkg_by_id(man)
    out = []
    for i in ids:
        if byid[i].get("image_form") == "tree":
            out.append((f"{byid[i]['tree_dest']}/", ("tree", i)))      # extracted into the image (run in place from the disk)
            continue
        for fl in byid[i]["files"]:
            out.append((f"pkg/{i}/{fl['name']}", ("dl", i, fl["name"])))
    if tools:
        for fn in sorted(os.listdir(tools)):
            if fn.endswith(".exe"):
                out.append((f"tools/{fn}", ("tools", fn)))
    for gd in ("guest",):
        base = os.path.join(guest_root, gd)
        for root, _, files in os.walk(base):
            for fn in sorted(files):
                full = os.path.join(root, fn)
                rel = os.path.relpath(full, guest_root)
                out.append((rel.replace(os.sep, "/"), ("repo", rel)))
    out.append(("apps.json", ("repo", "apps.json")))
    return out


PTH = "python312.zip\r\n.\r\nLib\\site-packages\r\nimport site\r\n"


def extract_tree_package(pkg, dl, tree):
    """Extract a pre-extracted-tree package (python_embed, pywheels) into tree/<tree_dest>; returns {isopath: record}
    for the manifest. python_embed's python312._pth is rewritten so the interpreter finds Lib\\site-packages
    (the wheels are unpacked there) and `site` runs; nothing else is modified."""
    import zipfile
    rec = {}
    root = os.path.join(tree, pkg["tree_dest"])
    os.makedirs(root, exist_ok=True)
    for fl in pkg["files"]:
        src = os.path.join(dl, pkg["id"], fl["name"])
        got = sha256_file(src)
        if fl["sha256"] != got:
            raise RuntimeError(f"{fl['name']}: manifest {fl['sha256']} actual {got}")
        with zipfile.ZipFile(src) as z:
            for info in z.infolist():
                target = os.path.realpath(os.path.join(root, info.filename))
                if not target.startswith(os.path.realpath(root) + os.sep) and target != os.path.realpath(root):
                    raise RuntimeError(f"unsafe path in {fl['name']}: {info.filename}")
            z.extractall(root)
        rec[f"{pkg['tree_dest']}/<{fl['name']}>"] = dict(sha256=got, size=os.path.getsize(src))
    if pkg["id"] == "python_embed":
        for fn in os.listdir(root):
            if fn.endswith("._pth"):
                with open(os.path.join(root, fn), "w", newline="") as f:
                    f.write(PTH)
    return rec


def stage_table(man):
    """The guest-side package table (C:\\kf\\stage.json): what kf_stage.ps1 needs per package."""
    t = {}
    for p in man["packages"]:
        st = dict(p["stage"])
        if p.get("image_form") == "tree":
            st = dict(mode="tree", dest=p["tree_dest"])
        t[p["id"]] = dict(mode=st["mode"], dest=st["dest"].replace("/", "\\"), strip_top=bool(st.get("strip_top")), run=st.get("run", []),
                          files=[f["name"] for f in p["files"]])
    return t


def make_iso(tree, out_iso, label="KFAPPS"):
    epoch = os.environ.get("SOURCE_DATE_EPOCH", "1760000000")
    env = dict(os.environ, SOURCE_DATE_EPOCH=epoch)
    flags = ["-iso-level", "3", "-J", "-joliet-long", "-R", "-V", label, "-follow-links", "-quiet", "-o", out_iso, tree]
    # genisoimage/mkisofs can add a UDF filesystem (Windows prefers it for big files); xorriso's mkisofs emulation cannot
    if shutil.which("genisoimage"):
        cmd = ["genisoimage", "-udf"] + flags
    elif shutil.which("mkisofs") and "genisoimage" not in os.path.realpath(shutil.which("mkisofs")):
        cmd = ["mkisofs", "-udf"] + flags
    elif shutil.which("xorriso"):
        cmd = ["xorriso", "-as", "mkisofs"] + flags
    else:
        raise RuntimeError("no genisoimage/mkisofs/xorriso")
    r = subprocess.run(cmd, env=env, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"{cmd[0]} failed rc={r.returncode}: {r.stderr[-400:]}")
    return cmd[0]


def cmd_build(a):
    man = load(a.manifest)
    bad = check_manifest(man)
    if bad:
        print("manifest problems:\n  " + "\n  ".join(bad))
        return 1
    only = a.only.split(",") if a.only else None
    plan = plan_layout(man, only, a.max_tier, a.tools)
    os.makedirs(a.out, exist_ok=True)
    tree = tempfile.mkdtemp(prefix="kfapps-tree-", dir=a.out)
    files_rec = {}
    try:
        byid = pkg_by_id(man)
        for isop, src in plan:
            dst = os.path.join(tree, isop)
            if src[0] == "tree":
                files_rec.update(extract_tree_package(byid[src[1]], a.dl, tree))
                continue
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            if src[0] == "dl":
                s = os.path.join(a.dl, src[1], src[2])
                if not os.path.exists(s):
                    print(f"MISSING {s}")
                    return 1
                want = next(f["sha256"] for f in byid[src[1]]["files"] if f["name"] == src[2])
                got = sha256_file(s)
                if want is None or want != got:
                    print(f"HASH {isop}: manifest {want} actual {got}")
                    return 1
                os.symlink(os.path.abspath(s), dst)
                files_rec[isop] = dict(sha256=got, size=os.path.getsize(s))
            elif src[0] == "tools":
                s = os.path.join(a.tools, src[1])
                os.symlink(os.path.abspath(s), dst)
                files_rec[isop] = dict(sha256=sha256_file(s), size=os.path.getsize(s))
            else:
                s = os.path.join(HERE, src[1])
                shutil.copy2(s, dst)
                files_rec[isop] = dict(sha256=sha256_file(s), size=os.path.getsize(s))
        # the image's own identity: sha256 of the sorted file records, so the guest/driver can check it
        ident = hashlib.sha256(json.dumps(files_rec, sort_keys=True).encode()).hexdigest()
        with open(os.path.join(tree, "KFAPPS.ID"), "w") as f:
            f.write(ident + "\n")
        iso = os.path.join(a.out, man["image"]["file"])
        tool = make_iso(tree, iso, man["image"]["label"])
    finally:
        shutil.rmtree(tree, ignore_errors=True)
    man["image"].update(sha256=sha256_file(iso), size=os.path.getsize(iso), built_utc=utcnow(), identity=ident, built_with=tool)
    man["files"] = files_rec
    outm = os.path.join(a.out, "manifest.json")
    save(outm, man)
    print(f"BUILD_DONE image={iso} size={man['image']['size']} sha256={man['image']['sha256']} identity={ident} files={len(files_rec)} manifest={outm}")
    return 0


def cmd_verify_image(a):
    """Read every file back out of the ISO (isoinfo -J -x) and compare with manifest["files"]; also the image's own sha256."""
    man = load(a.manifest)
    mnt = None
    if a.mount:                                       # isoinfo cannot read files over 2 GiB (torch's wheel is 2.9 GB): loop-mount instead
        mnt = tempfile.mkdtemp(prefix="kfiso-")
        r = subprocess.run(["mount", "-o", "loop,ro", a.iso, mnt], capture_output=True, text=True)
        if r.returncode != 0:
            print("mount failed:", r.stderr.strip())
            return 2
    elif not shutil.which("isoinfo"):
        print("isoinfo missing (genisoimage package)")
        return 2
    try:
        return _verify_image(a, man, mnt)
    finally:
        if mnt:
            subprocess.run(["umount", mnt], capture_output=True)
            os.rmdir(mnt)


def _verify_image(a, man, mnt):
    bad = n = 0
    got = sha256_file(a.iso) if a.whole else None
    if got is not None:
        ok = got == man["image"]["sha256"]
        print(("OK" if ok else "MISMATCH") + f" image sha256 {got}")
        bad += 0 if ok else 1
    for path, rec in sorted(man.get("files", {}).items()):
        if "<" in path:
            continue                                  # entries of pre-extracted tree packages are recorded by archive, not by path
        h = hashlib.sha256()
        size = 0
        if mnt:
            with open(os.path.join(mnt, path), "rb") as fh:
                for chunk in iter(lambda: fh.read(1 << 22), b""):
                    h.update(chunk)
                    size += len(chunk)
        else:
            p = subprocess.Popen(["isoinfo", "-J", "-i", a.iso, "-x", "/" + path], stdout=subprocess.PIPE)
            for chunk in iter(lambda: p.stdout.read(1 << 22), b""):
                h.update(chunk)
                size += len(chunk)
            p.wait()
        n += 1
        if h.hexdigest() != rec["sha256"] or size != rec["size"]:
            print(f"MISMATCH {path}: image has {h.hexdigest()} ({size} B), manifest {rec['sha256']} ({rec['size']} B)")
            bad += 1
    ident = hashlib.sha256(json.dumps(man.get("files", {}), sort_keys=True).encode()).hexdigest()
    ok = ident == man["image"].get("identity")
    print(("OK" if ok else "MISMATCH") + f" image identity {ident}")
    bad += 0 if ok else 1
    print(f"VERIFY_IMAGE_DONE files={n} bad={bad}")
    return 1 if bad else 0


def cmd_layout(a):
    man = load(a.manifest)
    for isop, src in plan_layout(man, None, a.max_tier, None):
        print(isop, src)
    return 0


def cmd_check(a):
    man = load(a.manifest)
    apps = load(a.apps) if os.path.exists(a.apps) else None
    bad = check_manifest(man, apps)
    unpinned = [f"{p['id']}/{f['name']}" for p in man["packages"] for f in p["files"] if not f.get("sha256")]
    if bad:
        print("PROBLEMS:\n  " + "\n  ".join(bad))
        return 1
    print(f"CHECK_OK packages={len(man['packages'])} files={sum(len(p['files']) for p in man['packages'])} unpinned={len(unpinned)} dropped={len(man.get('dropped', []))}")
    if unpinned and a.require_pinned:
        print("UNPINNED:\n  " + "\n  ".join(unpinned))
        return 1
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    def common(p):
        p.add_argument("--manifest", default=DEFAULT_MANIFEST)
        p.add_argument("--max-tier", type=int, default=9)
    c = sub.add_parser("check"); common(c); c.add_argument("--apps", default=DEFAULT_APPS); c.add_argument("--require-pinned", action="store_true"); c.set_defaults(fn=cmd_check)
    f = sub.add_parser("fetch"); common(f); f.add_argument("--dl", required=True); f.add_argument("--pipdl"); f.add_argument("--only"); f.add_argument("--no-write", action="store_true"); f.set_defaults(fn=cmd_fetch)
    v = sub.add_parser("verify"); common(v); v.add_argument("--dl", required=True); v.set_defaults(fn=cmd_verify)
    b = sub.add_parser("build"); common(b); b.add_argument("--dl", required=True); b.add_argument("--tools"); b.add_argument("--out", required=True); b.add_argument("--only"); b.set_defaults(fn=cmd_build)
    vi = sub.add_parser("verify-image"); common(vi); vi.add_argument("--iso", required=True); vi.add_argument("--whole", action="store_true", help="also hash the whole image file"); vi.add_argument("--mount", action="store_true", help="loop-mount (root) instead of isoinfo; needed for files > 2 GiB"); vi.set_defaults(fn=cmd_verify_image)
    l = sub.add_parser("layout"); common(l); l.set_defaults(fn=cmd_layout)
    a = ap.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
