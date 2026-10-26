from pathlib import Path
import subprocess,sys
D=Path('/workspace/codex/sweep-win-20261021');iid=int(sys.argv[1]);source=sys.argv[2]
try:
 subprocess.run(['python3',str(D/'upload_bundle.py'),str(iid),source],check=True)
except BaseException:
 print('DEPLOYMENT_STOP: no transport retry; exporting local receipt and destroying owned id',iid,flush=True)
 subprocess.run(['python3',str(D/'destroy_owned.py'),str(iid)],check=True)
 raise
