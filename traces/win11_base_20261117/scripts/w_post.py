#!/usr/bin/env python3
"""Win11 base, step 3 (VM running, ssh up): facts, DWM key, restart test, quiesce, record MachineGuid/EK/stop time, clean shutdown.
Runs ON the pc as root. Every command reaches the guest over the image's own ssh key; nothing is written to image A.
"""
import json
import shutil
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from w_lib import *  # noqa: E402,F401

PI = load_pi()
st = json.loads((D / "state.json").read_text())
KEY = D / "keys/id_ed25519"
OUT = D / "run/post"
OUT.mkdir(exist_ok=True)

FACTS_PS = r"""
$ErrorActionPreference='SilentlyContinue'
$o=[ordered]@{}
$o.computer=$env:COMPUTERNAME
$os=Get-CimInstance Win32_OperatingSystem
$o.os=$os.Caption; $o.build=$os.BuildNumber; $o.os_version=$os.Version; $o.install_date=$os.InstallDate.ToUniversalTime().ToString('o')
$cv=Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$o.edition=$cv.EditionID; $o.display_version=$cv.DisplayVersion; $o.ubr=$cv.UBR
$lic=Get-CimInstance SoftwareLicensingProduct -Filter "PartialProductKey IS NOT NULL AND Name LIKE 'Windows%'" | Select-Object -First 1
$o.license_status=$lic.LicenseStatus; $o.license_desc=$lic.Description
$o.timezone=(Get-TimeZone).Id; $o.culture=(Get-Culture).Name
$s=Get-Service sshd; $o.sshd=@{status=[string]$s.Status;start=[string]$s.StartType}
$o.openssh_file_version=(Get-Item "$env:ProgramFiles\OpenSSH\sshd.exe").VersionInfo.FileVersion
$o.openssh_default_shell=(Get-ItemProperty 'HKLM:\SOFTWARE\OpenSSH').DefaultShell
$o.setup_ssh_log=@(Get-Content C:\Windows\Temp\kf_setup_ssh.log)
$o.problem_devices=@(Get-PnpDevice -PresentOnly | Where-Object { $_.Status -ne 'OK' } | ForEach-Object { "$($_.Class)|$($_.FriendlyName)|$($_.InstanceId)|problem=$($_.Problem)" })
$o.virtio_drivers=@(Get-CimInstance Win32_PnPSignedDriver | Where-Object { $_.Manufacturer -match 'Red Hat' } | ForEach-Object { "$($_.DeviceName) $($_.DriverVersion)" })
$o.secure_boot=(Confirm-SecureBootUEFI)
$o.bitlocker_c=((manage-bde -status C: | Select-String 'Conversion Status|Protection Status') -join ' ; ')
$o.hiberboot=(Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power').HiberbootEnabled
$o.bypassnro=(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\OOBE').BypassNRO
$o.dwm_overlay=(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\Dwm').OverlayTestMode
$o.autologon=(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon').AutoAdminLogon
$o.ip=@(Get-NetIPAddress -AddressFamily IPv4 | ForEach-Object { "$($_.InterfaceAlias) $($_.IPAddress)/$($_.PrefixLength)" })
$o.firewall_rule_kayfabe_block=[bool](Get-NetFirewallRule -DisplayName 'kayfabe-block-internet')
$o.local_users=@(Get-LocalUser | ForEach-Object { "$($_.Name) enabled=$($_.Enabled)" })
$o.wu_service=(Get-Service wuauserv).StartType.ToString()
$o.free_c_gb=[math]::Round((Get-PSDrive C).Free/1GB,1); $o.used_c_gb=[math]::Round((Get-PSDrive C).Used/1GB,1)
try { $o.internet_tcp443=[bool](Test-NetConnection -ComputerName 1.1.1.1 -Port 443 -WarningAction SilentlyContinue).TcpTestSucceeded } catch { $o.internet_tcp443='error' }
try { Resolve-DnsName example.com -Type A -DnsOnly -ErrorAction Stop | Out-Null; $o.dns_example_com='RESOLVED' } catch { $o.dns_example_com='FAILED' }
$o | ConvertTo-Json -Compress -Depth 4
"""

QUIET_PS = r"""
$ok=0
for($i=0;$i -lt 60;$i++){
  $c=(Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -Filter "Name='_Total'").PercentProcessorTime
  $ti=@(Get-Process TiWorker,TrustedInstaller,MoUsoCoreWorker,wuauclt,msiexec,setup,SetupHost -ErrorAction SilentlyContinue)
  if($c -lt 6 -and $ti.Count -eq 0){$ok++}else{$ok=0}
  if($ok -ge 4){ "QUIET after $i rounds"; exit 0 }
  Start-Sleep -Seconds 10
}
"NOT QUIET (last cpu $c; busy: $(($ti|ForEach-Object{$_.Name}) -join ','))"
"""


def ps(script, timeout=400):
    p = ssh_ps(st, KEY, script, timeout=timeout)
    return p


def facts(tag):
    p = ps(FACTS_PS)
    (OUT / ("facts_%s.json" % tag)).write_text(p.stdout)
    (OUT / ("facts_%s.err" % tag)).write_text(p.stderr)
    line = [l for l in p.stdout.splitlines() if l.startswith("{")]
    return json.loads(line[-1]) if line else {}


