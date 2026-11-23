"""Run one configured Blender production case and compare its differential artifacts."""

import hashlib
import json
import math
from pathlib import Path
import re
import subprocess
import sys
import time
from urllib.parse import urlsplit

from blender_case import required_proof
from image_compare import compare_images, validate_image

WORKER = Path(__file__).with_name("blender_case.py")
MARKER = "KF_BLEND_RESULT "
STATUSES = {"pass", "not_supported", "fail", "timeout", "crash", "not_run"}
# INFERRED native signatures. The KF_BLEND_RESULT protocol belongs to this adapter.
OOM = {
    "optix_oom": r"OPTIX_ERROR_(?:HOST_|DEVICE_)?OUT_OF_MEMORY",
    "cuda_oom": r"CUDA_ERROR_OUT_OF_MEMORY|CUDA error:\s*out of memory|out of memory in cu\w+",
    "hip_oom": r"hipErrorOutOfMemory",
    "vulkan_host_oom": r"VK_ERROR_OUT_OF_HOST_MEMORY",
    "vulkan_device_oom": r"VK_ERROR_OUT_OF_DEVICE_MEMORY",
    "oom": r"out of (?:device |gpu |host )?memory|std::bad_alloc",
}


def images(case, out):
    if "frames" in case:
        start, end = case["frames"]
        return (out / f"frame-{frame:04d}.tga" for frame in range(start, end + 1))
    return iter((out / "render.tga",))


def result(case, code, log, out, t0, t1, secs):
    row = dict(case=case["case"], area=case["area"], status="fail", reason="-",
               secs=secs, t0=t0, t1=t1, kv={})
    if code is None:
        return dict(row, status="timeout", reason="time budget exceeded", kv={"error_class": "timeout"})
    if code < 0:
        return dict(row, status="crash", reason=f"signal {-code}", kv={"error_class": f"signal_{-code}"})
    error_classes = [kind for kind, pattern in OOM.items() if re.search(pattern, log, re.I)]
    if error_classes:
        specific = [kind for kind in error_classes if kind != "oom"]
        return dict(row, reason="Blender reported allocation failure",
                    kv={"error_class": "+".join(specific or error_classes)})
    markers = [line[len(MARKER):] for line in log.splitlines() if line.startswith(MARKER)]
    try:
        payload = json.loads(markers[0]) if len(markers) == 1 else {}
        status, reason, kv = payload["status"], payload["reason"], payload["kv"]
        if status not in STATUSES or not isinstance(reason, str) or not isinstance(kv, dict):
            raise ValueError("invalid result")
    except (KeyError, TypeError, ValueError):
        return dict(row, reason=f"missing or invalid result (exit {code})",
                    kv={"error_class": "exit" if code else "protocol", "exit": code})
    if code and status != "fail":
        return dict(row, reason=f"unexpected Blender exit {code}", kv={"error_class": "exit", "exit": code})
    if status == "pass" and case.get("image", True):
        try:
            for path in images(case, out):
                validate_image(path)
        except (OSError, ValueError) as exc:
            return dict(row, reason=f"invalid image output: {exc}", kv={"error_class": "artifact"})
    return dict(row, status=status, reason=reason, kv=kv)


def fetch(url, path):
    deadline = time.monotonic() + 600
    for _ in range(6):
        parts = urlsplit(url)
        if parts.scheme != "https" or parts.netloc not in {"download.blender.org", "studio.blender.org"}:
            raise ValueError("asset URL must use official Blender HTTPS")
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            break
        response = subprocess.run([
            "curl", "--disable", "--fail", "--silent", "--show-error", "--proto", "=https",
            "--max-time", str(remaining), "--max-filesize", "10000000000", "--output", str(path),
            "--write-out", "%{http_code}\n%{redirect_url}", "--", url,
        ], check=True, timeout=remaining, capture_output=True, text=True)
        code, redirect = response.stdout.split("\n", 1)
        if code == "200":
            return
        if not redirect:
            break
        url = redirect
    raise ValueError("asset download did not finish within the request limits")


def asset_path(spec, out):
    if set(spec) - {"path", "url", "sha256"} or ("path" in spec) == ("url" in spec):
        raise ValueError("configure one local asset path or official Blender URL")
    if "path" in spec:
        path = Path(spec["path"]).resolve(strict=True)
    else:
        path = out / "asset.blend"
        try:
            fetch(spec["url"], path)
        except subprocess.SubprocessError as exc:
            # A Studio redirect can contain signed query parameters.
            raise ValueError("official asset download failed") from exc
    with path.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    if "sha256" in spec and spec["sha256"].lower() != digest:
        raise ValueError("asset SHA-256 differs from configured pin")
    (out / "asset.sha256").write_text(digest + "\n")
    return path


def command(blender, asset, case, out):
    args = [blender, "--factory-startup", "--disable-autoexec"]
    if not case.get("gui", False):
        args += ["--background"]
    backend = case.get("backend", case["value"] if case["area"] == "gpu_backend" else None)
    if backend:
        args += ["--gpu-backend", backend.lower()]
    return [*args, str(asset), "--python-exit-code", "17", "--python", str(WORKER),
            "--", "run", str(out / "case.json"), str(out)]


