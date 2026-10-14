import socket, json, sys
R = "/var/lib/kf-windows-20261005/boundary-kayfabe-2990"
s = socket.socket(socket.AF_UNIX); s.connect(R + "/qmp.sock"); f = s.makefile("rw"); f.readline()
def c(cmd, **a):
    f.write(json.dumps({"execute": cmd, "arguments": a}) + "\n"); f.flush()
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r: return r
c("qmp_capabilities")
r = c("human-monitor-command", **{"command-line": sys.argv[1]})
print(r.get("return", r))
