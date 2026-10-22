# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Exercise the real launcher's USB arguments and QEMU's hotplug boundary without a GPU."""
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

LAUNCHER = Path(__file__).resolve().parents[1] / 'windows/win_vm.sh'


def usb_args():
    source = LAUNCHER.read_text()
    fn = re.search(r'qemu_common\(\) \{\n.*?\n\}', source, re.S).group()
    script = '''set -euo pipefail
NAME=fixture RAM_MB=64 SMP=1 VM=/fixture OVMF_CODE=/fixture OVMF_VARS_TEMPLATE=/fixture SSH_BIND=127.0.0.1 SSH_PORT=1 MAC=52:54:00:00:00:01
''' + fn + '\nqemu_common\nprintf "%s\\n" "${QA[@]}"\n'
    p = subprocess.run(['bash', '-c', script], capture_output=True, text=True, check=True)
    args = p.stdout.splitlines()
    return [args[i+1] for i in range(len(args)-1) if args[i]=='-device' and args[i+1].startswith('qemu-xhci,')]


class DesktopUsbTests(unittest.TestCase):
    def test_matrix_controller_is_part_of_the_real_boot_arguments(self):
        self.assertIn('qemu-xhci,id=xhci', usb_args())

    def test_boot_defines_exactly_one_matrix_bus(self):
        self.assertEqual(usb_args(), ['qemu-xhci,id=xhci'])

    @unittest.skipUnless(shutil.which('qemu-system-x86_64'), 'needs CPU-only QEMU')
    def test_qemu_accepts_the_read_only_cd_on_the_boot_bus(self):
        controllers = usb_args()
        self.assertTrue(controllers, 'launcher must create the USB controller at boot')
        with tempfile.TemporaryDirectory() as tmp:
            iso = Path(tmp)/'fixture.iso'; iso.write_bytes(bytes(2048*20))
            argv = ['nice','-n','19','qemu-system-x86_64','-machine','q35,accel=tcg','-m','64','-nodefaults','-display','none','-S','-qmp','stdio']
            for c in controllers: argv.extend(['-device',c])
            commands = [dict(execute='qmp_capabilities'), dict(execute='blockdev-add', arguments={'driver':'raw','node-name':'testcd','read-only':True,'file':{'driver':'file','filename':str(iso),'read-only':True}}), dict(execute='device_add', arguments={'driver':'usb-bot','id':'testbot','bus':'xhci.0'}), dict(execute='device_add', arguments={'driver':'scsi-cd','id':'testcddev','bus':'testbot.0','drive':'testcd'}), dict(execute='query-block'), dict(execute='quit')]
            p = subprocess.run(argv,input='\n'.join(json.dumps(c) for c in commands)+'\n',capture_output=True,text=True,timeout=15,check=True)
            rows=[json.loads(x) for x in p.stdout.splitlines()]
            self.assertFalse([x for x in rows if 'error' in x], p.stdout)
            drives=[x for row in rows if isinstance(row.get('return'),list) for x in row['return']]
            self.assertTrue(any(x.get('inserted',{}).get('ro') for x in drives),p.stdout)
