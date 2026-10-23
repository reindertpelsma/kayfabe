"""Keep the guest script off the OS argv; enqueue the exact UTF-8 payload."""
import sys

def command(agent_path,limit):
 return [sys.executable,'-c',
  "import sys,runpy;path=sys.argv[1];limit=sys.argv[2];payload=sys.stdin.buffer.read().decode('utf-8');sys.argv=[path,'run',payload,limit];runpy.run_path(path,run_name='__main__')",
  str(agent_path),str(limit)]
