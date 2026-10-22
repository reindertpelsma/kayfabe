"""Execute pinned supplement staging; failure must never create a cached-success marker."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
SCRIPT=Path(__file__).resolve().parents[1]/'diesweep/desktop/stage_nvrtc.ps1'
PWSH=shutil.which('pwsh') or '/root/am/pwsh/pwsh'
@unittest.skipUnless(Path(PWSH).is_file(),'PowerShell required')
class NvrtcStageTests(unittest.TestCase):
 def stage(self,bad_hash=False,tar_rc=0,dlls=True):
  with tempfile.TemporaryDirectory() as d:
   p=Path(d);(p/'kfmedia').mkdir();(p/'kfmedia/cuda_nvrtc.zip').write_bytes(b'pinned fixture archive');sha=hashlib.sha256(b'pinned fixture archive').hexdigest()
   wrapper=r'''
New-PSDrive -Name C -PSProvider FileSystem -Root $env:STAGE_ROOT | Out-Null
function tar.exe {
 $global:LASTEXITCODE=[int]$env:TAR_RC
 Add-Content -Path C:\tar_calls.txt -Value ($args -join ' ')
 if($global:LASTEXITCODE -eq 0 -and $env:MAKE_DLLS -eq '1') {
  New-Item -ItemType Directory -Force C:\kfapps\cuda_nvrtc\archive\bin | Out-Null
  Set-Content C:\kfapps\cuda_nvrtc\archive\bin\nvrtc64_120_0.dll fixture
  Set-Content C:\kfapps\cuda_nvrtc\archive\bin\nvrtc-builtins64_129.dll fixture
 }
}
& $env:STAGE_SCRIPT -ExpectedSha256 $env:WANT_SHA -ExpectedSize 22
'''
   (p/'run.ps1').write_text(wrapper)
   x=subprocess.run([PWSH,'-NoProfile','-NonInteractive','-File',str(p/'run.ps1')],env=dict(os.environ,STAGE_ROOT=d,STAGE_SCRIPT=str(SCRIPT),WANT_SHA='0'*64 if bad_hash else sha,TAR_RC=str(tar_rc),MAKE_DLLS='1' if dlls else '0'),capture_output=True,text=True,timeout=30)
   marker=p/'kfapps/.staged/cuda_nvrtc.json';calls=p/'tar_calls.txt'
   return x,json.loads(marker.read_text()) if marker.exists() else None,calls.read_text() if calls.exists() else ''
 def test_pinned_missing_media_supplement_is_staged(self):
  x,j,c=self.stage();self.assertEqual(x.returncode,0,x.stdout+x.stderr);self.assertTrue(j['ok'])
 def test_staged_marker_retains_the_verified_archive_hash(self):
  x,j,c=self.stage();self.assertEqual(x.returncode,0,x.stderr);self.assertEqual(j['sha256'],hashlib.sha256(b'pinned fixture archive').hexdigest())
 def test_wrong_hash_never_reaches_extractor(self):
  x,j,c=self.stage(bad_hash=True);self.assertNotEqual(x.returncode,0);self.assertIsNone(j);self.assertEqual(c,'')
 def test_extractor_failure_has_no_success_marker(self):
  x,j,c=self.stage(tar_rc=1);self.assertNotEqual(x.returncode,0);self.assertIsNone(j)
 def test_missing_runtime_dlls_have_no_success_marker(self):
  x,j,c=self.stage(dlls=False);self.assertNotEqual(x.returncode,0);self.assertIsNone(j)
if __name__=='__main__':unittest.main()
