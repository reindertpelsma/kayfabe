# Evidence: Linux proprietary-image tooling, slice 1 (text only)

All numbers MEASURED on the coordinator box on 2026-11-16, run dir `/workspace/imagec_20261116/proof`; the same logs are in
`traces/propimage_linux_slice1_20261116/` of the branch `claude/propimage-linux-tooling-20261116`.

- Base: Ubuntu 24.04 cloud image `release-20261017`, sha256 `04e8f2461a16a44e78a370284471f29616cdd9baf1893df25807b71f985b6c94` (verified after download).
- `init` -> `setup` (unattended, real clock, network on, VNC 127.0.0.1:5907, ssh hostfwd): cloud-init done 28 s after boot; stage A (kernel, headers,
  build-essential, dkms, tmux, ubuntu-desktop-minimal) 462 s; stage B 3 s; two reboots (~8 s to ssh each); total 565 s. Kernel 6.8.0-149-generic,
  1645 packages. Probe: machine-id, hostname, SMBIOS uuid + 3 serials, MAC, disk serial all MATCH; headers + gcc-13 present; cloud-init disabled.
  QMP screendump: GNOME desktop, autologin, X11.
- `inventory` (read only): 53 lines right after provisioning (paths, sizes, mtimes; no contents).
- `seal` (test patterns: e-mail-like regex, token-like regex, literal): 576 s total; guest grep 157 116 files in about 5 min, 15 stock paths hit the
  token-like regex (cert bundles, PPDs, a JWT library's docs); raw image scan 6.7 GB data of 30 GB: literal 0, e-mail-like 0, token-like 33;
  guest stop 9 s; `qemu-img convert -c` -> 2 462 646 272 bytes, sha256 f4474d309c5e0ff81bfb97fdcdd7f94d9a6cb13f2c398a40d234ef7a103b6c52.
  `scan-image` (independent reader) on the sealed qcow2: the same counts, 1 min 46 s.
- `run` x2 (separate clones, `restrict=on`, `-rtc base=2026-11-16T20:10:10Z,clock=vm`): both RESULT MATCH; machine-id, hostname, MAC, SMBIOS, disk serial
  equal the state values (so equal across clones); guest `date` 32 s / 40 s after base; ping and https unreachable; kernel headers + compiler present.
  ssh up 33 s after start; a clone's shutdown ~90 s (user-session stop job timeout).
- `offline-check` (echo commands): +1 reboot: PASS both rounds, guest UTC 20:10:44 then 20:11:42; +35 days: clone started 2026-12-21T20:10:43Z, PASS.
- `auth-selftest` (stub app, no vendor traffic): loopback with localhost, 127.0.0.1 and [::1] redirects: URL + forward command printed, a request through
  `ssh -L` reached the stub, state matched (3/3 PASS); paste flow: code of 20 characters delivered with the right hash, 0 occurrences in the pane log.
- `novnc`: websockify 0.13.0 (wheel sha256 pinned, `pip --no-deps --require-hashes`; its numpy dependency is optional) + noVNC v1.6.0 (sha256 pinned): `vnc.html`
  HTTP 200, listening on 127.0.0.1:6080 only, websocket upgrade 101 to the clone's VNC.
- `pack` 12.5 s, `unpack` 12.6 s (sha256 per member, seed binding): OK; zip 2 463 371 583 bytes.
- Unit tests and mutants: see the PR. NOT run: any application install, the NVIDIA installer (stub only), a kf3 device, Vast.
