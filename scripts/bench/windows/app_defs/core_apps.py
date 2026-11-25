# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The seven applications of propimage_check.CHECKS, as registry definitions (STATUS: LIVE, 2026-11-16).

The timeouts and evidence kinds must equal propimage_check.CHECKS and propimage_apps.EXPECTED_KIND (test_propimage_runspec.py pins them).
Order is the run order: Wukong is last and solo."""
APPS = [
    # without --superposition-run the harness only launches the launcher (LAUNCH_ONLY)
    {"id": "superposition", "title": "Superposition", "timeout": 900, "kind": "FILE",
     "match": [r"unigine.*superposition", r"superposition benchmark"], "harness_args": ["--superposition-run"]},
    {"id": "3dmark", "title": "3DMark", "timeout": 1800, "kind": "FILE", "match": [r"^3dmark", r"ul benchmarks"]},
    {"id": "solidworks", "title": "SOLIDWORKS", "timeout": 900, "kind": "FILE", "match": [r"^solidworks(?! viewer)(?!.*edrawings)"]},
    {"id": "resolve", "title": "DaVinci Resolve", "timeout": 1800, "kind": "FILE", "match": [r"davinci resolve"]},
    {"id": "edrawings", "title": "eDrawings", "timeout": 300, "kind": "UIA", "match": [r"edrawings"]},
    {"id": "edge", "title": "Microsoft Edge", "timeout": 300, "kind": "UIA", "match": [r"^microsoft edge(?! webview)"]},
    # --fb-mb 8192: MEASURED 2026-11-14, 5 of 5 benchmark runs at 8192, 11 of 12 TDR at 4096
    {"id": "wukong", "title": "Black Myth: Wukong Benchmark", "timeout": 480, "kind": "FILE", "match": [r"black myth", r"wukong"],
     "aliases": ["blackmyth"], "solo": True, "harness_args": ["--fb-mb", "8192"]},
]
