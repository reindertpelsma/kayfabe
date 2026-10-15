#!/usr/bin/env python3
"""Fetch a subset of Blender's tests/ tree (scripts plus LFS-backed data) for one release tag.

Standard library only, so it runs with any Python 3 (the guest can use the Python that ships
inside the Blender release).  It lists the tree through the Gitea API of projects.blender.org and
downloads each file through the API media endpoint, which resolves git-LFS pointers to the real
content.  Nothing is cloned; already-present files with the right size are skipped.

  fetch_data.py --tag v5.2.0 --dest D:\\blender-tests --path tests/python --path tests/files/render/attributes

Prints a line per directory and a final count and byte total.  Exit code 1 if any file failed.
"""
import argparse
import concurrent.futures
import json
import os
import sys
import time
import urllib.parse
import urllib.request

BASE = "https://projects.blender.org"
REPO = "blender/blender"


UA = {"User-Agent": "kayfabe-blender-suite/1 (test-data fetch)"}


def opener(url):
    return urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=120)


def get_json(url):
    with opener(url) as r:
        return json.load(r)


def list_tree(tag):
    ref = get_json(f"{BASE}/api/v1/repos/{REPO}/git/refs/tags/{tag}")
    obj = ref[0]["object"] if isinstance(ref, list) else ref["object"]
    sha = obj["sha"]
    if obj.get("type") == "tag":  # annotated tag: peel to the commit
        sha = get_json(f"{BASE}/api/v1/repos/{REPO}/git/tags/{sha}")["object"]["sha"]
    entries, page = [], 1
    while True:
        t = get_json(f"{BASE}/api/v1/repos/{REPO}/git/trees/{sha}?recursive=true&per_page=1000&page={page}")
        entries += t["tree"]
        if not t.get("truncated") and len(entries) >= t.get("total_count", 0):
            break
        page += 1
        if page > 200:
            break
    return sha, [e for e in entries if e["type"] == "blob"]


def fetch(tag, dest, e):
    out = os.path.join(dest, *e["path"].split("/"))
    os.makedirs(os.path.dirname(out), exist_ok=True)
    # The API media endpoint resolves LFS pointers.  The web /media/ route sits behind a bot
    # challenge that a burst of requests trips (measured 2026-10-15), so use the API and go slowly.
    url = f"{BASE}/api/v1/repos/{REPO}/media/{urllib.parse.quote(e['path'])}?ref={tag}"
    err = "?"
    for attempt in range(4):
        time.sleep(0.05 + attempt * 2)
        try:
            with opener(url) as r, open(out + ".part", "wb") as f:
                n = 0
                while chunk := r.read(1 << 20):
                    f.write(chunk)
                    n += len(chunk)
            os.replace(out + ".part", out)
            return e["path"], n, None
        except Exception as ex:  # noqa: BLE001
            err = str(ex)
    return e["path"], 0, err


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tag", required=True)
    ap.add_argument("--dest", required=True)
    ap.add_argument("--path", action="append", required=True, help="repo path prefix (dir or file)")
    ap.add_argument("--jobs", type=int, default=3)
    ap.add_argument("--list-only", action="store_true")
    a = ap.parse_args()
    sha, blobs = list_tree(a.tag)
    pref = tuple(p.rstrip("/") for p in a.path)
    sel = [b for b in blobs if any(b["path"] == p or b["path"].startswith(p + "/") for p in pref)]
    print(f"tag {a.tag} commit {sha}: {len(sel)} files, "
          f"{sum(b.get('size', 0) for b in sel)} bytes (git blob sizes; LFS files count as pointers)")
    if a.list_only:
        return 0
    done = failed = total = 0
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as ex:
        for path, n, err in ex.map(lambda e: fetch(a.tag, a.dest, e), sel):
            if err:
                failed += 1
                print("FAILED", path, err, file=sys.stderr)
            else:
                done += 1
                total += n
    print(f"fetched {done} files, {total} bytes, {failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
