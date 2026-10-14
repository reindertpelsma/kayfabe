import socket, json
R = "/var/lib/kf-windows-20261005/boundary-kayfabe-2990"
s = socket.socket(socket.AF_UNIX); s.connect(R + "/qmp.sock"); f = s.makefile("rw"); f.readline()
f.write(json.dumps({"execute": "qmp_capabilities"}) + "\n"); f.flush(); f.readline()
f.write(json.dumps({"execute": "screendump", "arguments": {"filename": "/tmp/claude-2990.ppm"}}) + "\n"); f.flush(); f.readline()
from PIL import Image
Image.open("/tmp/claude-2990.ppm").convert("RGB").resize((960, 540)).save("/tmp/claude-2990.png")
