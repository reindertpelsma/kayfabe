# blender_render.py -- render a procedural Cycles scene on the GPU (argv after --: DEVICE OUT.png)
# and print BLENDER_OK <device> <mean pixel> so host and guest images can be compared.
import bpy, sys
dev, out = sys.argv[sys.argv.index("--") + 1:][:2]
prefs = bpy.context.preferences.addons["cycles"].preferences
prefs.compute_device_type = dev
prefs.get_devices()
gpus = [d for d in prefs.devices if d.type == dev]
for d in prefs.devices: d.use = d.type == dev
if not gpus:
    print("BLENDER_FAIL no %s device (devices: %s)" % (dev, [(d.name, d.type) for d in prefs.devices])); sys.exit(1)
sc = bpy.context.scene
sc.render.engine = "CYCLES"; sc.cycles.device = "GPU"; sc.cycles.samples = 256; sc.cycles.seed = 1
sc.cycles.use_denoising = False
sc.render.resolution_x = 960; sc.render.resolution_y = 540
for i in range(5):
    bpy.ops.mesh.primitive_uv_sphere_add(radius=0.5, location=(i - 2, 0, 0.5), segments=64, ring_count=32)
    m = bpy.data.materials.new("m%d" % i); m.use_nodes = True
    m.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.2 * i, 0.5, 1 - 0.2 * i, 1)
    m.node_tree.nodes["Principled BSDF"].inputs["Roughness"].default_value = 0.1 * i
    bpy.context.object.data.materials.append(m)
bpy.ops.mesh.primitive_plane_add(size=20)
sc.render.filepath = out
bpy.ops.render.render(write_still=True)
img = bpy.data.images.load(out); px = list(img.pixels)
mean = sum(px) / len(px)
print("BLENDER_OK %s gpu=%s mean=%.4f" % (dev, gpus[0].name, mean))
