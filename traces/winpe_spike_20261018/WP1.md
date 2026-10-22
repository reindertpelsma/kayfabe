# WP1 WinPE spike: stopped before image preparation

STATUS: RESEARCH, 2026-10-22. Evidence-only lane; base revision
`baa2977527c4509145e8033143bd664f768175a6`.

CODE READ: the WP1 assignment requests a locally built WinPE image, an NVIDIA
580.88 driver-load check, CUDA, D3D11 pixel readback, and a ten-minute hold,
followed by a decision about a release CI tier. It also requires stopping and
asking when the provenance or permitted-use question is in doubt.

CODE READ: Microsoft's [WinPE limitations](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/winpe-intro?view=windows-11#limitations),
read on 2026-10-22, limit its purposes to deployment and recovery and explicitly
advise against hardware validation. This confirms the concern in the supplied
research; it is a vendor-documentation finding, not a hardware failure or a
legal opinion. The current ADK EULA and any separate applicable permission have
not been examined.

INFERRED: the requested compute/render/hold CI workload does not establish a
deployment or recovery purpose. The permitted-use conflict needs an owner
answer before building or booting the proposed image. WP1 stopped at this
preparation gate; no technical stage was declared red.

CODE READ: Microsoft's [Validation OS overview](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/validation-os-overview?view=windows-11)
describes a Windows 11-based command-line environment for diagnosing hardware
during device manufacturing, with Win32 application support. Its applicable
terms and NVIDIA/WDDM compatibility have not been verified here. It was not
substituted for WinPE.

INFERRED: owner options are (1) provide applicable permission and its scope for
the proposed WinPE use, (2) revise the lane to evaluate Validation OS after
checking its terms, or (3) close this spike at the preparation gate. Recommendation:
do not adopt WinPE as a release CI tier until this question is resolved.

MEASURED (run WP1-preflight-20261022, 2026-10-22; evidence WP1.md): no SSH connection,
media/driver download, host write, VM launch, GPU query, lock acquisition, build,
test-target directory, or persistent process was created by this lane before
the stop. Only a local evidence worktree was created. The owner's VM was not
operated on; its PID/start-time continuity was not independently observed.

INFERRED: the following are the unchanged future test hypotheses and falsifiers,
recorded before any technical attempt. None has an experimental result.

| Stage | Hypothesis | Falsifier | Verdict / attempts |
| --- | --- | --- | --- |
| S1 | Driver-free PE boots under the specified kayfabe model. | No PE boot within the attempt budget. | NOT RUN / 0 |
| S2 | NVIDIA binds with device problem code 0 and nvidia-smi lists the GPU. | Nonzero problem code, missing nvidia-smi, or failed GPU enumeration. | NOT RUN / 0 |
| S3 | A simple CUDA sample completes correctly. | CUDA initialization, execution, or result error. | NOT RUN / 0 |
| S4 | Hardware D3D11 draws an offscreen triangle with correct pixels. | Device creation fails, software adapter is selected, or pixels mismatch. | NOT RUN / 0 |
| S5 | The S4 loop holds for ten minutes without attributable GPU errors. | Test-attributable Xid, event 153, or TDR in the requested logs. | NOT RUN / 0 |

MEASURED (run WP1-preflight-20261022, 2026-10-22; evidence WP1.md): hardware attempts 0;
before/after technical test counts 0/0; mutation checks not applicable because
no runtime behavior changed; hardware sample count n=0, spread unavailable.
Host Xid delta and filtered QEMU logs were not collected. D3D12 was not run.

INFERRED from the zero-attempt stage table in WP1.md: no WinPE CI coverage is demonstrated. Even a future pass of the
requested stages would leave full Windows desktop/application compatibility,
performance, and long-duration reliability outside this spike's coverage.

CODE READ: the commands below reproduce the vendor-document read only. They
are not an executed VM reproduction recipe; no such recipe was validated.
The owner-specific launcher, tools, memory budget, and firmware settings were
not inspected after the preparation stop.

```sh
python3 - <<'PY'
from html.parser import HTMLParser
from urllib.request import urlopen

class Text(HTMLParser):
    def __init__(self):
        super().__init__()
        self.parts = []
    def handle_data(self, value):
        self.parts.append(value)

sources = (
    ("https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/winpe-intro?view=windows-11", "Limitations"),
    ("https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/validation-os-overview?view=windows-11", "Microsoft Validation OS is"),
)
for url, marker in sources:
    parser = Text()
    with urlopen(url, timeout=30) as response:
        parser.feed(response.read().decode("utf-8"))
    body = " ".join(" ".join(parser.parts).split())
    assert marker in body, (url, "section missing; inspect source")
    print(url)
    print(body[body.index(marker):body.index(marker) + 600])
PY
```

CODE READ: the evidence-only commit requires `[skip ci]`, the configured repo
identity and lane trailers, a diff secret scan, and `python3 scripts/ci/debt.py
claims` reporting `new=0` before its lane-only push. Gate outputs are recorded
in `checks.txt` beside this file.
