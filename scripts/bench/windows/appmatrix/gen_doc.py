#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""gen_doc.py [--check] -- fill the generated tables of docs/design/V3_WINDOWS_APP_MATRIX.md from manifest.json and
apps.json, so the inventory in the document can never drift from the files the lane runs. The document marks the
spans with <!-- BEGIN GENERATED name --> ... <!-- END GENERATED name -->; everything else is hand-written.
Tables: packages, linuxmap, apps (one per category, in the order below), dropped, counts."""
import collections
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", "..", ".."))
DOC = os.path.join(REPO, "docs", "design", "V3_WINDOWS_APP_MATRIX.md")
CAT_ORDER = [("probe", "System and driver probes"), ("cuda-demo", "CUDA Toolkit demo suite (prebuilt by NVIDIA)"),
             ("cuda-ladder", "CUDA driver-API ladder (kayfabe's own, cross-built with mingw)"), ("stream-probe", "CUDA stream-shape probes (the Linux stream_probe rows)"), ("cuda-equiv", "CUDA samples counterparts (PyTorch / CuPy / NVRTC)"),
             ("torch", "PyTorch and CuPy (the Linux rows, unmodified scripts)"), ("stress", "Stress"), ("llm", "LLM (llama.cpp CUDA and Vulkan)"),
             ("vulkan", "Vulkan"), ("opengl", "OpenGL"), ("d3d", "Direct3D 11/12 (and DirectML)"), ("video", "Video (NVENC, NVDEC, D3D11VA/DXVA2, Edge playback)"),
             ("browser", "Browser (Edge WebGL / WebGPU)"), ("render", "Render"), ("crypto", "Crypto"), ("opencl", "OpenCL")]


sys.path.insert(0, HERE)
import gen_apps  # noqa: E402

CLASSES = {"C-small": gen_apps.W_COMPUTE_SMALL[1], "C-big": gen_apps.W_COMPUTE_BIG[1], "GFX": gen_apps.W_GFX[1], "VID": gen_apps.W_VIDEO[1], "LONG": gen_apps.W_LONG[1]}


def why_cell(a):
    for tag, text in CLASSES.items():
        if a["why"] == text:
            return f"**{a['difficulty']}** [{tag}]" + (f" {md(a['note'])}" if a.get("note") else "")
    return f"**{a['difficulty']}**: {md(a['why'])}" + (f" Note: {md(a['note'])}" if a.get("note") else "")


def md(s):
    return str(s).replace("|", "\\|").replace("\n", " ")


def packages_table(man):
    out = ["| package | version | size | sha256 | licence / redistribution | download URL |", "|---|---|---|---|---|---|"]
    for p in man["packages"]:
        for i, f in enumerate(p["files"]):
            out.append(f"| {p['id'] if i == 0 else ''}{' (tier ' + str(p.get('tier', 1)) + ')' if i == 0 and p.get('tier', 1) > 1 else ''} | {md(p['version']) if i == 0 else ''} | {f['size'] / 1e6:.1f} MB | `{f['sha256']}` | "
                       f"{md(p['license']) + ' — ' + md(p['redistribution']) if i == 0 else '(same)'} | {f['url']} |")
    return "\n".join(out)


def linuxmap_table(doc):
    txt = open(os.path.join(REPO, "scripts", "apps", "run_apps.sh")).read()
    block = txt.split("APPS=$(cat <<'EOF'\n", 1)[1].split("\nEOF\n", 1)[0]
    linux = [l.split("|", 1)[0] for l in block.splitlines() if l and "|" in l]
    out = ["| Linux row | Windows app(s) | note |", "|---|---|---|"]
    for l in linux:
        if l in doc["linux_map"]:
            out.append(f"| {l} | {', '.join(doc['linux_map'][l])} | |")
        else:
            out.append(f"| {l} | **none** | {md(doc['no_equivalent'][l])} |")
    return "\n".join(out)


def apps_tables(doc):
    by = collections.defaultdict(list)
    for a in doc["apps"]:
        by[a["category"]].append(a)
    out = []
    seen_titles = set()
    for cat, title in CAT_ORDER:
        if cat not in by:
            continue
        apps = by[cat] + (by["smi"] if cat == "probe" else [])
        out.append(f"\n**{title}** ({len(apps)})\n")
        out.append("| app | tier / session | command | success | GPU-use proof | runtime / timeout | difficulty: why |")
        out.append("|---|---|---|---|---|---|---|")
        for a in apps:
            out.append(f"| `{a['id']}` | {a['tier']} / {'desktop' if a['session'] == 'interactive' else 'service'} | `{md(a['cmd'])}` | `{md(a['rx'])}` | "
                       f"{md(', '.join(a['proof']) or '-')} | ~{a['expect_s']} s / {a['timeout_s']} s | {why_cell(a)} |")
    return "\n".join(out)


def dropped_table(man):
    out = ["| item | URL | size | why dropped |", "|---|---|---|---|"]
    for d in man["dropped"]:
        out.append(f"| {d['id']} | {d['url']} | {(str(round(d['size'] / 1e6)) + ' MB') if d.get('size') else '-'} | {md(d['reason'])} |")
    return "\n".join(out)


def classes_table():
    out = ["| class | reasoned meaning |", "|---|---|"]
    for tag, text in CLASSES.items():
        out.append(f"| {tag} | {md(text)} |")
    return "\n".join(out)


def counts(man, doc):
    c = collections.Counter(a["category"] for a in doc["apps"])
    tiers = collections.Counter(a["tier"] for a in doc["apps"])
    diff = collections.Counter(a["difficulty"] for a in doc["apps"])
    files = sum(len(p["files"]) for p in man["packages"])
    mapped = len(doc["linux_map"])
    return (f"{len(doc['apps'])} apps in {len(c)} categories ({', '.join(f'{k} {v}' for k, v in sorted(c.items()))}); tier 1: {tiers[1]}, tier 2: {tiers[2]}, tier 3: {tiers[3]}; "
            f"reasoned difficulty low {diff['low']} / medium {diff['medium']} / high {diff['high']}. {len(man['packages'])} packages, {files} files, all sha256-pinned and "
            f"url-verified; {len(man['dropped'])} items dropped. Linux rows: {mapped} mapped to at least one Windows app, {len(doc['no_equivalent'])} without an equivalent.")


def fill(text, man, doc):
    gens = {"packages": packages_table(man), "linuxmap": linuxmap_table(doc), "apps": apps_tables(doc), "dropped": dropped_table(man), "counts": counts(man, doc), "classes": classes_table()}
    for name, body in gens.items():
        pat = re.compile(rf"(<!-- BEGIN GENERATED {name} -->).*?(<!-- END GENERATED {name} -->)", re.S)
        if not pat.search(text):
            raise SystemExit(f"marker {name} missing in the document")
        text = pat.sub(lambda m: m.group(1) + "\n" + body + "\n" + m.group(2), text)
    return text


def main(argv):
    man = json.load(open(os.path.join(HERE, "manifest.json")))
    doc = json.load(open(os.path.join(HERE, "apps.json")))
    cur = open(DOC).read()
    new = fill(cur, man, doc)
    if "--check" in argv:
        if new != cur:
            print("docs/design/V3_WINDOWS_APP_MATRIX.md is out of date: run gen_doc.py")
            return 1
        print("document tables up to date")
        return 0
    open(DOC, "w").write(new)
    print("filled", DOC)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
