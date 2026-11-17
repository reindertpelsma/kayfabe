#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The MATLAB rows of the Linux proprietary image as an optional payload stage (stdlib only).

STATUS: DESIGN-ONLY, 2026-11-17. GPU-free tests only (test_matlab_stage.py); the stage has never run on a box, it needs a KVM box with a GPU. Wired into vast_payload.py
as the stage `matlab` (large tier, off by default: `--only matlab`, or an instruction that carries `matlab`), and its record is validated in vast_report.py
(`validate_matlab`). Design: docs/design/V3_PROPIMAGE_LINUX.md section 14. The rows, the guest transport and the verdicts are scripts/bench/linux/matlab_rows.py.

    matlab_stage.py run --sha SHA --checkout ROOT --prefix DIR --out FILE [--urls FILE] [--bench DIR] [--deadline-epoch N] [--evidence-dir DIR]
                        ( --instruction-matlab FILE | --matlab-image PATH|URL --matlab-driver VERSION|PATH|URL )
                        [--rows a,b] [--isolation row|shared] [--lane ROW=gate|discovery] [--matlab PATH-IN-GUEST]
    matlab_stage.py plan

Two shapes, never both. STRICT (the Worker): the instruction's `matlab` object names the image by pin (`{from: pin, ref, sha256}`: the ref is looked up in
scripts/ci/matlab_pins.json, the sha256 must equal the pin's, the object is fetched through the presigned-URL file by box_downloader.py and checked against the pinned
size and sha256), the NVIDIA driver version (pinned in ci/vast-broker/release-matrix.json: official URL, sha256, size) and the rows with their lanes; every listed row
must run (zero skips). STANDALONE (`--matlab-image PATH|URL`): the image is the caller's own, NO sha256 pin is enforced (the computed sha256 is recorded), the driver is a
pinned VERSION, a .run file or an https URL (a file or URL driver is sha256-hashed and recorded, not pinned), the policy is permissive (a row that cannot run is a skip
with its reason, exit 0) and every row is in the discovery lane unless `--lane` says otherwise.

One line is printed, `KF_MATLAB_RESULT {json}` (schema kayfabe.matlab-rows/1, at most 48 KiB), and written to --out. Exit: 0 PASS or SKIPPED, 1 a gate row failed, 3 RETRY
(a harness problem or no time), 4 a strict skip, 2 usage. The image never stays on the box (the stage deletes what it downloaded and unpacked) and is never printed.
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import propimage_apps as pa  # noqa: E402

PINS_PATH = HERE / "matlab_pins.json"
MATRIX_PATH = ROOT / "ci/vast-broker/release-matrix.json"
PINS_SCHEMA = "kayfabe.matlab-pins/1"
DEFAULT_PREFIX = "/workspace/kf-matlab"
IMAGE_DEST = "image/image.zip"
VERSION_RE = re.compile(r"[0-9]+\.[0-9]+(\.[0-9]+)?")
HEX64 = re.compile(r"[0-9a-f]{64}")
DISK_FACTOR = 3                 # the zip, its unpacked image and the clone overlays exist together
DISK_HEADROOM = 20 * 1024 ** 3  # DESIGN: driver build tree in the guest is not on the host; overlays and evidence


def rows_module(root=ROOT):
    return pa.load_module(Path(root) / "scripts/bench/linux/matlab_rows.py", "kf_matlab_rows_for_stage")


def load_pins(path=PINS_PATH):
    doc = json.loads(Path(path).read_text())
    if type(doc) is not dict or doc.get("schema") != PINS_SCHEMA or set(doc) != {"schema", "status", "items"}:
        raise ValueError("matlab pins: schema")
    items = doc["items"]
    if type(items) is not list or len(items) != 1:
        raise ValueError("matlab pins: items")
    for it in items:
        keys = {"role", "name", "dest", "ref", "sha256", "size", "needed_by", "recorded_in"}
        if type(it) is not dict or set(it) != keys or it["role"] != "image" or it["dest"] != IMAGE_DEST or it["needed_by"] != "image":
            raise ValueError("matlab pins: item")
        if not pa.NAME_RE.fullmatch(str(it["name"])) or not pa.NAME_RE.fullmatch(str(it["ref"])):
            raise ValueError("matlab pins: names")
        if (it["sha256"] is not None and not (type(it["sha256"]) is str and HEX64.fullmatch(it["sha256"]))) or (
                it["size"] is not None and not (type(it["size"]) is int and 0 < it["size"] < 1 << 40)):
            raise ValueError("matlab pins: sha256 or size")
    return doc