def run(config, case, out):
    out = out.resolve()
    budget = config["seconds"]
    if not math.isfinite(budget) or budget <= 0:
        raise ValueError("seconds must be finite and positive")
    if "frames" in case and not 0 <= case["frames"][0] <= case["frames"][1] <= 999999:
        raise ValueError("invalid animation frame range")
    out.mkdir(parents=True)
    try:
        asset = asset_path(config["asset"], out)
        inputs = dict(asset=(out / "asset.sha256").read_text().strip(),
                      case={k: v for k, v in case.items() if k != "operation"},
                      budget=budget)
        if "operation" in case:
            operation = Path(case["operation"]).resolve(strict=True)
            inputs["operation"] = hashlib.sha256(operation.read_bytes()).hexdigest()
            case = dict(case, operation=str(operation))
        (out / "inputs.json").write_text(json.dumps(inputs, sort_keys=True) + "\n")
    except (ValueError, OSError, subprocess.SubprocessError) as exc:
        row = dict(case=case["case"], area=case["area"], status="not_run", reason=str(exc),
                   secs=0, t0=None, t1=None, kv={})
    else:
        (out / "case.json").write_text(json.dumps(case) + "\n")
        t0, start = time.time(), time.monotonic()
        log = out / "blender.log"
        with log.open("wb") as stream:
            try:
                code = subprocess.run(command(config["blender"], asset, case, out),
                                      stdout=stream, stderr=subprocess.STDOUT, timeout=budget).returncode
            except subprocess.TimeoutExpired:
                code = None
            except OSError as exc:
                row = dict(case=case["case"], area=case["area"], status="not_run", reason=str(exc),
                           secs=0, t0=None, t1=None, kv={})
                code = "not_run"
        if code != "not_run":
            t1, elapsed = time.time(), time.monotonic() - start
            row = result(case, code, log.read_text(errors="replace"), out, t0, t1, elapsed)
    (out / "row.json").write_text(json.dumps(row) + "\n")
    return row


def outcome(row):
    return row["status"], row["kv"].get("error_class")


def compare_case(case, host_rows, guest_row, host_dirs, guest_dir, margin):
    """Check repeat control, error parity and images; retain source rows for diffrun."""
    if len(host_rows) < 2 or len(host_rows) != len(host_dirs):
        raise ValueError("need at least two independent host runs")
    all_dirs = [*host_dirs, guest_dir]
    if len({directory.resolve() for directory in all_dirs}) != len(all_dirs):
        raise ValueError("each host repeat and the guest require distinct run directories")
    try:
        inputs = [(directory / "inputs.json").read_text() for directory in all_dirs]
    except OSError:
        return {"match": None, "reason": "missing input provenance"}
    if len(set(inputs)) != 1:
        return {"match": None, "reason": "input provenance differs"}
    reference = outcome(host_rows[0])
    if any(outcome(row) != reference for row in host_rows[1:]) or reference[0] == "not_run":
        return {"match": None, "reason": "host control incomplete or unstable"}
    actual = outcome(guest_row)
    if actual[0] == "not_run":
        return {"match": None, "reason": "guest run incomplete"}
    if guest_row["case"] != case["case"] or any(row["case"] != case["case"] for row in host_rows):
        return {"match": None, "reason": "row case differs"}
    if actual != reference:
        return {"match": False, "reason": "outcome differs", "host": reference, "guest": actual}
    if reference[0] != "pass":
        return {"match": True, "reason": "same outcome", "host": reference}
    wanted_proof = required_proof(case)
    if wanted_proof:
        for row, directory in zip([*host_rows, guest_row], all_dirs):
            try:
                proof = json.loads((directory / "gpu-proof.json").read_text())
                if (proof["backend"] != wanted_proof or not proof["evidence"]
                        or not row["t0"] <= proof["t0"] <= proof["t1"] <= row["t1"]):
                    raise ValueError("proof backend or time window differs")
            except (OSError, KeyError, TypeError, ValueError):
                return {"match": None, "reason": "missing or invalid GPU-use proof"}
    if not case.get("image", True):
        if any(row["kv"] != host_rows[0]["kv"] for row in host_rows):
            return {"match": None, "reason": "host operation result unstable"}
        return {"match": host_rows[0]["kv"] == guest_row["kv"], "reason": "operation result comparison"}
    details = []
    try:
        for *hosts, guest in zip(*(images(case, directory) for directory in all_dirs)):
            details.append(compare_images(hosts, guest, margin))
    except (OSError, ValueError) as exc:
        return {"match": False, "reason": f"image comparison failed: {exc}"}
    return {"match": bool(details) and all(frame["match"] for frame in details), "images": details}


def main(args):
    mode, case_list, case_id, *rest = args
    rows = json.loads(Path(case_list).read_text())
    case, = (row for row in rows if row["case"] == case_id)
    if mode == "run":
        config, output = rest
        print(json.dumps(run(json.loads(Path(config).read_text()), case, Path(output))))
        return 0
    if mode == "compare":
        margin, guest, *hosts = rest
        host_dirs, guest_dir = list(map(Path, hosts)), Path(guest)
        host_rows = [json.loads((directory / "row.json").read_text()) for directory in host_dirs]
        guest_row = json.loads((guest_dir / "row.json").read_text())
        verdict = compare_case(case, host_rows, guest_row, host_dirs, guest_dir, float(margin))
        print(json.dumps(verdict))
        return {True: 0, False: 1, None: 2}[verdict["match"]]
    raise ValueError("expected run or compare")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
