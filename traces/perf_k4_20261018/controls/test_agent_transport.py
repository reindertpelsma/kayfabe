import pathlib,subprocess,tempfile,unittest
from agent_transport import command
class AgentTransportTests(unittest.TestCase):
 def run_payload(self,payload):
  with tempfile.TemporaryDirectory(prefix='q4-transport-') as d:
   p=pathlib.Path(d)/'stub.py';p.write_text('import sys,json;print(json.dumps(sys.argv[1:]))')
   import json
   args=command(str(p),60)
   self.assertLess(max(map(len,args)),4096)
   return json.loads(subprocess.run(args,input=payload,text=True,capture_output=True,check=True).stdout)
 def test_large_installer_is_exact(self):
  s='header\n'+('A'*220000)+'\nfooter';self.assertEqual(self.run_payload(s),['run',s,'60'])
 def test_unicode_and_newlines_are_exact(self):
  s='α\r\nβ\n'*50000;self.assertEqual(self.run_payload(s),['run',s,'60'])
 def test_empty_script_still_preserves_arguments(self):self.assertEqual(self.run_payload(''),['run','','60'])
if __name__=='__main__':unittest.main()