def load_matrix(path=MATRIX_PATH):
    return json.loads(Path(path).read_text())


def driver_pin(version, matrix):
    """The pinned Linux installer of `version` in the release matrix, or None: {version, url, sha256, size, checked}."""
    for p in matrix.get("driver_installers", []):
        if p.get("os") == "linux" and p.get("version") == version and p.get("status") == "PINNED":
            return p
    return None


def instruction_refusal(obj, pins_path=PINS_PATH, matrix=None, root=ROOT):
    """The named refusal of the instruction's `matlab` object (the schema already checked the shape), or None: PIN_UNKNOWN, PIN_UNSET, PIN_SHA256_MISMATCH,
    DRIVER_UNPINNED:<version>, ROW_UNKNOWN:<row>, NO_ROWS. A Worker object is strict: nothing here accepts a source that is not a pin."""
    try:
        pins = load_pins(pins_path)
        matrix = matrix if matrix is not None else load_matrix()
        rows = rows_module(root).ROWS
    except (OSError, ValueError):
        return "PINS_UNREADABLE"
    item = next((i for i in pins["items"] if i["ref"] == obj["image"]["ref"]), None)
    if item is None:
        return "PIN_UNKNOWN"
    if item["sha256"] is None or item["size"] is None:
        return "PIN_UNSET"
    if obj["image"]["sha256"] != item["sha256"]:
        return "PIN_SHA256_MISMATCH"
    if driver_pin(obj["driver"]["version"], matrix) is None:
        return "DRIVER_UNPINNED:" + obj["driver"]["version"]
    if not obj["rows"]:
        return "NO_ROWS"
    for r in obj["rows"]:
        if r not in rows:
            return "ROW_UNKNOWN:" + pa.clean(r, 24)
    return None


def build_manifest(image_item, driver_item):
    """kayfabe.download-manifest/1 for the pinned image (the presigned-URL file) and, when it is a pinned release-matrix installer, the driver (its official URL)."""
    items = [{"name": image_item["name"], "dest": image_item["dest"], "sha256": image_item["sha256"], "size": image_item["size"], "needed_by": "image",
              "sources": [{"kind": "signed", "ref": image_item["ref"]}]}]
    if driver_item is not None:
        items.append({"name": "NVIDIA-Linux-x86_64-%s.run" % driver_item["version"], "dest": "driver/NVIDIA-Linux-x86_64-%s.run" % driver_item["version"],
                      "sha256": driver_item["sha256"], "size": driver_item["size"], "needed_by": "image", "sources": [{"kind": "mirror", "url": driver_item["url"]}]})
    return {"schema": "kayfabe.download-manifest/1", "stages": ["image"], "items": items}


def free_row(free_bytes, size):
    need = DISK_FACTOR * size + DISK_HEADROOM
    return free_bytes >= need, "free %d GiB, need %d GiB" % (free_bytes >> 30, need >> 30)


class Source:
    """Where the standalone image or driver comes from: an existing path or an https URL. Never a pin."""

    def __init__(self, text):
        self.text = text or ""

    @property
    def is_url(self):
        return self.text.startswith("https://")

    def valid(self):
        return bool(self.text) and (self.is_url and re.fullmatch(r"https://[ -~]{1,1000}", self.text) is not None or (not self.is_url and self.text.startswith("/")))


