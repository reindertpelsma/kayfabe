#!/usr/bin/env python3
"""Reproduce V2's text inventories at the pinned source revision. No hardware.

This is a candidate finder, not a call-graph or Rust type checker. It reads
tracked text only, never a binary or a third-party checkout. Findings require
the source readings in the audit. Output contains locations rather than source
excerpts, so a provenance finding does not republish the offending material.
"""
from pathlib import Path
from collections import Counter
import json
import re
import subprocess

SUBJECT = "e64b8e5f6d0f64c41c9c4d40775e74be08de7164"
ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parent


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def write_json(name, value):
    (OUT / name).write_text(json.dumps(value, indent=2) + "\n")


def locations(name, paths, pattern):
    rx = re.compile(pattern)
    rows = []
    for path in paths:
        for line, text in enumerate((ROOT / path).read_text(errors="replace").splitlines(), 1):
            if rx.search(text):
                rows.append((path, line))
    (OUT / name).write_text("path\tline\n" + "".join(f"{p}\t{n}\n" for p, n in rows))
    return len(rows)


def main():
    # Refuse to inventory a source tree changed from the named subject. V2 adds
    # docs/evidence only. Checking exit status, not whether diff output is empty.
    scopes = ["crates", "qemu", "cuda", "tools", "scripts", "firmware", "Cargo.toml"]
    subprocess.run(["git", "diff", "--exit-code", SUBJECT, "--", *scopes],
                   cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    tracked = git("ls-tree", "-r", "--name-only", SUBJECT).splitlines()
    files = set(tracked)
    product = [p for p in tracked if
               (p.startswith("crates/kf-") and "/src/" in p and p.endswith(".rs")) or
               (p.startswith("qemu/hw/misc/kf3/") and p.endswith((".c", ".h")))]
    blocking = (r"\.lock\s*\(|\.join\s*\(|(?:thread::|std::thread::)?sleep\s*\("
                r"|\.wait(?:_timeout)?\s*\(|read_exact\s*\(|write_all\s*\("
                r"|epoll_wait|poll_wait|ctx_synchronize|stream_synchronize"
                r"|qemu_mutex_lock|bql_lock|cu(?:Ctx|Stream|Event)Synchronize|sem_wait|futex"
                r"|(?:recv|send|connect|accept|recv_timeout|send_timeout|ioctl|mmap|munmap|openat|open|read|write|flush)\s*\(")
    # Known positives BEFORE a zero could be accepted. They are strings, not
    # mutations of the product or extra worktrees.
    assert re.search(blocking, "mutex.lock()")
    assert re.search(blocking, "std::thread::sleep(d)")
    assert re.search(blocking, "cuda.ctx_synchronize()")
    assert not re.search(blocking, "atomic.load(Relaxed)")
    counts = {"blocking_candidate_locations": locations("blocking-sites.tsv", product, blocking)}
    entrypoints = []
    for path, kind, pattern in (
        ("crates/kf-qemu/src/ffi_unsafe.rs", "Rust FFI", r'pub (?:unsafe )?extern "C" fn (kf3_\w+)'),
        ("qemu/hw/misc/kf3/kf3.c", "C function candidate", r'^static (?:void|int|bool|uint64_t) (kf3_\w+)\s*\('),
    ):
        for line, text in enumerate((ROOT / path).read_text().splitlines(), 1):
            m = re.search(pattern, text)
            if m:
                entrypoints.append((kind, path, line, m[1]))
    (OUT / "qemu-entry-candidates.tsv").write_text("kind\tpath\tline\tname\n" +
        "".join(f"{k}\t{p}\t{n}\t{name}\n" for k, p, n, name in entrypoints))
    counts["qemu_rust_ffi_entries"] = sum(r[0] == "Rust FFI" for r in entrypoints)
    counts["qemu_c_function_candidates"] = sum(r[0] == "C function candidate" for r in entrypoints)
    counts["environment_read_locations"] = locations(
        "environment-read-sites.tsv", product,
        r"(?:env::(?:var|var_os|vars|vars_os)|getenv)\s*\(")
    counts["literal_die_driver_locations"] = locations(
        "compatibility-selectors.tsv", product,
        r"(?i)\b(?:tu10[246]|tu11[67]|ga10[023467]|ad10[23467]|gh100|gb10[02]|"
        r"gb11[02]|gb20[23567]|580\.88|595\.91\.07|535\.\d|545\.\d|550\.\d|"
        r"555\.\d|560\.\d|565\.\d|570\.\d|575\.\d|580\.\d|590\.\d|595\.\d|610\.\d)\b")
    counts["symbolic_selector_locations"] = locations(
        "symbolic-selectors.tsv", product,
        r"Family::|DieGroup::|DriverVersion|HostDriverVersion|arch::|"
        r"\bimplementation\b|\bpci_device\b|\bdevice_id\b|\.ip_version\b|\bfor_version\b")
    # Source-tag proximity is only triage. A constant may be generated, inherit
    # a module's provenance, or have its Why on the following definition. Do not
    # turn either a nearby citation or its absence into a policy verdict.
    definition = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+([A-Z][A-Z0-9_]*)\s*:")
    source_tag = re.compile(r"(?i)ogkm(?:-| )|gvisor|nouveau|Why::|Provenance::|measured:|\[measured|\bsource:|\binferred\b|\badvertised\b")
    constants = []
    for path in product:
        lines = (ROOT / path).read_text(errors="replace").splitlines()
        for index, text in enumerate(lines):
            m = definition.search(text)
            if not m:
                continue
            tags = [str(j + 1) for j in range(max(0, index - 12), min(len(lines), index + 8))
                    if source_tag.search(lines[j])]
            constants.append((path, index + 1, m[1], ",".join(tags) or "NONE_NEARBY"))
    (OUT / "constant-source-candidates.tsv").write_text("path\tline\tname\tnearby_tag_lines\n" +
        "".join(f"{p}\t{n}\t{k}\t{tags}\n" for p, n, k, tags in constants))
    counts["constant_definition_candidates"] = len(constants)
    counts["constant_candidates_without_nearby_tag"] = sum(r[3] == "NONE_NEARBY" for r in constants)
    text_scope = [p for p in tracked if not p.startswith(("archive/", "traces/")) and
                  Path(p).suffix in (".rs", ".py", ".md", ".sh", ".toml", ".c", ".h", ".yml", ".yaml", ".txt")]
    counts["provenance_marker_locations"] = locations(
        "provenance-marker-sites.tsv", text_scope,
        r"(?i)(decompil|ghidra|ida pro|nvlddmkm\.sys|export_row_rva|re-spec:|"
        r"source.{0,15}blob|from.{0,15}(?:retail|closed).{0,20}(?:driver|binar))")
    env_keys = []
    for path in product:
        for line, text in enumerate((ROOT / path).read_text(errors="replace").splitlines(), 1):
            for key in re.findall(r'"((?:KF3_|KF_|KAYFABE_)[A-Z0-9_]+)"', text):
                env_keys.append((key, path, line))
    (OUT / "environment-key-sites.tsv").write_text("key\tpath\tline\n" +
        "".join(f"{k}\t{p}\t{n}\n" for k, p, n in env_keys))
    counts["literal_environment_keys"] = len({k for k, _, _ in env_keys})
    missing = []
    design_docs = [p for p in tracked if p.startswith("docs/design/") and p.endswith(".md")]
    for path in design_docs:
        lines = (ROOT / path).read_text(errors="replace").splitlines()
        head = "\n".join(lines[:30])
        status = bool(re.search(r"\bSTATUS\b", head))
        date = bool(re.search(r"202[0-9]-[01][0-9]-[0-3][0-9]", head))
        if not status or not date:
            positions = [n for n, line in enumerate(lines, 1) if "STATUS" in line]
            missing.append({"path": path, "status_in_first_30": status,
                            "date_in_first_30": date, "status_lines": positions})
    write_json("doc-status-candidates.json", missing)
    counts["design_docs"] = len(design_docs)
    counts["doc_header_candidates"] = len(missing)
    claims = []
    for path in ("README.md", "docs/STATUS_DETAIL.md"):
        for line, text in enumerate((ROOT / path).read_text().splitlines(), 1):
            for rev in re.findall(r"`([0-9a-f]{7,40})`", text):
                r = subprocess.run(["git", "rev-parse", "--verify", rev + "^{commit}"],
                                   cwd=ROOT, text=True, capture_output=True)
                ok = r.returncode == 0
                ancestor = ok and subprocess.run(["git", "merge-base", "--is-ancestor",
                            r.stdout.strip(), SUBJECT], cwd=ROOT).returncode == 0
                claims.append({"doc": path, "line": line, "kind": "revision", "value": rev,
                               "exists_in_local_git": ok, "ancestor_of_subject": ancestor})
            for value in re.findall(r"(?:traces|docs/design|design)/[A-Za-z0-9_./{}*,+()<>-]+", text):
                value = value.rstrip(".,;)")
                normalized = "docs/" + value if value.startswith("design/") else value
                present = normalized in files or any(f.startswith(normalized.rstrip("/") + "/") for f in files)
                state = "PRESENT" if present else "DYNAMIC" if any(c in normalized for c in "{}*<>") else "MISSING"
                claims.append({"doc": path, "line": line, "kind": "path", "value": value,
                               "normalized": normalized, "status": state})
            # Links to an ordinary relative file, including HTML image sources.
            links = re.findall(r"\]\(([^)]+)\)", text) + re.findall(r'src="([^"]+)"', text)
            for value in links:
                value = value.split("#", 1)[0]
                if not value or "://" in value or value.startswith("mailto:"):
                    continue
                parts = []
                for part in (Path(path).parent / value).parts:
                    if part == "..":
                        if parts:
                            parts.pop()
                    elif part != ".":
                        parts.append(part)
                normalized = "/".join(parts)
                present = normalized in files or any(f.startswith(normalized.rstrip("/") + "/") for f in files)
                claims.append({"doc": path, "line": line, "kind": "link", "value": value,
                               "normalized": normalized, "status": "PRESENT" if present else "MISSING"})
    # The location check has a known missing-path control before evaluating the real set.
    assert "traces/v2-intentionally-nonexistent-control/" not in files
    write_json("status-claims.json", claims)
    counts["claims_by_kind_state"] = dict(Counter(
        f"{c['kind']}:{c.get('status', 'exists' if c.get('exists_in_local_git') else 'missing')}" for c in claims))
    counts["consttab_paths"] = [p for p in tracked if "consttab" in p]
    write_json("inventory-summary.json", {"subject": SUBJECT, "date": "2026-10-21", "counts": counts,
        "limits": "Lexical candidates include comments, test code, and string joins; no complete transitive call graph, AST proof, provenance attestation, or runtime certification."})
    print(json.dumps(counts, indent=2))


if __name__ == "__main__":
    main()
