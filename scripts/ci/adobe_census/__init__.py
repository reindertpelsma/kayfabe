# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline census of allowlisted Adobe GPU observations."""
import re

RENDERERS = ("cuda", "opencl", "software")
VIDEO = (
    "hardware-encoding.h264", "hardware-encoding.hevc",
    "hardware-decoding.h264", "hardware-decoding.hevc",
    "nvenc.h264", "nvenc.hevc", "nvdec.h264", "nvdec.hevc",
)
MERCURY = tuple(f"mercury.{backend}" for backend in RENDERERS)
AVAILABLE = tuple(f"mercury-available.{backend}" for backend in RENDERERS)
PROBE = tuple(f"sniffer-device.{backend}" for backend in ("cuda", "opencl"))
APPS = {
    "Photoshop": ("graphics-processor", "opencl", "opencl-capable", "directx",
                  "display-p3-conversion", "sniffer-opencl-probe"),
    "Premiere Pro": MERCURY + AVAILABLE + VIDEO + PROBE,
    "After Effects": MERCURY + AVAILABLE + PROBE + ("multi-frame-rendering", "advanced-3d", "gpu-panel-preview"),
    "Illustrator": ("gpu-performance", "gpu-preview", "animated-zoom", "real-time-drawing"),
    "InDesign": ("gpu-performance", "gpu-preview"),
    "Media Encoder": MERCURY + AVAILABLE + VIDEO + PROBE,
    "Acrobat": ("2d-graphics-acceleration",),
    "Creative Cloud": ("desktop-gpu-acceleration",),
}
STATES = ("enabled", "disabled", "unsupported", "unknown")
VERSION = r"\d+(?:\.\d+){1,4}"
BASENAME = r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}"


def feature(name, state="unknown", reason="No observation collected", source=None):
    return {"name": name, "state": state, "reason": reason, "source": source}


def validate(records):
    """Require unique app and feature keys, with provenance for observed states."""
    if not isinstance(records, list):
        raise ValueError("expected a list of app records")
    apps = set()
    for record in records:
        if not isinstance(record, dict) or record.keys() != {"app", "version", "features", "raw_files"}:
            raise ValueError("invalid app record fields")
        app, version = record["app"], record["version"]
        if not isinstance(app, str) or app not in APPS or app in apps:
            raise ValueError("unknown or duplicate app")
        apps.add(app)
        if not isinstance(version, str) or (version != "unknown" and not re.fullmatch(VERSION, version)):
            raise ValueError("invalid app version")
        files = record["raw_files"]
        if not isinstance(files, list) or any(
            not isinstance(name, str) or not re.fullmatch(BASENAME, name) for name in files
        ) or len(files) != len(set(files)):
            raise ValueError("invalid or duplicate evidence basename")
        names = set()
        if not isinstance(record["features"], list):
            raise ValueError("expected a feature list")
        for item in record["features"]:
            if not isinstance(item, dict) or item.keys() != {"name", "state", "reason", "source"}:
                raise ValueError("invalid feature fields")
            name, state, reason, source = (item[key] for key in ("name", "state", "reason", "source"))
            if not isinstance(name, str) or name not in APPS[app] or name in names:
                raise ValueError("unknown or duplicate feature")
            names.add(name)
            if state not in STATES or not isinstance(reason, str) or not reason.strip():
                raise ValueError("invalid feature observation")
            if (source is None and state != "unknown") or (source is not None and source not in files):
                raise ValueError("invalid evidence source")
    return records


def unique(matches, label):
    values = set(matches)
    if len(values) > 1:
        raise ValueError(f"conflicting observations for {label}")
    return next(iter(values), None)


def photoshop(text, source):
    """Parse only explicit English System Info fields. Ignore other profile text."""
    version = unique(re.findall(rf"^Adobe Photoshop Version:\s*(?:Adobe Photoshop )?({VERSION})\b",
                                text, re.M), "version")
    result = []
    for label, name in (("useOpenCL", "opencl"), ("IsOpenCLGPUCapable", "opencl-capable"),
                        ("Use Graphics Processor", "graphics-processor")):
        value = unique(re.findall(rf"^[ \t]*{label}:[ \t]*(0|1|true|false|on|off)[ \t]*$",
                                  text, re.M | re.I), label)
        if value is not None:
            state = "enabled" if value.lower() in ("1", "true", "on") else "disabled"
            if name == "opencl-capable" and state == "disabled":
                state = "unsupported"
            result.append(feature(name, state, f"{label}: {value}", source))
    return version, result


