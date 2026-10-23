#!/usr/bin/env python3
"""Steel Nomad arm: boot (hold.sh under the fastguest flock), 3DMark Demo GUI Run, N runs per boot, events, stop. Usage: arm_auto.py N REV on|off NRUNS"""
import json,os,subprocess,sys,time
from pathlib import Path
L=Path("/data/claude-hc-20261023");N,REV,MIR,NR=sys.argv[1],sys.argv[2],sys.argv[3],int(sys.argv[4])
A=L/"W"/("boundary-kayfabe-"+N);env=dict(os.environ,Q4_AGENT_STATE=str(L/"agent"))
def sh(*a,t=120):return subprocess.run(list(a),capture_output=True,text=True,timeout=t,cwd=L,env=env)
def agent(ps,t=60):
    p=sh("python3","agent.py","run",ps,str(t),t=t+15);return p.stdout.strip()
def ui(*a):return sh("python3","ui.py",N,*a)
log=open(L/("logs/arm-%s.log"%N),"w")
def say(*a):
    m=time.strftime("%H:%M:%S ")+" ".join(map(str,a));print(m,flush=True);log.write(m+"\n");log.flush()
ATTACH=len(sys.argv)>5 and sys.argv[5]=="attach"
class _H:
    def poll(self):
        q=(L/"W"/("windows-broker-run"+N+".state")).read_text();pid=[l.split("=")[1] for l in q.splitlines() if l.startswith("QPID=")][0]
        return None if os.path.exists("/proc/"+pid) else 1
    def wait(self,timeout=0):
        t=time.time()
        while self.poll() is None and time.time()-t<timeout:time.sleep(2)
    pid=0
hold=_H() if ATTACH else subprocess.Popen(["flock","-o","/tmp/kayfabe-fastguest.lock","bash",str(L/"hold.sh"),N,REV,MIR],stdout=open(L/("logs/hold-%s.log"%N),"w"),stderr=subprocess.STDOUT,env={k:v for k,v in os.environ.items() if not k.startswith("KF3_")})
say("hold pid",hold.pid)
t0=time.time()
while not (A/"qga.sock").exists() and time.time()-t0<120 and hold.poll() is None:time.sleep(2)
ready=False
while time.time()-t0<420 and hold.poll() is None:
    o=agent("hostname",20)
    if "kayfabe-windows" in o.lower():ready=True;break
    time.sleep(5)
say("guest_ready",ready,"after_s",int(time.time()-t0))
res={"run":N,"rev":REV,"mirror":MIR,"ready":ready,"boot_s":int(time.time()-t0),"sn":[]}
if ready:
    time.sleep(25)  # let the desktop settle
    xb=int(sh("bash","-c","dmesg | grep -c \"NVRM: Xid\"").stdout)
    res["xid_before"]=xb
    ui("doubleclick","37","240");say("3DMark launched");time.sleep(55)
    ui("shot","home")
    for i in range(1,NR+1):
        ui("click","788" if i==1 else "456","608" if i==1 else "838")
        st=time.strftime("%H:%M:%S");say("run",i,"clicked")
        w=sh("python3","sn_wait.py",t=450);say("wait",w.stdout.strip())
        time.sleep(4);ui("shot","res%d"%i)
        res["sn"].append({"i":i,"wait":w.stdout.strip(),"shot":str(A/"ui"/("res%d.png"%i))})
        time.sleep(3)
    res["events"]=agent((L/"guest_events.ps1").read_text(),60)
    res["xid_after"]=int(sh("bash","-c","dmesg | grep -c \"NVRM: Xid\"").stdout)
    q=(A/"qemu.log").read_text(errors="replace")
    import re
    m=re.findall(r"mirror=\[[^\]]*\]",q);res["mirror_last"]=m[-1] if m else None
    res["unload_fn47"]=q.count("GSP fn 47 UNLOADING_GUEST_DRIVER")
    res["birth_lines"]=q.count("TSG-MIRROR BIRTH")
    res["refused_acts"]=len(re.findall(r"REFUSED",q))
(L/("logs/result-%s.json"%N)).write_text(json.dumps(res,indent=1))
say(json.dumps(res))
(L/("stop-"+N)).touch()
hold.wait(timeout=300)
sh("bash","-c","gzip -c %s/qemu.log > %s/logs/qemu-%s.log.gz"%(A,L,N))
say("done")
