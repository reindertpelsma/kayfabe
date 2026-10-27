#!/usr/bin/env python3
"""Build twice; export only from Git-committed inputs and a clean final TeX log."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile


def git(root, *args):
    return subprocess.check_output(["git", *args], cwd=root, stderr=subprocess.DEVNULL)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def source_records(root, paths):
    records = []
    for path in sorted(set(Path(p).resolve() for p in paths)):
        name = path.relative_to(root.resolve()).as_posix()
        try:
            committed = git(root, "show", "HEAD:" + name)
            blob = git(root, "rev-parse", "HEAD:" + name).decode().strip()
        except subprocess.CalledProcessError as exc:
            raise ValueError(f"input must be committed: {name}") from exc
        if path.read_bytes() != committed:
            raise ValueError(f"input must match committed bytes: {name}")
        records.append({"path": name, "git_blob": blob, "sha256": digest(committed)})
    if not records:
        raise ValueError("no committed inputs recorded")
    return records


def check_log(log):
    patterns = (
        r"^!",
        r"Undefined control sequence",
        r"Missing character",
        r"undefined references",
        r"Reference .* undefined",
        r"Citation .* undefined",
        r"Label\(s\) may have changed",
    )
    for pattern in patterns:
        if re.search(pattern, log, re.MULTILINE | re.IGNORECASE):
            raise ValueError(f"TeX log rejected: {pattern}")


def verify(root, pdf, manifest):
    data = json.loads(manifest.read_text())
    if data["schema"] != 1:
        raise ValueError("unknown manifest schema")
    if digest(pdf.read_bytes()) != data["pdf_sha256"]:
        raise ValueError("PDF digest mismatch")
    records = source_records(root, [root / row["path"] for row in data["inputs"]])
    if records != data["inputs"]:
        raise ValueError("source manifest mismatch")
    for row in records:
        original = git(root, "show", data["source_commit"] + ":" + row["path"])
        if digest(original) != row["sha256"]:
            raise ValueError("source commit mismatch")
    return data


def build(root, directory, stem):
    main = directory / (stem + ".tex")
    source_records(root, [main])
    source_commit = git(root, "rev-parse", "HEAD").decode().strip()
    environment = dict(os.environ, TZ="UTC", FORCE_SOURCE_DATE="1")
    epoch = git(root, "show", "-s", "--format=%ct", "HEAD").decode().strip()
    environment["SOURCE_DATE_EPOCH"] = epoch
    with tempfile.TemporaryDirectory(prefix="kayfabe-paper-") as temporary:
        output = Path(temporary)
        for _ in range(2):
            result = subprocess.run(
                ["xelatex", "-halt-on-error", "-interaction=nonstopmode", "-recorder",
                 "-output-directory=" + temporary, main.name],
                cwd=directory, env=environment, stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT, text=True,
            )
            if result.returncode:
                raise ValueError("XeLaTeX failed:\n" + result.stdout[-5000:])
        check_log((output / (stem + ".log")).read_text(errors="replace"))
        inputs = []
        for line in (output / (stem + ".fls")).read_text().splitlines():
            if not line.startswith("INPUT "):
                continue
            path = Path(line[6:])
            path = (directory / path).resolve() if not path.is_absolute() else path.resolve()
            if path.is_relative_to(root) and not path.is_relative_to(output):
                inputs.append(path)
        records = source_records(root, inputs)
        if main.resolve() not in inputs:
            raise ValueError("recorder omitted main source")
        if git(root, "rev-parse", "HEAD").decode().strip() != source_commit:
            raise ValueError("HEAD changed during build")
        pdf_bytes = (output / (stem + ".pdf")).read_bytes()
        version = subprocess.check_output(["xelatex", "--version"], text=True).splitlines()[0]
        manifest = {
            "schema": 1, "source_commit": source_commit, "inputs": records,
            "pdf_sha256": digest(pdf_bytes), "engine": version, "passes": 2,
        }
        pdf = directory / (stem + ".pdf")
        receipt = directory / (stem + ".build.json")
        pdf.write_bytes(pdf_bytes)
        receipt.write_text(json.dumps(manifest, indent=2) + "\n")
        verify(root, pdf, receipt)
        info = subprocess.check_output(["pdfinfo", str(pdf)], text=True)
        print(next(line for line in info.splitlines() if line.startswith("Pages:")))
        print(f"committed inputs={len(records)} PDF sha256={manifest['pdf_sha256']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--legacy", action="store_true")
    args = parser.parse_args()
    directory = Path(__file__).resolve().parent
    root = Path(git(directory, "rev-parse", "--show-toplevel").decode().strip()).resolve()
    stem = "kayfabe_architecture" if args.legacy else "kayfabe_v3"
    if args.check:
        verify(root, directory / (stem + ".pdf"), directory / (stem + ".build.json"))
        print("PDF and committed inputs match build receipt")
    else:
        build(root, directory, stem)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, subprocess.CalledProcessError, OSError) as error:
        raise SystemExit(str(error)) from error