def fetch_standalone(src, dest_dir, name, fetch_url, sha256_file):
    """A path is used where it is; a URL is streamed into dest_dir. -> (path, sha256 computed from the bytes)."""
    if src.is_url:
        dest_dir.mkdir(parents=True, exist_ok=True)
        path = dest_dir / name
        try:
            return path, fetch_url(src.text, path, None)
        except OSError:
            raise pa.Refused("DOWNLOAD") from None
    path = Path(src.text)
    if not path.is_file():
        raise pa.Refused("SOURCE_MISSING")
    return path, sha256_file(path)


def record_for(mr, reason, verdict, sha, strict, rows, lanes, hw, image=None, driver=None, family=None, isolation="row", rows_strict=False):
    """A record with every row refused for one named reason (the shape the report validator reads)."""
    ctx = mr.Context(lanes=lanes, hw=hw, strict=strict or rows_strict, isolation=isolation)
    refusal = mr.BootRefusal(reason, verdict)
    result = {"rows": [mr.refused_record(r, lanes.get(r, "discovery"), refusal) for r in [mr.PREAMBLE] + list(rows)], "boots": []}
    return mr.build_record(result, ctx, sha, image or {"sha256": None, "size": None, "source": "pin" if strict else "user", "pin_enforced": strict},
                           driver or {"version": None, "sha256": None, "source": "pin" if strict else "user"}, family)


