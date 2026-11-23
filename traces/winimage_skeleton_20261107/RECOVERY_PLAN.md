# Recovery plan, written BEFORE the first reboot into the UKI (instance 59574978, 2026-11-07)

STATUS: LIVE, 2026-11-07. Written before the swap. The instance is a throwaway (no valuable data).

Facts it rests on (MEASURED before the swap, `facts_before_swap.txt`): Secure Boot off; firmware entries are only `{bootmgr}`
(Windows Boot Manager, `\EFI\Microsoft\Boot\bootmgfw.efi`), UiApp, UEFI Misc Device, EFI Internal Shell; `\EFI\Boot\bootx64.efi` exists
and is byte-identical to `bootmgfw.efi`; the ESP is 100 MiB FAT32 with 65.8 MB free; the NIC is a virtio NIC on QEMU user networking
(10.0.2.15, DHCP), so the public ssh port is a host-forward to guest port 22.

| situation | what I do | how I know |
|---|---|---|
| UKI boots and the network works | ssh in as root with the session key; restore, capture, restore again | `vastai logs <id>` shows `KFUKI READY ip=...` on the serial console |
| UKI boots but ssh never answers (network, key) | nothing: the UKI's own timer (`kf.autorestore=900`) restores the Windows boot files from `\EFI\kf-backup` and reboots after 15 min of no ssh login | serial console shows `NO LOGIN in 900s: automatic restore`, then `Windows Boot Manager` lines |
| UKI does not start at all (firmware refuses it, no `KFUKI` line on the serial console) | the firmware falls through its boot order to the next entry. If the instance then sits in the UEFI menu or shell, there is no console input on Vast: destroy the instance by its own id and, if the swap is to be repeated, rent a new one. Nothing of value is lost on the throwaway. For a box that matters: do not replace both paths; boot the UKI once through a firmware `BootNext` entry instead, so a failed start falls back to Windows (open question for the owner) | no `KFUKI` line within 5 min |
| Windows does not come back after the restore | the restore is verified by sha256 against the backup before the reboot; if it still fails, the same recovery as the row above | serial console, ssh as `vast` |
| the lease is about to end | `vastai destroy instance 59574978 -y`, then `vastai show instances-v1 --raw` must not list the id. An auto-destroy timer (175 min from the rent) is armed | `/root/night/winimage_private/autodestroy.log` |
