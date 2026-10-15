# probe.py - what Blender sees: Cycles devices per backend, GPU module platform info. Run: blender -b --factory-startup -P probe.py
import bpy, sys
print("PROBE blender", bpy.app.version_string, "hash", bpy.app.build_hash.decode())
try:
    import gpu
    for a in ("backend_type_get", "vendor_get", "renderer_get", "version_get"):
        print("PROBE gpu.platform." + a, getattr(gpu.platform, a)())
except Exception as e:
    print("PROBE gpu.platform unavailable in background:", e)
prefs = bpy.context.preferences.addons['cycles'].preferences
for t in ('NONE', 'CUDA', 'OPTIX', 'HIP', 'ONEAPI', 'METAL'):
    try:
        prefs.compute_device_type = t
        devs = prefs.get_devices_for_type(t)
        print("PROBE type", t, "->", [(d.name, d.type, d.id, d.use) for d in devs])
    except Exception as e:
        print("PROBE type", t, "error", e)
prefs.refresh_devices() if hasattr(prefs, "refresh_devices") else None
print("PROBE all devices:", [(d.name, d.type, d.id) for d in prefs.devices])
print("PROBE end")
