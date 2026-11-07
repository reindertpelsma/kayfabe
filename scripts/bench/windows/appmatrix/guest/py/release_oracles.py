# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Pure release-definition oracles. No GPU imports and no hardware qualification here."""
import hashlib
import math
import re

IMAGE_MAE = 1 / 255
IMAGE_MAX_ERROR = 8 / 255


def compare_pixels(actual, reference, width, height):
    count = width * height * 4
    if width < 2 or height < 2 or len(actual) != count or len(reference) != count:
        raise ValueError("empty or wrong-sized RGBA image")
    if not all(math.isfinite(x) and 0 <= x <= 1 for x in actual + reference):
        raise ValueError("invalid image component")
    # The procedural scene contains several differently coloured objects. Two blank images prove nothing.
    span_actual = max(max(actual[c::4]) - min(actual[c::4]) for c in (0, 1, 2))
    span_ref = max(max(reference[c::4]) - min(reference[c::4]) for c in (0, 1, 2))
    if min(span_actual, span_ref) < .05:
        raise ValueError("blank or flat image")
    errors = [abs(a - b) for a, b in zip(actual, reference)]
    mae, worst = math.fsum(errors) / count, max(errors)
    if mae > IMAGE_MAE or worst > IMAGE_MAX_ERROR:
        raise ValueError("image differs from bare-metal reference: mae=%g max=%g" % (mae, worst))
    return {"mae": mae, "max_error": worst}


def compare_text(actual, reference):
    actual, reference = " ".join(actual.split()), " ".join(reference.split())
    if min(len(actual), len(reference)) <= 20 or actual != reference:
        raise ValueError("empty, short or unequal inference output")
    return hashlib.sha256(actual.encode("utf-8")).hexdigest()


def check_reference(doc, expected, raw):
    if (not isinstance(doc, dict) or doc.get("schema") != 1 or doc.get("kind") != "bare-metal" or not raw
            or not re.fullmatch(r"[0-9a-f]{40}", str(doc.get("revision", "")))
            or not re.fullmatch(r"20[0-9]{2}-[0-9]{2}-[0-9]{2}", str(doc.get("date", "")))
            or not isinstance(doc.get("evidence"), str) or not doc["evidence"].strip()
            or doc.get("sha256") != hashlib.sha256(raw).hexdigest()):
        raise ValueError("bare-metal reference provenance/hash missing or invalid")
    for key, value in expected.items():
        if key not in doc or doc[key] != value:
            raise ValueError("reference contract mismatch: " + key)


def check_llm_run(rc, log):
    offload = re.search(r"offloaded ([1-9][0-9]*)/([1-9][0-9]*) layers to GPU", log)
    if rc != 0 or not offload or offload[1] != offload[2] or not re.search(r"(?:Device 0:|CUDA0:).*NVIDIA", log):
        raise ValueError("inference did not finish with full NVIDIA CUDA offload")
