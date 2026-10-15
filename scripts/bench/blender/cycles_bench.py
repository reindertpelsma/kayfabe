# cycles_bench.py - procedural Cycles benchmark scene for the kayfabe Win11 VM (own code).
#   blender -b --factory-startup -P cycles_bench.py -- --device OPTIX --sub 8 --objs 3 --ntex 8 --texsize 4096 \
#           --res 1920 1080 --samples 96 --denoise 0 --frames 0 --texdir C:\kf\apps\blender\tex --out result.json
# Scene: OBJS distinct displaced ico spheres (SUB = ico subdivisions: 20*4^(SUB-1) triangles each), NTEX small spheres each with
# its own TEXSIZE^2 8-bit PNG texture (distinct files, so Cycles uploads each separately), a ground plane, sun + area light.
# Fixed seed, no adaptive sampling. Prints "BENCH key=value" lines; writes a JSON result.
import bpy, sys, os, time, json, math, zlib, struct
import numpy as np

def parse():
    a = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    o = dict(device="OPTIX", sub=7, objs=2, ntex=4, texsize=4096, res=[1920, 1080], samples=64, denoise=0, frames=0,
             texdir=r"C:\kf\apps\blender\tex", out="result.json", threads=0, png="", warm=1, gensec=0)
    i = 0
    while i < len(a):
        k = a[i].lstrip("-")
        if k == "res": o["res"] = [int(a[i + 1]), int(a[i + 2])]; i += 3; continue
        v = a[i + 1]
        o[k] = type(o[k])(v) if not isinstance(o[k], list) else v
        i += 2
    return o

def write_png(path, arr):  # arr: HxWx3 uint8; pure zlib PNG writer
    h, w, _ = arr.shape
    raw = np.concatenate([np.zeros((h, 1), np.uint8), arr.reshape(h, w * 3)], axis=1).tobytes()
    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) +
                chunk(b"IDAT", zlib.compress(raw, 1)) + chunk(b"IEND", b""))

