import socket, json, sys
sock = sys.argv[1]
cmds = sys.argv[2:]
s = socket.socket(socket.AF_UNIX); s.settimeout(10); s.connect(sock)
f = s.makefile("rw")
f.readline()
f.write(json.dumps({"execute":"qmp_capabilities"})+"\n"); f.flush(); f.readline()
for c in cmds:
    if c.startswith("hmp:"):
        msg = {"execute":"human-monitor-command","arguments":{"command-line":c[4:]}}
    else:
        msg = {"execute":c}
    f.write(json.dumps(msg)+"\n"); f.flush()
    while True:
        line = f.readline()
        if not line: break
        d = json.loads(line)
        if "return" in d or "error" in d:
            print(json.dumps(d) if "error" in d else (d["return"] if isinstance(d["return"],str) else json.dumps(d["return"])))
            break
