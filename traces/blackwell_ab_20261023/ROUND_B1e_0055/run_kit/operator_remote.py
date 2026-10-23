from pathlib import Path
import subprocess,sys
D=Path(__file__).resolve().parent
p=Path(sys.argv[1]);assert p.resolve().is_relative_to(D)
r=subprocess.run(['ssh','-F',str(D/'ssh.conf'),(D/'alias').read_text().strip(),
                  'python3 -I -'],input=p.read_text(),text=True,capture_output=True,timeout=50)
print(r.stdout,end='');print(r.stderr,end='',file=sys.stderr)
raise SystemExit(r.returncode)