def make_textures(texdir, n, size):
    os.makedirs(texdir, exist_ok=True)
    base = os.path.join(texdir, "base_%d.png" % size)
    if not os.path.exists(base):
        rng = np.random.default_rng(1234)
        y, x = np.mgrid[0:size, 0:size].astype(np.float32) / size
        r = 0.5 + 0.5 * np.sin(18 * x + 7 * np.sin(11 * y))
        g = 0.5 + 0.5 * np.sin(23 * y + 5 * np.cos(9 * x))
        b = 0.5 + 0.5 * np.sin(31 * (x + y))
        img = np.stack([r, g, b], -1)
        img += (rng.random((size // 16, size // 16, 1), dtype=np.float32) * 0.2).repeat(16, 0).repeat(16, 1)
        write_png(base, (np.clip(img, 0, 1) * 255).astype(np.uint8))
    paths = []
    for i in range(n):  # distinct file names -> Cycles loads/uploads each separately
        p = os.path.join(texdir, "t%d_%03d.png" % (size, i))
        if not os.path.exists(p):
            with open(base, "rb") as s, open(p, "wb") as d: d.write(s.read())
        paths.append(p)
    return paths

def material(name, tex_path, seed):
    m = bpy.data.materials.new(name); m.use_nodes = True
    nt = m.node_tree; nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputMaterial"); bsdf = nt.nodes.new("ShaderNodeBsdfPrincipled")
    tc = nt.nodes.new("ShaderNodeTexCoord"); noise = nt.nodes.new("ShaderNodeTexNoise"); bump = nt.nodes.new("ShaderNodeBump")
    noise.inputs["Scale"].default_value = 25 + seed % 7; noise.inputs["Detail"].default_value = 8
    bump.inputs["Strength"].default_value = 0.4
    nt.links.new(tc.outputs["Object"], noise.inputs["Vector"]); nt.links.new(noise.outputs["Fac"], bump.inputs["Height"])
    nt.links.new(bump.outputs["Normal"], bsdf.inputs["Normal"])
    if tex_path:
        it = nt.nodes.new("ShaderNodeTexImage"); it.image = bpy.data.images.load(tex_path); it.projection = "BOX"; it.projection_blend = 0.2
        nt.links.new(tc.outputs["Object"], it.inputs["Vector"]); nt.links.new(it.outputs["Color"], bsdf.inputs["Base Color"])
    bsdf.inputs["Roughness"].default_value = 0.25 + 0.05 * (seed % 6); bsdf.inputs["Metallic"].default_value = 0.0 if seed % 3 else 0.8
    nt.links.new(bsdf.outputs["BSDF"], out.inputs["Surface"])
    return m

def displaced_sphere(name, sub, loc, radius, seed):
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=sub, radius=radius, location=loc)
    ob = bpy.context.active_object; ob.name = name; me = ob.data
    n = len(me.vertices); co = np.empty(n * 3, np.float32); me.vertices.foreach_get("co", co); co = co.reshape(n, 3)
    ph = seed * 1.7
    d = 1 + 0.12 * np.sin(5 * co[:, 0] + ph) * np.sin(4 * co[:, 1] + 2 * ph) + 0.05 * np.sin(17 * co[:, 2] + 3 * co[:, 0] + ph)
    me.vertices.foreach_set("co", (co * d[:, None]).astype(np.float32).ravel()); me.update()
    me.polygons.foreach_set("use_smooth", np.ones(len(me.polygons), np.bool_)); me.update()
    return ob

def build(o, reset=True):
    if reset:
        bpy.ops.wm.read_factory_settings(use_empty=True)
    else:  # GUI session: keep the UI, drop the startup objects
        for ob in list(bpy.data.objects): bpy.data.objects.remove(ob)
    sc = bpy.context.scene
    t0 = time.perf_counter()
    paths = make_textures(o["texdir"], o["ntex"], o["texsize"]) if o["ntex"] > 0 else []
    tex_s = time.perf_counter() - t0
    t0 = time.perf_counter(); tris = 0
    cols = max(1, int(math.ceil(math.sqrt(o["objs"]))))
    for i in range(o["objs"]):
        ob = displaced_sphere("heavy%02d" % i, o["sub"], ((i % cols) * 2.6 - (cols - 1) * 1.3, (i // cols) * 2.6, 1.2), 1.0, i)
        tris += len(ob.data.polygons)
        ob.data.materials.append(material("hm%02d" % i, paths[i % len(paths)] if paths else None, i))
    nt = len(paths)
    for j in range(nt):  # small spheres, one texture each
        ob = displaced_sphere("small%03d" % j, 3, (-5 + (j % 12) * 0.9, -3.0 - (j // 12) * 0.9, 0.35), 0.38, 100 + j)
        tris += len(ob.data.polygons)
        ob.data.materials.append(material("sm%03d" % j, paths[j], 100 + j))
    bpy.ops.mesh.primitive_plane_add(size=60, location=(0, 0, 0))
    pm = bpy.data.materials.new("floor"); pm.use_nodes = True
    pm.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.35, 0.35, 0.4, 1)
    bpy.context.active_object.data.materials.append(pm)
    bpy.ops.object.light_add(type="SUN", location=(4, -4, 8)); bpy.context.active_object.data.energy = 3.0
    bpy.context.active_object.rotation_euler = (math.radians(50), 0, math.radians(30))
    bpy.ops.object.light_add(type="AREA", location=(0, -6, 6)); L = bpy.context.active_object; L.data.energy = 800; L.data.size = 6
    L.rotation_euler = (math.radians(55), 0, 0)
    w = bpy.data.worlds.new("w"); w.use_nodes = True; w.node_tree.nodes["Background"].inputs[0].default_value = (0.3, 0.45, 0.7, 1)
    w.node_tree.nodes["Background"].inputs[1].default_value = 0.8; sc.world = w
    # camera on an orbit empty so an animation just rotates the empty
    bpy.ops.object.empty_add(location=(0, 1, 1)); emp = bpy.context.active_object; emp.name = "orbit"
    bpy.ops.object.camera_add(location=(0, -11, 5.5)); cam = bpy.context.active_object
    cam.parent = emp; sc.camera = cam
    tr = cam.constraints.new("TRACK_TO"); tr.target = emp; tr.track_axis = "TRACK_NEGATIVE_Z"; tr.up_axis = "UP_Y"
    return sc, tris, tex_s, time.perf_counter() - t0, emp

def configure(o, sc):
    sc.render.engine = "CYCLES"; cy = sc.cycles
    cy.samples = o["samples"]; cy.use_adaptive_sampling = False; cy.seed = 7; cy.use_animated_seed = False
    cy.max_bounces = 6; cy.use_denoising = bool(o["denoise"])
    if o["denoise"]:
        cy.denoiser = "OPTIX" if o["device"] == "OPTIX" else "OPENIMAGEDENOISE"
    sc.render.resolution_x, sc.render.resolution_y = o["res"]; sc.render.resolution_percentage = 100
    sc.render.image_settings.file_format = "PNG"
    if o["threads"]:
        sc.render.threads_mode = "FIXED"; sc.render.threads = o["threads"]
    prefs = bpy.context.preferences.addons["cycles"].preferences
    if o["device"] == "CPU":
        cy.device = "CPU"; return "CPU"
    prefs.compute_device_type = o["device"]
    names = []
    for d in prefs.get_devices_for_type(o["device"]):
        d.use = (d.type == o["device"]); names.append(d.name) if d.use else None
    cy.device = "GPU"; return o["device"] + ":" + ",".join(names)

def pix_stats(path):
    img = bpy.data.images.load(path); n = len(img.pixels)
    a = np.empty(n, np.float32); img.pixels.foreach_get(a); a = a.reshape(-1, 4)[:, :3]
    r = (float(a.mean()), float(a.std())); bpy.data.images.remove(img); return r

def main():
    o = parse(); res = dict(args=o, blender=bpy.app.version_string, status="started")
    def save():
        with open(o["out"], "w") as f: json.dump(res, f, indent=1)
    def P(k, v): print("BENCH %s=%s" % (k, v), flush=True); res[k] = v
    try:
        sc, tris, tex_s, build_s, emp = build(o)
        P("tris", tris); P("tex_gen_s", round(tex_s, 2)); P("scene_build_s", round(build_s, 2))
        P("device", configure(o, sc))
        png = o["png"] or os.path.splitext(o["out"])[0] + ".png"
        sc.render.filepath = png
        if o["warm"]:  # kernel load / device init, tiny render
            rx, ry, sm, pc = sc.render.resolution_x, sc.render.resolution_y, sc.cycles.samples, sc.render.resolution_percentage
            sc.render.resolution_x = sc.render.resolution_y = 64; sc.cycles.samples = 1; sc.render.filepath = png + ".warm.png"
            t = time.perf_counter(); bpy.ops.render.render(write_still=True); P("warmup_s", round(time.perf_counter() - t, 2))
            sc.render.resolution_x, sc.render.resolution_y, sc.cycles.samples, sc.render.filepath = rx, ry, sm, png
        res["status"] = "rendering"; save()
        t = time.perf_counter()
        if o["frames"] > 0:
            sc.frame_start, sc.frame_end = 1, o["frames"]
            emp.rotation_euler = (0, 0, 0); emp.keyframe_insert("rotation_euler", frame=1)
            emp.rotation_euler = (0, 0, math.radians(120)); emp.keyframe_insert("rotation_euler", frame=o["frames"])
            sc.render.filepath = os.path.splitext(png)[0] + "_f"
            r = bpy.ops.render.render(animation=True)
        else:
            r = bpy.ops.render.render(write_still=True)
        dt = time.perf_counter() - t
        P("render_result", sorted(r)); P("render_s", round(dt, 2))
        if o["frames"] > 0: P("per_frame_s", round(dt / o["frames"], 2))
        elif os.path.exists(png):
            m, s = pix_stats(png); P("pix_mean", round(m, 5)); P("pix_std", round(s, 5))
        res["status"] = "ok"
    except BaseException as e:  # RuntimeError from a failed render, etc.
        res["status"] = "error"; res["error"] = repr(e)[:500]; print("BENCH error=%r" % (e,), flush=True)
    save(); print("BENCH status=%s" % res["status"], flush=True)

if __name__ == "__main__":
    main()
