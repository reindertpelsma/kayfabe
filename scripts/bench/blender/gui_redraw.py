# gui_redraw.py - viewport redraw benchmark inside a windowed Blender GUI session (own code).
#   blender --factory-startup --window-geometry 60 60 1280 800 -P gui_redraw.py -- --sub 8 --objs 3 --ntex 8 --texsize 4096 --iters 60 --out C:\...\gui.json
# Builds the same scene as cycles_bench.py, then for each shading mode times bpy.ops.wm.redraw_timer(type='DRAW_WIN_SWAP', iterations=N)
# (wall clock / N = ms per redraw) in the 3D viewport, records gpu.platform info, then quits. Windowed only; never fullscreen.
import bpy, sys, os, time, json, importlib.util

def args():
    a = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    d = dict(sub="8", objs="3", ntex="8", texsize="4096", iters="60", out=r"C:\kf\apps\blender\out\gui.json", modes="SOLID,MATERIAL",
             texdir=r"C:\kf\apps\blender\tex", warm="5")
    for i in range(0, len(a) - 1, 2): d[a[i].lstrip("-")] = a[i + 1]
    return d

A = args(); R = dict(args=A, status="started", modes={})
def save():
    with open(A["out"], "w") as f: json.dump(R, f, indent=1)
def P(*x): print("GUIBENCH", *x, flush=True)

def run():
    try:
        import gpu
        R["gpu"] = dict(backend=gpu.platform.backend_type_get(), vendor=gpu.platform.vendor_get(),
                        renderer=gpu.platform.renderer_get(), version=gpu.platform.version_get())
        P("gpu", R["gpu"])
        spec = importlib.util.spec_from_file_location("cb", os.path.join(os.path.dirname(__file__) if "__file__" in globals() else r"C:\kf\apps\blender", "cycles_bench.py"))
        cb = importlib.util.module_from_spec(spec); spec.loader.exec_module(cb)
        o = dict(sub=int(A["sub"]), objs=int(A["objs"]), ntex=int(A["ntex"]), texsize=int(A["texsize"]), texdir=A["texdir"])
        t = time.perf_counter(); sc, tris, tex_s, build_s, emp = cb.build(o, reset=False)
        R["tris"] = tris; R["build_s"] = round(time.perf_counter() - t, 1); P("tris", tris, "build_s", R["build_s"])
        win = bpy.context.window_manager.windows[0]
        area = next(a for a in win.screen.areas if a.type == "VIEW_3D")
        region = next(r for r in area.regions if r.type == "WINDOW"); sp = area.spaces[0]
        R["window"] = [win.width, win.height]; R["area"] = [area.width, area.height]
        with bpy.context.temp_override(window=win, area=area, region=region):
            bpy.ops.view3d.view_camera() if False else None
            bpy.ops.view3d.view_all()
            for mode in A["modes"].split(","):
                sp.shading.type = mode
                bpy.ops.wm.redraw_timer(type="DRAW_WIN_SWAP", iterations=int(A["warm"]))   # warm-up (shader compile, EEVEE probes)
                t = time.perf_counter()
                bpy.ops.wm.redraw_timer(type="DRAW_WIN_SWAP", iterations=int(A["iters"]))
                dt = time.perf_counter() - t; n = int(A["iters"])
                R["modes"][mode] = dict(iters=n, total_s=round(dt, 2), ms_per_redraw=round(1000 * dt / n, 2), fps=round(n / dt, 2))
                P(mode, R["modes"][mode]); save()
        R["status"] = "ok"
    except BaseException as e:
        R["status"] = "error"; R["error"] = repr(e)[:500]; P("error", repr(e))
    save(); P("status", R["status"])
    bpy.ops.wm.quit_blender()
    return None

bpy.app.timers.register(run, first_interval=4.0)