def run(a, hooks=None):
    """-> (record, exit_code). `hooks` (tests): {fetch_url, sha256_file, fetch, execute, hardware, klog, free_bytes}."""
    hooks = hooks or {}
    root = Path(a.checkout)
    mr = rows_module(root)
    strict = bool(a.instruction_matlab)
    # --strict-rows (standalone shape, 2026-11-17, `vast_payload.py --full`): the Worker's zero-skip rule for the rows (a row that did not run is STRICT_SKIP, exit 4)
    # WITHOUT its pins: the caller's image and driver are still the caller's, nothing is enforced against a sha256 of ours, the computed sha256 is recorded
    rows_strict = bool(getattr(a, "strict_rows", False)) and not strict
    instruction = json.loads(Path(a.instruction_matlab).read_text()) if strict else None
    prefix = Path(a.prefix)
    rows = mr.select_rows(a.rows) if not strict else [r for r in mr.ROW_ORDER if r in instruction["rows"] and r != mr.PREAMBLE]
    lanes = {r: "discovery" for r in mr.ROW_ORDER}
    lanes.update({r: v["lane"] for r, v in instruction["rows"].items()} if strict else {})
    lanes.update(mr.parse_lanes(a.lane, mr.ROW_ORDER) if not strict and a.lane else {})
    isolation = (instruction.get("isolation") if strict else None) or a.isolation
    hw = (hooks.get("hardware") or mr.host_gpu_facts)()
    family = pa.FAMILY_BY_CC.get(hw.get("compute_cap"))
    image_rec = {"sha256": None, "size": None, "source": "pin" if strict else "user", "pin_enforced": strict}
    driver_rec = {"version": None, "sha256": None, "source": "pin" if strict else "user"}
    ctx_args = dict(lanes=lanes, hw=hw, evidence_dir=a.evidence_dir, workdir=str(prefix / "work"), isolation=isolation, strict=strict or rows_strict, deadline_epoch=a.deadline_epoch,
                    matlab=a.matlab)
    mctx = mr.Context(**ctx_args)
    started, klog = False, None
    pctx = pa.Context(a.sha, root, prefix, a.out, a.urls, bench=Path(a.bench), deadline_epoch=a.deadline_epoch, hardware=hw)
    try:
        pa.check_prefix(prefix)
        if strict:
            reason = instruction_refusal(instruction, root=root, matrix=load_matrix(root / "ci/vast-broker/release-matrix.json"), pins_path=root / "scripts/ci/matlab_pins.json")
            if reason:
                raise pa.Refused(reason)
        started = True
        work, dl, logs = prefix / "work", prefix / "dl", prefix / "logs"
        for d in (work, dl, logs):
            d.mkdir(parents=True, exist_ok=True)
        free = hooks["free_bytes"] if "free_bytes" in hooks else __import__("shutil").disk_usage(pa._existing(prefix)).free
        if strict:
            pins = load_pins(root / "scripts/ci/matlab_pins.json")
            item = next(i for i in pins["items"] if i["ref"] == instruction["image"]["ref"])
            dpin = driver_pin(instruction["driver"]["version"], load_matrix(root / "ci/vast-broker/release-matrix.json"))
            ok, why = free_row(free, item["size"])
            if not ok:
                raise pa.Refused("DISK_FREE")
            manifest = build_manifest(item, dpin)
            mpath = prefix / "manifest.json"
            mpath.write_text(json.dumps(manifest))
            (hooks.get("fetch") or pa.fetch)(pctx, mpath, dl, logs)
            image_path, driver_path = dl / item["dest"], dl / ("driver/NVIDIA-Linux-x86_64-%s.run" % dpin["version"])
            image_rec.update(sha256=item["sha256"], size=item["size"], ref=item["ref"])
            driver_sha, driver_created = dpin["sha256"], dpin["checked"] + "T00:00:00Z"
            driver_rec.update(version=dpin["version"], sha256=dpin["sha256"], ref="linux/" + dpin["version"])
            image_sha_pin = item["sha256"]
        else:
            import propimage_images as pim
            fetch_url, sha_file = hooks.get("fetch_url") or pim.fetch_url, hooks.get("sha256_file") or mr.sha256_file
            if not Source(a.matlab_image).valid():
                raise pa.Refused("IMAGE_SOURCE_BAD")      # an absolute path or an https URL
            image_path, image_sha = fetch_standalone(Source(a.matlab_image), dl, "image.zip", fetch_url, sha_file)
            image_rec.update(sha256=image_sha, size=Path(image_path).stat().st_size)
            image_sha_pin = None
            ok, why = free_row(free, image_rec["size"])
            if not ok:
                raise pa.Refused("DISK_FREE")
            driver_path, driver_sha, driver_created = resolve_standalone_driver(a.matlab_driver, dl, load_matrix(root / "ci/vast-broker/release-matrix.json"), fetch_url,
                                                                                 sha_file, hooks.get("fetch"), pctx, logs, prefix)
        klog = (hooks.get("klog") or (lambda: pa.KernelLog(prefix / "kernel", pctx.klog_source)))()
        klog.start()
        mctx.xid_mark, mctx.xid_since = klog.count, (lambda mark: len(klog.xids(mark + 1, 1 << 62)))
        rec = (hooks.get("execute") or mr.execute)(str(image_path), None, str(work), str(pctx.qemu), str(driver_path), driver_sha, rows, lanes, mctx, a.sha,
                                                   image_sha256=image_sha_pin, driver_created_utc=driver_created, image_rec=image_rec, driver_rec=driver_rec, family=family)
        rec["family"] = family
    except pa.Refused as r:
        harness = r.reason in ("DOWNLOAD",) or r.reason.startswith("KERNEL_LOG")
        rec = record_for(mr, r.reason, "HARNESS" if harness else "NOT_RUN", a.sha, strict, rows, lanes, hw, image_rec, driver_rec, family, isolation, rows_strict)
    except (ValueError, OSError, KeyError, subprocess.SubprocessError) as exc:
        rec = record_for(mr, "INTERNAL", "HARNESS", a.sha, strict, rows, lanes, hw, image_rec, driver_rec, family, isolation, rows_strict)
        rec["summary"] = (rec["summary"] + " (" + pa.clean(exc, 80) + ")")[:200]
    finally:
        if klog is not None:
            klog.stop()
        if started:
            pa.cleanup(pctx)
    return rec, rec["exit_code"]


