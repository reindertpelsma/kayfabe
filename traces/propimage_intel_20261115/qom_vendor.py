import socket,json,subprocess,sys,time,os
Q=sys.argv[1]; cpu=sys.argv[2]
sock="/tmp/qv.sock"
try: os.unlink(sock)
except OSError: pass
p=subprocess.Popen([Q,"-accel","kvm","-machine","q35","-m","256","-nodefaults","-display","none","-S","-cpu",cpu,"-qmp","unix:%s,server=on,wait=off"%sock],stderr=subprocess.DEVNULL)
time.sleep(3)
s=socket.socket(socket.AF_UNIX); s.connect(sock); f=s.makefile("rw"); f.readline()
def c(x):
    f.write(json.dumps(x)+"\n"); f.flush()
    while True:
        r=json.loads(f.readline())
        if "return" in r or "error" in r: return r
c({"execute":"qmp_capabilities"})
cpus=c({"execute":"qom-list","arguments":{"path":"/machine/unattached"}})["return"]
cp=[x["name"] for x in cpus if x["name"].startswith("device")]
for d in cp:
    t=c({"execute":"qom-get","arguments":{"path":"/machine/unattached/"+d,"property":"vendor"}})
    if "return" in t: print(d,"vendor=",t["return"],"family=",c({"execute":"qom-get","arguments":{"path":"/machine/unattached/"+d,"property":"family"}}).get("return")); break
p.terminate()
