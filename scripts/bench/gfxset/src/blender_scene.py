# blender_scene.py — one deterministic scene, rendered headless by the engine named on the command line.
#   blender -b --factory-startup [--gpu-backend opengl|vulkan] --python blender_scene.py -- <ENGINE> <DEVICE> <out.png>
#   ENGINE: CYCLES (DEVICE = CUDA | OPTIX) · EEVEE (Blender's GPU module: OpenGL or Vulkan backend) · WORKBENCH
# Prints BLENDER_OK <engine> <device> gpu=<name> backend=<gpu backend> w=<w> h=<h> mean=<m>, or BLENDER_FAIL <why>.
# ★ Graded by the caller: md5 of the PNG vs bare metal (same binary, same GPU, same driver). The mean is
#   printed for eyes only (a mean equality is weaker than a byte compare — V3_APP_MATRIX used it).
# ⊘ Determinism: fixed seed, no denoiser, no adaptive sampling, no motion blur, a fixed frame; the
#   noise floor is measured by the caller (two bare-metal runs) rather than assumed.
import sys, bpy

eng, dev, out = sys.argv[sys.argv.index("--") + 1:][:3]
sc = bpy.context.scene
sc.render.resolution_x, sc.render.resolution_y, sc.render.resolution_percentage = 640, 360, 100
sc.render.image_settings.file_format = "PNG"
sc.render.image_settings.color_depth = "8"
sc.render.use_motion_blur = False
sc.frame_set(1)

# the scene: five spheres of different roughness on a plane, one sun, one camera
for o in list(bpy.data.objects):
    bpy.data.objects.remove(o, do_unlink=True)
for i in range(5):
    bpy.ops.mesh.primitive_uv_sphere_add(radius=0.5, location=(i - 2, 0, 0.5), segments=48, ring_count=24)
    bpy.ops.object.shade_smooth()
    m = bpy.data.materials.new("m%d" % i); m.use_nodes = True
    b = m.node_tree.nodes["Principled BSDF"]
    b.inputs["Base Color"].default_value = (0.2 * i, 0.5, 1 - 0.2 * i, 1)
    b.inputs["Roughness"].default_value = 0.1 + 0.2 * i
    bpy.context.object.data.materials.append(m)
bpy.ops.mesh.primitive_plane_add(size=20)
bpy.ops.object.light_add(type="SUN", location=(3, -4, 6))
bpy.context.object.data.energy = 3.0
bpy.context.object.rotation_euler = (0.6, 0.2, 0.4)
bpy.ops.object.camera_add(location=(0, -7, 3.2), rotation=(1.2, 0, 0))
sc.camera = bpy.context.object
if sc.world is None:
    sc.world = bpy.data.worlds.new("w")
sc.world.use_nodes = True
sc.world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.05, 0.07, 0.1, 1)

gpu = "-"
if eng == "CYCLES":
    prefs = bpy.context.preferences.addons["cycles"].preferences
    prefs.compute_device_type = dev
    prefs.get_devices()
    gpus = [d for d in prefs.devices if d.type == dev]
    for d in prefs.devices:
        d.use = d.type == dev
    if not gpus:
        print("BLENDER_FAIL no %s device (devices: %s)" % (dev, [(d.name, d.type) for d in prefs.devices])); sys.exit(1)
    gpu = gpus[0].name
    sc.render.engine = "CYCLES"; sc.cycles.device = "GPU"
    sc.cycles.samples = 64; sc.cycles.seed = 1; sc.cycles.use_denoising = False
    sc.cycles.use_adaptive_sampling = False
elif eng == "EEVEE":
    # 4.2+ names the engine BLENDER_EEVEE_NEXT; 4.5 renamed it back to BLENDER_EEVEE
    ids = [e.identifier for e in bpy.types.RenderSettings.bl_rna.properties["engine"].enum_items]
    sc.render.engine = "BLENDER_EEVEE_NEXT" if "BLENDER_EEVEE_NEXT" in ids else "BLENDER_EEVEE"
    try:
        sc.eevee.taa_render_samples = 16
    except AttributeError:
        pass
elif eng == "WORKBENCH":
    sc.render.engine = "BLENDER_WORKBENCH"
else:
    print("BLENDER_FAIL unknown engine %s" % eng); sys.exit(2)

sc.render.filepath = out
bpy.ops.render.render(write_still=True)
try:
    import gpu as _gpu
    backend = _gpu.platform.backend_type_get()
    gpu = gpu if eng == "CYCLES" else _gpu.platform.renderer_get()
except Exception as e:  # the gpu module is unavailable when no GPU context was ever made
    backend = "none(%s)" % type(e).__name__
img = bpy.data.images.load(out)
px = list(img.pixels)
print("BLENDER_OK %s %s gpu=%s backend=%s w=%d h=%d mean=%.6f" % (eng, dev, str(gpu).replace(" ", "_"), backend,
      img.size[0], img.size[1], sum(px) / len(px)))