def wait_ssh(timeout):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if not qemu_pid():
            raise SystemExit("qemu is gone while waiting for ssh")
        try:
            p = sh(ssh_base(st, KEY) + ["echo", "up"], check=False, timeout=30)
            if p.returncode == 0 and "up" in p.stdout:
                return int(time.time() - t0)
        except subprocess.TimeoutExpired:
            pass
        time.sleep(10)
    raise SystemExit("ssh did not come back in %d s" % timeout)


def main():
    log("post: first probe (propimage.py GUEST_PS)")
    g0 = PI.run_guest_probe(st, str(KEY))
    (OUT / "probe_first.json").write_text(json.dumps(g0, indent=1))
    rows = PI.compare(dict(st, clock={"last_time_at_stop_utc": None, "gap_s": 0}, cpu_fingerprint=None), g0)
    log("post: ids as the guest sees them: " + "; ".join("%s=%s" % (r[0], r[1]) for r in rows if r[0] in
        ("smbios_uuid", "smbios_serial", "baseboard_serial", "chassis_serial", "mac", "disk_serial")))
    f0 = facts("first")
    log("post: facts: computer=%s edition=%s build=%s.%s license_status=%s secure_boot=%s tpm_present=%s problem_devices=%d internet=%s dns=%s" % (
        f0.get("computer"), f0.get("edition"), f0.get("build"), f0.get("ubr"), f0.get("license_status"), f0.get("secure_boot"),
        g0.get("tpm_present"), len(f0.get("problem_devices") or []), f0.get("internet_tcp443"), f0.get("dns_example_com")))
    r = ps(r"reg add HKLM\SOFTWARE\Microsoft\Windows\Dwm /v OverlayTestMode /t REG_DWORD /d 5 /f")
    log("post: DWM OverlayTestMode=5 rc=%d %s" % (r.returncode, r.stdout.strip()))
    # autologon (desktop_prep.ps1 from the gate flow; password on stdin only, never logged): the check pipeline needs a signed-in desktop
    ps(r"New-Item -Force -ItemType Directory C:\kf | Out-Null")
    sh(["scp", "-q", "-i", str(KEY), "-P", str(st["ids"]["net"]["ssh_port"]), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
        "-o", "LogLevel=ERROR", str(D / "bin/desktop_prep.ps1"), "%s@127.0.0.1:C:/kf/desktop_prep.ps1" % st["guest"]["user"]])
    p = subprocess.run(ssh_base(st, KEY) + ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", r"C:\kf\desktop_prep.ps1",
                       "-User", st["guest"]["user"], "-PasswordFromStdin"], input=PI.derive_password(st["seed"]) + "\n", text=True, capture_output=True, timeout=120)
    (OUT / "desktop_prep.txt").write_text(p.stdout)
    log("post: desktop_prep (autologon): %s" % (p.stdout.strip().splitlines() or ["(no output)"])[-1])
    log("post: restart test (the image must boot from its own disk, ssh back, autologon to a desktop)")
    ps("Restart-Computer -Force", timeout=60)
    time.sleep(40)
    log("post: ssh back after %d s" % wait_ssh(900))
    desk = "no"
    for _ in range(40):
        r = ps(r"(Get-Process explorer -IncludeUserName -ErrorAction SilentlyContinue | Select-Object -First 1).UserName", 60)
        if st["guest"]["user"].lower() in r.stdout.lower():
            desk = r.stdout.strip()
            break
        time.sleep(8)
    log("post: signed-in desktop after restart (autologon): %s" % desk)
    ps(r"Remove-Item -Recurse -Force C:\kf -ErrorAction SilentlyContinue")
    q = ps(QUIET_PS, timeout=900)
    log("post: quiesce: " + q.stdout.strip().replace("\n", " | "))
    f1 = facts("after_restart")
    log("post: after restart: dwm_overlay=%s uptime ok; used_c_gb=%s free_c_gb=%s" % (f1.get("dwm_overlay"), f1.get("used_c_gb"), f1.get("free_c_gb")))
    # record: MachineGuid, EK hash, stop time, CPU fingerprint (guide step 8), then stop at once
    shutil.copyfile(D / "state.json", D / "run/state.before_record.json")
    guest = PI.run_guest_probe(st, str(KEY))
    PI.fill_from_guest(st, guest)
    (D / "state.json").write_text(json.dumps(st, indent=2) + "\n")
    (D / "run/probe_record.json").write_text(json.dumps(guest, indent=1))
    log("post: recorded machine_guid, ek_pub_hash (%s), last_time_at_stop_utc=%s, cpu_fingerprint" % (
        "set" if guest.get("ek_pub_hash") else "EMPTY", guest.get("date_utc")))
    ps("Stop-Computer -Force", timeout=60)
    for _ in range(90):
        if not qemu_pid():
            break
        time.sleep(4)
    else:
        raise SystemExit("qemu still running 6 min after Stop-Computer (not killed: inspect)")
    log("post: guest stopped cleanly; qemu exited")
    return 0


if __name__ == "__main__":
    sys.exit(main())
