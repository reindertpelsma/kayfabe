"""QMP UI helper for the host-campaign VM (same actions as Q4 ui.py): ui.py N shot name | click x y | key k.. """
import json,subprocess,sys,time
from pathlib import Path
N=sys.argv[1];cmd=sys.argv[2];args=sys.argv[3:];A=Path("/data/claude-hc-20261023/W/boundary-kayfabe-%s"%N);out=A/"ui";out.mkdir(exist_ok=True)
QMP="/var/lib/kf-windows-20261005/boundary-tools/qmp.py"
def q(name,values):
 return subprocess.run(["python3",QMP,str(A/"qmp.sock"),"cmd",name,json.dumps(values)],check=True,capture_output=True,text=True,timeout=15).stdout
if cmd=="shot":
 p=out/(args[0]+".ppm");q("screendump",{"filename":str(p),"device":"kf0"})
 from PIL import Image
 Image.open(p).save(out/(args[0]+".png"));p.unlink();print(out/(args[0]+".png"))
elif cmd=="key":q("send-key",{"keys":[{"type":"qcode","data":k} for k in args]})
elif cmd in("click","doubleclick","move"):
 x,y=map(int,args)
 q("input-send-event",{"events":[{"type":"abs","data":{"axis":"x","value":x*32767//1919}},{"type":"abs","data":{"axis":"y","value":y*32767//1079}}]});time.sleep(.15)
 if cmd!="move":
  for _ in range(2 if cmd=="doubleclick" else 1):
   for down in [True,False]:q("input-send-event",{"events":[{"type":"btn","data":{"button":"left","down":down}}]})
   time.sleep(.1)
with (out/"actions.jsonl").open("a") as f:f.write(json.dumps({"t":time.time(),"cmd":cmd,"args":args})+"\n")
