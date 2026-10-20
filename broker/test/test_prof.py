# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Run independent C cases separately so every mutation reports all kills."""
import os,resource,subprocess,sys,tempfile,unittest
CASES=['content','repeat-gap','gap-threshold','quantile','overflow','second-window','exclusive','wire','overflow-next','quantile-tail']
exe=sys.argv.pop(1) if len(sys.argv)>1 else './test-prof'
class ProfCases(unittest.TestCase):pass
def case(name):
 def test(self):
  resource.setrlimit(resource.RLIMIT_CORE,(0,0))
  with tempfile.TemporaryDirectory(prefix='kf-prof-k1-c-') as d:
   r=subprocess.run([exe,name],env=dict(os.environ,KF_PROF_C_KEEP_DIR=d),capture_output=True,text=True,timeout=5);self.assertEqual(r.returncode,0,r.stderr)
 return test
for name in CASES:setattr(ProfCases,'test_'+name.replace('-','_'),case(name))
if __name__=='__main__':unittest.main()