def resolve_standalone_driver(spec, dl, matrix, fetch_url, sha_file, fetch, pctx, logs, prefix):
    """A pinned VERSION (downloaded through box_downloader from its official URL, checked against the matrix), an https URL or a path to a .run (hashed, not pinned).
    -> (path, sha256, created_utc or None). created_utc: the matrix's `checked` date for a pinned version (an upper bound on the real creation date); None for a file
    or URL, which makes the clock rule fall back to the file's mtime (a WARN; the clock rule refuses a base earlier than that, named KF_CLOCK_BEFORE_INPUT)."""
    spec = spec or ""
    if VERSION_RE.fullmatch(spec):
        pin = driver_pin(spec, matrix)
        if pin is None:
            raise pa.Refused("DRIVER_UNPINNED:" + spec)
        mpath = prefix / "driver_manifest.json"
        manifest = build_manifest({"name": "x", "dest": IMAGE_DEST, "sha256": "0" * 64, "size": 1, "ref": "x"}, pin)
        manifest["items"] = manifest["items"][1:]
        mpath.write_text(json.dumps(manifest))
        (fetch or pa.fetch)(pctx, mpath, dl, logs)
        return dl / manifest["items"][0]["dest"], pin["sha256"], pin["checked"] + "T00:00:00Z"
    if not Source(spec).valid():
        raise pa.Refused("DRIVER_SOURCE_BAD")
    path, sha = fetch_standalone(Source(spec), dl / "driver", re.sub(r"[^A-Za-z0-9._-]", "_", spec.rsplit("/", 1)[-1])[:100] or "driver.run", fetch_url, sha_file)
    return path, sha, None


def finish(rec, out):
    mr = rows_module()
    data = mr.serialise(rec)
    if out:
        tmp = Path(str(out) + ".tmp")
        tmp.write_text(data + "\n")
        tmp.replace(out)
    print("KF_MATLAB_RESULT " + data, flush=True)
    for b in rec.get("breakages", []):
        print("KF_MATLAB_BREAKAGE row=%s class=%s" % (b["row"], b["class"]), flush=True)
    return rec["exit_code"]


def main(argv=None):
    p = argparse.ArgumentParser(prog="matlab_stage.py", description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--sha", required=True)
    r.add_argument("--checkout", default=str(ROOT))
    r.add_argument("--prefix", default=DEFAULT_PREFIX)
    r.add_argument("--out", required=True)
    r.add_argument("--urls", default=pa.DEFAULT_URLS)
    r.add_argument("--bench", default="/workspace/bench")
    r.add_argument("--deadline-epoch", type=float, default=0.0)
    r.add_argument("--evidence-dir")
    r.add_argument("--instruction-matlab", help="the instruction's `matlab` object as a JSON file (the Worker's strict shape)")
    r.add_argument("--matlab-image", default="")
    r.add_argument("--matlab-driver", default="")
    r.add_argument("--strict-rows", action="store_true", help="standalone shape: a row that did not run fails the stage (STRICT_SKIP, exit 4) as in the Worker's shape, "
                   "without its pins (used by `vast_payload.py --full`)")
    r.add_argument("--rows")
    r.add_argument("--isolation", choices=("row", "shared"), default="row")
    r.add_argument("--lane", action="append", default=[])
    r.add_argument("--matlab", default="matlab", help="the matlab binary in the guest")
    sub.add_parser("plan")
    a = p.parse_args(argv)
    try:
        if a.cmd == "plan":
            return rows_module().main(["plan"])
        if not pa.SHA_RE.fullmatch(a.sha):
            return 2
        if bool(a.instruction_matlab) == bool(a.matlab_image):
            print("KF_MATLAB_USAGE give exactly one of --instruction-matlab and --matlab-image", flush=True)
            return 2
        if a.matlab_image and not a.matlab_driver:
            print("KF_MATLAB_USAGE --matlab-image needs --matlab-driver", flush=True)
            return 2
        if a.instruction_matlab and (a.rows or a.lane or a.matlab_driver):
            print("KF_MATLAB_USAGE the instruction decides rows, lanes and driver", flush=True)
            return 2
        rec, _ = run(a)
        return finish(rec, a.out)
    except (ValueError, OSError) as exc:
        print("KF_MATLAB_USAGE " + pa.clean(exc, 100), flush=True)
        return 2


if __name__ == "__main__":
    sys.exit(main())
