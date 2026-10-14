# kfagent: guest command channel for the Win11 test image (QGA does not start in this image)

**STATUS: LIVE, 2026-10-14.** Lab tooling for the trusted host only. `kfagent_server.py` is a loopback-only broker on the host
(127.0.0.1:8765, reached by the guest at 10.0.2.2 with user-mode networking); the guest runs `agent.ps1` as a SYSTEM scheduled task
(`kfagent`, created by the local-GPO startup script that `inject_qga.sh` writes onto the NTFS volume: also creates the `kfadmin`
auto-logon admin; the password is generated per run into the host's win11-full-b/kfadmin.password and never printed).
CLI: `python3 kfagent_server.py run '<powershell>' [timeout]`. `click.py` / `shot_remote.py` drive QMP mouse clicks and screendumps;
`cnt.sh` counts resets and scanner refusals in the run's qemu.log. Traps: QMP quit does not stop the VM without a guest agent (SIGTERM
the qemu pid); inject while the VM is stopped (`ntfsfix -d` first); re-injection regenerates the password (set it with `net user`).
