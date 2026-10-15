# cycles_soak.py - in-process render loop for the soak test (own code). Reuses build()/configure() of cycles_bench.py.
#   blender -b --factory-startup -P cycles_soak.py -- --device OPTIX --minutes 10 --sub 9 --objs 3 --ntex 8 --out #d\result.json
# Each iteration alternates resolution and denoise on/off; prints "SOAK i=.. res=.. denoise=.. s=.. mean=.." and stops at the deadline or on the first error.
import bpy, sys, os, time, json, importlib.util
a = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
A = dict(device="OPTIX", minutes="10", sub="9", objs="3", ntex="8", texsize="4096", samples="64", out=r"C:\kf\apps\blender\out\soak.json", texdir=r"C:\kf\apps\blender\tex")
for i in range(0, len(a) - 1, 2): A[a[i].lstrip("-")] = a[i + 1]
spec = importlib.util.spec_from_file_location("cb", r"C:\kf\apps\blender\cycles_bench.py"); cb = importlib.util.module_from_spec(spec); spec.loader.exec_module(cb)
R = dict(args=A, iters=[], status="started")
def save():
    with open(A["out"], "w") as f: json.dump(R, f, indent=1)
o = dict(device=A["device"], sub=int(A["sub"]), objs=int(A["objs"]), ntex=int(A["ntex"]), texsize=int(A["texsize"]), texdir=A["texdir"],
         res=[1920, 1080], samples=int(A["samples"]), denoise=0, threads=0)
try:
    sc, tris, _, _, emp = cb.build(o); print("SOAK tris=%d device=%s" % (tris, cb.configure(o, sc)), flush=True)
    sizes = [(1920, 1080), (1280, 720), (2560, 1440), (1024, 576)]; end = time.time() + 60 * float(A["minutes"]); i = 0
    while time.time() < end:
        w, h = sizes[i % 4]; dn = i % 2
        sc.render.resolution_x, sc.render.resolution_y = w, h; sc.cycles.use_denoising = bool(dn)
        if dn: sc.cycles.denoiser = "OPTIX" if o["device"] == "OPTIX" else "OPENIMAGEDENOISE"
        sc.cycles.samples = [64, 96, 128][i % 3]; sc.render.filepath = os.path.splitext(A["out"])[0] + "_it.png"
        emp.rotation_euler = (0, 0, 0.07 * i)   # moves the camera so each frame re-syncs
        t = time.perf_counter(); r = bpy.ops.render.render(write_still=True); dt = time.perf_counter() - t
        R["iters"].append(dict(i=i, res=[w, h], denoise=dn, samples=sc.cycles.samples, s=round(dt, 2), result=sorted(r)))
        print("SOAK i=%d res=%dx%d denoise=%d spp=%d s=%.2f %s" % (i, w, h, dn, sc.cycles.samples, dt, sorted(r)), flush=True)
        if "FINISHED" not in r: raise RuntimeError("render returned %s" % sorted(r))
        i += 1; save()
    R["status"] = "ok"
except BaseException as e:
    R["status"] = "error"; R["error"] = repr(e)[:500]; print("SOAK error=%r" % (e,), flush=True)
save(); print("SOAK status=%s iters=%d" % (R["status"], len(R["iters"])), flush=True)
