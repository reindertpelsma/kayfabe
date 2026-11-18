#!/usr/bin/env python3
"""Win11 base, step 1: new seed, state file, key pair, answer file, KFTOOLS ISO, OVMF vars, swtpm manufacture, empty disk.
Runs ON the pc as root, from PROP_DIR/bin (copied there from the coordinator). Refuses to run twice (init must run once).
Nothing here writes into images A or B or any other existing image. Reads only: (a) the state ids of A and B, to prove the new image
shares none of them, (b) image A's media dir for the OpenSSH MSI and image B's media for the virtio-win ISO (copied, sha256 pinned),
(c) B's signed boot GOP efi (a tool, not identity). The NVIDIA installer is downloaded fresh from NVIDIA's official host and checked
against the repo pin (route 3: sha256 pin; the files inside carry NVIDIA's Authenticode signature).
"""
import hashlib
import json
import secrets
import shutil
import socket
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from w_lib import *  # noqa: E402,F401

PI = load_pi()
STATE = D / "state.json"
A_MEDIA = A_DIR / "media"
B_MEDIA = B_DIR / "media"
NV_URL = "https://us.download.nvidia.com/Windows/580.88/" + "580.88-desktop-win10-win11-64bit-international-dch-whql.exe"


def sha(p):
    return PI.sha256_file(p)


