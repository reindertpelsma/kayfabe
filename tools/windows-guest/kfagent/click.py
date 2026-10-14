import socket, json, sys, time
R = "/var/lib/kf-windows-20261005/boundary-kayfabe-2990"
s = socket.socket(socket.AF_UNIX); s.connect(R + "/qmp.sock"); f = s.makefile("rw"); f.readline()
def q(c, a=None):
    f.write(json.dumps({"execute": c, **({"arguments": a} if a else {})}) + "\n"); f.flush()
    while True:
        l = json.loads(f.readline())
        if "return" in l or "error" in l: return l
q("qmp_capabilities")
for fx, fy in [(float(sys.argv[i]), float(sys.argv[i+1])) for i in range(1, len(sys.argv), 2)]:
    x, y = int(fx*32767), int(fy*32767)
    q("input-send-event", {"events": [{"type":"abs","data":{"axis":"x","value":x}},{"type":"abs","data":{"axis":"y","value":y}}]})
    time.sleep(0.3)
    q("input-send-event", {"events": [{"type":"btn","data":{"button":"left","down":True}}]}); time.sleep(0.15)
    q("input-send-event", {"events": [{"type":"btn","data":{"button":"left","down":False}}]}); time.sleep(2)
print("clicked")
