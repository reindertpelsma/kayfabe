# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Blender UI viewport animation; no render farm or background-only path.
import bpy
import gpu
import json
import math
import sys
from pathlib import Path
root=Path(sys.argv[sys.argv.index('--')+1])
for i in range(30):
    bpy.ops.mesh.primitive_cube_add(size=0.6,location=((i%6)-3,(i//6)-2,0))
for screen in bpy.data.screens:
    for area in screen.areas:
        if area.type=='VIEW_3D':
            area.spaces.active.shading.type='SOLID'
(root/'viewport-info.txt').write_text(json.dumps(dict(renderer=gpu.platform.renderer_get(),vendor=gpu.platform.vendor_get())))
phase=0.
def tick():
    global phase
    phase+=0.03
    for i,obj in enumerate(bpy.data.objects):
        if obj.type=='MESH': obj.rotation_euler=(phase,phase*(i+1)/20,phase*0.3)
    for screen in bpy.data.screens:
        for area in screen.areas:
            if area.type=='VIEW_3D': area.tag_redraw()
    return 1/30
bpy.app.timers.register(tick,first_interval=0.01,persistent=True)