def main():
    if STATE.exists():
        raise SystemExit("REFUSED: %s exists; init runs once (it would replace the key and drop recorded facts)" % STATE)
    need_free(30, "install media copy + 160G thin disk growth")
    for d in ("keys", "media", "firmware", "tpm", "run", "bin", "evidence"):
        (D / d).mkdir(parents=True, exist_ok=True)
    for d in ("keys", "tpm", "firmware", "run"):
        (D / d).chmod(0o700)

    # --- media (all sha256-checked against the guide's pins)
    log("media: sha256 of the Windows ISO")
    assert sha(ISO["win"]) == ISO["win_sha"], "Windows ISO sha mismatch"
    win = D / "media/win.iso"
    if not win.is_symlink():
        win.symlink_to(ISO["win"])
    log("media: copy virtio-win.iso from image B's media (read) and verify")
    vio = D / "media/virtio-win.iso"
    shutil.copyfile(B_MEDIA / "virtio-win.iso", vio)
    assert sha(vio) == ISO["virtio_sha"], "virtio-win sha mismatch"
    src = D / "media/src"
    src.mkdir(exist_ok=True)
    shutil.copyfile(A_MEDIA / "tools.d" / PI.OPENSSH_MSI, src / PI.OPENSSH_MSI)
    assert sha(src / PI.OPENSSH_MSI) == PI.PINS[PI.OPENSSH_MSI], "pin mismatch " + PI.OPENSSH_MSI
    # the NVIDIA installer: fresh from NVIDIA's official host (the pc has direct internet), sha256 against the repo pin
    log("media: download the NVIDIA 580.88 installer from NVIDIA's official host and verify the pin")
    sh(["curl", "-fsSL", "--retry", "3", "-o", str(src / PI.NVIDIA_EXE), NV_URL], timeout=3600)
    got = sha(src / PI.NVIDIA_EXE)
    assert got == PI.PINS[PI.NVIDIA_EXE], "NVIDIA installer sha256 %s != pin" % got
    log("media: NVIDIA installer sha256 %s == the repo pin (size %d)" % (got, (src / PI.NVIDIA_EXE).stat().st_size))

    # --- seed: random, never A's; keep it out of every report (it derives the guest password)
    a_ids = json.loads((A_DIR / "state.json").read_text())["ids"]
    b_ids = json.loads((B_DIR / "state.json").read_text())["ids"]
    listening = subprocess.run(["ss", "-ltnH"], capture_output=True, text=True).stdout
    for _ in range(50):
        seed = "kfprop-w11-" + secrets.token_hex(10)
        st = PI.init_state(seed)
        ids = st["ids"]
        flat = lambda i: {i["smbios"]["uuid"], i["smbios"]["serial"], i["smbios"]["baseboard_serial"], i["smbios"]["chassis_serial"],
                          i["nics"][0]["mac"], i["disks"][0]["serial"], i["gpu_uuid"], i["net"]["subnet"], i["net"]["ssh_port"]}
        if flat(ids) & (flat(a_ids) | flat(b_ids)):
            continue
        if ":%d " % ids["net"]["ssh_port"] in listening:
            continue
        break
    else:
        raise SystemExit("no collision-free seed found")
    log("seed generated (not logged); derived ids share nothing with images A and B; ssh port %d free" % ids["net"]["ssh_port"])

    # --- fresh guest key pair for B
    key = D / "keys/id_ed25519"
    sh(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "kfprop-w11-guest", "-f", str(key)])
    st["guest"]["ssh_private_key"] = key.read_text()
    st["guest"]["ssh_public_key"] = (D / "keys/id_ed25519.pub").read_text().strip()
    computer = "KFW-" + PI._h(seed, "computer-name")[:4].hex().upper()   # NOT A's KFPROP nor B's KFB-
    (D / "run/computer_name.txt").write_text(computer + "\n")   # NOT in state.json: B keeps A's schema exactly
    STATE.write_text(json.dumps(st, indent=2) + "\n")
    STATE.chmod(0o600)
    PI.validate_state(st)
    assert PI.seed_matches(st)
    log("state.json written; computer name %s" % computer)

    # --- tools dir -> KFTOOLS.iso; the answer file is rendered with the new computer name plus two deviations (BypassNRO, first-logon SSH setup)
    td = D / "media/tools.d"
    if td.exists():
        shutil.rmtree(td)
    names = PI.write_tools_dir(st, td, src / PI.OPENSSH_MSI, src / PI.NVIDIA_EXE, template_path=D / "bin/propimage_autounattend.xml.in")
    xml = PI.render_unattend((D / "bin/propimage_autounattend.xml.in").read_text(encoding="utf-8"), st, computer=computer)
    marker7 = '<RunSynchronousCommand wcm:action="add"><Order>10</Order>'
    i = xml.index(marker7)
    j = xml.index("</RunSynchronousCommand>", i) + len("</RunSynchronousCommand>")
    bypass = ('\n    <!-- DEVIATION B1 (image B only): the guest is offline (restrict=on) for the whole build; BypassNRO lets OOBE finish '
              'without a network. -->\n    <RunSynchronousCommand wcm:action="add"><Order>11</Order><Path>reg add '
              'HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\OOBE /v BypassNRO /t REG_DWORD /d 1 /f</Path></RunSynchronousCommand>')
    xml = xml[:j] + bypass + xml[j:]
    tail = "one way only. -->\n   <TimeZone>UTC</TimeZone>"
    assert tail in xml
    flc = ('one way only. -->\n   <TimeZone>UTC</TimeZone>\n'
           '   <!-- DEVIATION B2 (image B only): no human at the console, so the first logon runs the SAME setup_ssh.ps1 '
           'from the KFTOOLS CD (still the one and only way SSH gets in). -->\n'
           '   <FirstLogonCommands><SynchronousCommand wcm:action="add"><Order>1</Order><Description>kfprop setup_ssh</Description>'
           '<CommandLine>powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "foreach ($v in Get-Volume) { if ($v.FileSystemLabel -eq '
           "'KFTOOLS' -and $v.DriveLetter) { &amp; ($v.DriveLetter + ':\\setup_ssh.ps1') *&gt; C:\\Windows\\Temp\\kf_setup_ssh.log } }"
           '"</CommandLine></SynchronousCommand></FirstLogonCommands>')
    xml = xml.replace(tail, flc)
    (td / "autounattend.xml").write_text(xml, encoding="utf-8")
    sh(["xorriso", "-as", "mkisofs", "-quiet", "-V", "KFTOOLS", "-J", "-R", "-iso-level", "3", "-o", str(D / "media/tools.iso"), str(td)])
    log("tools.iso built: %s" % ", ".join(sorted(p.name for p in td.iterdir())))
    # keep only the iso (the installers inside it are pinned); drop the staging copies to save ~1.8 GB
    (D / "media/tools.files.sha256").write_text("".join("%s  %s\n" % (sha(p), p.name) for p in sorted(td.iterdir())))
    shutil.rmtree(td)
    shutil.rmtree(src)

    # --- firmware: the distro's Microsoft-keys variables file (secure boot on), fresh copy
    shutil.copyfile("/usr/share/OVMF/OVMF_VARS_4M.ms.fd", D / "firmware/OVMF_VARS.fd")
    shutil.copyfile("/usr/share/OVMF/OVMF_VARS_4M.ms.fd", D / "firmware/OVMF_VARS.before-gop.fd")
    shutil.copyfile(B_DIR / "firmware/kf-gop.signed.efi", D / "firmware/kf-gop.signed.efi")

    # --- TPM: manufactured once here, kept with the image (state.json embeds it at pack time)
    sh(["swtpm_setup", "--tpm2", "--tpmstate", str(D / "tpm"), "--createek", "--create-ek-cert", "--create-platform-cert",
        "--lock-nvram", "--overwrite", "--pcr-banks", "sha256", "--config", "/etc/swtpm_setup.conf",
        "--logfile", str(D / "run/swtpm_setup.log")])
    log("swtpm manufactured: %s" % sorted(p.name for p in (D / "tpm").iterdir()))

    # --- the empty thin disk
    sh(["qemu-img", "create", "-q", "-f", "qcow2", str(D / "image.qcow2"), "160G"])

    sh(["chown", "-R", "%s:%s" % (SU, SU), str(D)])
    log("PREPARE DONE")


if __name__ == "__main__":
    main()
