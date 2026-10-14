# Owner Edge observation

STATUS: LIVE, 2026-10-11.

Owner reported starting Edge during this focus and seeing a black window recycle.
The click timestamp was not measured. Run1161 is marked as having additional
manual browser activity; its three recorded NVIDIA events occur at15:30:05.153,
15:30:05.679 and15:30:07.115 UTC during the HEVC test. A GSP recovery occurred,
but no host Xid. This correlation does not establish Edge as the trigger.
H264/run1160 and AV1/run1162 independently show the same recovery signature.
A fresh unchanged Edge WebGL/WebGPU run is queued separately. Original verdicts
and facts are unchanged; all three encoders remain open under the clean-run gate.
