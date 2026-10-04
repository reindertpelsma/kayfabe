# Native RTX 4070 D3D11 reference after borrowed-PC return

**STATUS: RESEARCH, 2026-10-04. Native D3D11 passes; no Windows-through-Kayfabe
success, no pool-query observation, no product behavior changed.**

The owner reported the borrowed PC available again. Its preserved working
Windows image passed `qemu-img check`; an independent clone passed check and
logical comparison before use. The [resume recipe](../../tests/resume-native4070.py)
at `219b4ee7` pins the public vast-windows VFIO helper, inventories current Linux
state, assigns only the RTX 4070 and its audio function, and restores original
bindings/services after Windows shuts down. The old pre-reboot journal is not
used as the current recovery state. No physical firmware was changed.

Windows 11 build 26100 booted with NVIDIA 580.88, GSP 580.65.05 and NVIDIA PnP
problem zero. The unrelated emulated VGA device reports problem 10. The probe
explicitly selected the NVIDIA hardware adapter, excluding Microsoft's software
adapter. The existing controller-built probe from `572411c1` completed at
20:53:56 UTC with exit zero: feature level 11.1, four actual GPU completion
events and four texture clear/copy/readbacks, each returning RGBA
`[64,127,191,255]` within the declared rounding tolerance. Device-removed reason
was zero. This is a small headless D3D11 check, not a game/display/application
suite. `probe-build.json` preserves pre-run build metadata; its historical
`runtime_tested: false` is superseded by `probe-result.json` and `probe.stdout`.

The bounded recorder retained **1,074** records: 535 requests, 539 replies,
including four asynchronous events. All pass strict framing, metadata and XOR
checksum validation. The complete text export reconstructs 7,521,456 source
bytes, SHA256 `6aedd98419a06c5169b6996a865865ca36a634bcb6f8db51bf5a7b4e4a2d9c1b`.
The first request/reply queue sequences are 3758/3766 with unknown prefixes.
There are no observed interior gaps, FIFO drops or read failures; 76 unstable
snapshots and scanner candidate rejections remain visible in the stats. An
unknown prefix and passive sampling still prevent completeness claims.

There are **zero** `GR_GFX_POOL_QUERY_SIZE` records. This is not evidence that
the driver never issued it during boot. All four outer RPC IDs occurred in the
earlier capture. Of 22 direct control IDs, 21 occurred there; `0x00809908` is
new to this bounded Windows corpus, with two requests and two successful replies,
16-byte parameters unchanged in the replies. Its meaning remains unresolved;
literal searches of local OGKM 575.51.03/580.65.06, the available ogkm-src tree
and Envytools found no definition. Do not invent a semantic name or call it
Windows-only. These records include background traffic; the capture alone does
not attribute each call to the probe. GPU pushbuffer methods remain outside the
recording and outside an emulation TODO list.

The first start harness attempted `--status` while the collector held an
exclusive device handle, receiving Win32 5. Collection itself continued. That
failed controller command and its exact script are retained. The probe instead
required the active task/service and already-written records; strict decoding
and final collector statistics verified the recording after drain. The corrected
start recipe removes the concurrent open, but was not rerun as a fresh capture.
No recorder binary or ACL changed. Collector exit was zero, its FIFO drained,
the task disabled, and the observer stopped/restored to demand-start.

Only text evidence returned to the controller. `gsp.jsonl.gz` compresses the
text export, not an executable or a physical-memory dump. Windows shut down
normally; `host-restored.json` records Linux restoration at 20:55:26 UTC with
no recovery errors. NVIDIA, snd_hda_intel, gdm and nvidia-persistenced were
verified restored. The PC retains regeneratable images; unique evidence and
recipes are retained here.

```sh
gzip -dc tools/windows-gsp-trace/evidence/2026-10-04-rtx4070-d3d11/gsp.jsonl.gz > /tmp/rtx4070-d3d11.jsonl
python3 tools/windows-gsp-trace/decode.py /tmp/rtx4070-d3d11.jsonl --require-query-pair
```

Expected exit: **4**, because framing validates but no target query pair exists.