def sniffer(text, source):
    """Report observed devices independently of project and codec settings."""
    result = []
    for backend, label in (("cuda", "CUDA"), ("opencl", "OpenCL")):
        value = unique(re.findall(rf"^{label} system device count:[ \t]*(\d+)[ \t]*$", text, re.M),
                       f"{label} system device count")
        if value is not None:
            state = "enabled" if int(value) else "unsupported"
            result.append(feature(f"sniffer-device.{backend}", state,
                                  f"{label} system device count: {value}", source))
    return None, result


def observations(app, text, source):
    """Read the census collector TSV, with optional original observation reasons."""
    result = []
    version = None
    has_version = False
    for line in text.splitlines():
        fields = line.split("\t")
        if len(fields) == 2 and fields[0] == "version" and not has_version:
            version = fields[1]
            has_version = True
        elif len(fields) in (2, 3) and fields[0] == "renderer" and MERCURY[0] in APPS[app]:
            renderer = fields[1]
            reason = fields[2] if len(fields) == 3 else f"Renderer: {renderer}"
            if renderer not in RENDERERS + ("unknown",):
                raise ValueError("invalid renderer")
            for backend in RENDERERS:
                state = "unknown" if renderer == "unknown" else (
                    "enabled" if backend == renderer else "disabled"
                )
                result.append(feature(f"mercury.{backend}", state, reason, source))
        elif len(fields) == 4 and fields[0] == "feature":
            _, name, state, reason = fields
            result.append(feature(name, state, reason, source))
        elif line:
            raise ValueError("invalid observation line")
    return version, result


def parse(app, inputs):
    """Build one complete record from (format, basename, decoded text) entries."""
    if app not in APPS:
        raise ValueError("unknown app")
    versions, files, found = [], [], []
    for kind, filename, content in inputs:
        content = content.lstrip("\ufeff").replace("\r\n", "\n")
        if kind == "system-info" and app == "Photoshop":
            version, values = photoshop(content, filename)
        elif kind == "sniffer" and app == "Photoshop":
            lines = content.splitlines()
            version = None
            values = []
            if "Start platform OpenCL" in lines and "End platform OpenCL" in lines:
                values.append(feature("sniffer-opencl-probe", "enabled",
                                      "Start platform OpenCL / End platform OpenCL", filename))
        elif kind == "sniffer" and set(PROBE) <= set(APPS[app]):
            version, values = sniffer(content, filename)
        elif kind == "observations":
            version, values = observations(app, content, filename)
        else:
            raise ValueError("unsupported format for this app")
        files.append(filename)
        found.extend(values)
        if version is not None:
            versions.append(version)
    record = {"app": app, "version": unique(versions, "version") or "unknown",
              "features": found, "raw_files": files}
    validate([record])
    observed = {item["name"]: item for item in found}
    record["features"] = [observed.get(name, feature(name)) for name in APPS[app]]
    return record


def compare(reference, kf3):
    """Compare the union, preserving missing observations and both reported reasons."""
    def index(records):
        return {record["app"]: (record["version"], {item["name"]: item for item in record["features"]})
                for record in validate(records)}

    ref, guest = index(reference), index(kf3)
    report = []
    for app in sorted(ref.keys() | guest.keys()):
        ref_version, ref_features = ref.get(app, ("unknown", {}))
        kf_version, kf_features = guest.get(app, ("unknown", {}))
        for name in sorted(ref_features.keys() | kf_features.keys()):
            a = ref_features.get(name, feature(name, reason="Feature missing from reference census"))
            b = kf_features.get(name, feature(name, reason="Feature missing from kf3 census"))
            ref_enabled = a["state"] == "enabled"
            kf_enabled = b["state"] == "enabled"
            if ref_enabled != kf_enabled:
                report.append({"app": app, "name": name,
                               "direction": "reference-only" if ref_enabled else "kf3-only",
                               "reference": {"version": ref_version, **a},
                               "kf3": {"version": kf_version, **b}})
    return report


def render_table(report):
    def cell(value):
        return (value.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
                .replace("|", "&#124;").replace("\r", " ").replace("\n", " "))

    def side(item):
        source = f" [{item['source']}]" if item["source"] else ""
        return f"{item['version']} / {item['state']} / {item['reason']}{source}"

    rows = ["| App | Feature | Direction | Reference | kf3 |", "| --- | --- | --- | --- | --- |"]
    for entry in report:
        fields = (entry["app"], entry["name"], entry["direction"],
                  side(entry["reference"]), side(entry["kf3"]))
        rows.append("| " + " | ".join(map(cell, fields)) + " |")
    return "\n".join(rows) + "\n"
