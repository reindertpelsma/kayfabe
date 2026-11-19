#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Compare the host run and the guest run of an open row from their two logs (stdlib only).

STATUS: LIVE, 2026-11-17 (design: docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md section 7).

    compare_logs.py nvcodec_matrix HOST_LOG GUEST_LOG
    compare_logs.py nvml_dcgm      HOST_LOG GUEST_LOG

Each log is the one run_apps.sh keeps for the row (apps_matrix.sh: <row>.host.log and <row>.guest.log); the rows print their result files
between @@FILE markers (rowio.py), and a run directory works in place of a log. Prints the per-case or per-field lines and one summary
line; exit status 1 when the comparison has a finding (a candidate kayfabe bug, or a number the host serves that the guest does not).
"""
import argparse
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import dcgm_nvml  # noqa: E402
import nvcodec_matrix  # noqa: E402


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("row", choices=("nvcodec_matrix", "nvml_dcgm"))
    ap.add_argument("host")
    ap.add_argument("guest")
    a = ap.parse_args(argv)
    return (nvcodec_matrix.cmd_compare if a.row == "nvcodec_matrix" else dcgm_nvml.cmd_compare)(a)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
