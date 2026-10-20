#!/usr/bin/env python3
"""The documented builder must publish a bundle with generated data present."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]

class BuildBundle(unittest.TestCase):
    def build(self, raw):
        root=Path(raw);qemu=root/'qemu';qemu.mkdir();(qemu/'VERSION').write_text('10.2.4\n')
        (qemu/'hw/misc').mkdir(parents=True)
        (qemu/'hw/misc/meson.build').write_text('')
        (qemu/'hw/misc/Kconfig').write_text('')
        (qemu/'configure').write_text('''#!/usr/bin/env python3
from pathlib import Path
root=Path.cwd()
(root/'pc-bios').mkdir()
bundle=root/'qemu-bundle/share/qemu';bundle.mkdir(parents=True)
(bundle/'firmware.fd').symlink_to(root/'pc-bios/firmware.fd')
(bundle/'trace-events-all').symlink_to(root/'trace/trace-events-all')
(root/'build.ninja').write_text("rule file\\n  command = mkdir -p $$(dirname $out) && printf fixture > $out\\nbuild qemu-system-x86_64: file\\nbuild pc-bios/firmware.fd: file\\nbuild trace/trace-events-all: file\\n")
''')
        (qemu/'configure').chmod(0o755)
        tools=root/'tools';tools.mkdir();cargo=tools/'cargo'
        cargo.write_text('''#!/usr/bin/env python3
from pathlib import Path
import os
path=Path(os.environ['CARGO_TARGET_DIR'])/'release/libkf_qemu.a'
path.parent.mkdir(parents=True);path.write_bytes(b'archive fixture')
''');cargo.chmod(0o755)
        build=root/'build'
        result=subprocess.run(['bash',str(ROOT/'scripts/bench/build_kf3.sh'),str(qemu),str(build)],
            capture_output=True,text=True,env={**os.environ,'PATH':str(tools)+os.pathsep+os.environ['PATH'],
                                            'CARGO_TARGET_DIR':str(root/'target')})
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        published=list((root/'kf3-bins').glob('*/qemu-system-x86_64'))
        self.assertEqual(len(published),1)
        return published[0].parent

    def test_published_bundle_has_generated_firmware(self):
        with tempfile.TemporaryDirectory() as raw:
            prefix=self.build(raw)
            self.assertTrue((prefix/'pc-bios/firmware.fd').is_file())
            self.assertEqual((prefix/'qemu-bundle/share/qemu/firmware.fd').read_bytes(),b'fixture')

    def test_published_bundle_has_generated_trace_data(self):
        with tempfile.TemporaryDirectory() as raw:
            prefix=self.build(raw)
            self.assertTrue((prefix/'qemu-bundle/share/qemu/trace-events-all').is_file())
            self.assertEqual((prefix/'qemu-bundle/share/qemu/trace-events-all').read_bytes(),b'fixture')

if __name__=='__main__':unittest.main()
