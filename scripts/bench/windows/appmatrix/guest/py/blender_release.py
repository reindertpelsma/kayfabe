# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Run the existing fixed-seed scene, then compare every RGBA component to the native reference.

Blender's Python: --python blender_release.py -- CUDA|OPTIX OUT.png REFS
The reference JSON binds GPU model, Blender version, scene hash, backend, seed and sample count.
"""
import hashlib
import json
from pathlib import Path
import runpy
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from release_oracles import check_reference, compare_pixels


def main():
    import bpy
    dev, out, refs = sys.argv[sys.argv.index("--") + 1:][:3]
    scene = Path(__file__).with_name("blender_render.py")
    ref = Path(refs) / ("blender_" + dev + ".png")
    try:
        raw = ref.read_bytes()
        meta = json.loads(ref.with_suffix(".json").read_text(encoding="utf-8-sig"))
        prefs = bpy.context.preferences.addons["cycles"].preferences
        prefs.compute_device_type = dev
        prefs.get_devices()
        devices = [d for d in prefs.devices if d.type == dev]
        if not devices:
            raise ValueError("no requested GPU backend")
        check_reference(meta, dict(backend=dev, gpu=devices[0].name, seed=1, samples=256, width=960, height=540,
                                  blender_version=bpy.app.version_string,
                                  scene_sha256=hashlib.sha256(scene.read_bytes()).hexdigest()), raw)
        try:
            Path(out).unlink(missing_ok=True)
        except OSError as e:
            print("CHECK cycles_output FAIL", e)
            return 1
        # Original script consumes the first two arguments after --; the extra reference path is ignored.
        runpy.run_path(str(scene), run_name="__main__")
        actual = bpy.data.images.load(out, check_existing=False)
        reference = bpy.data.images.load(str(ref), check_existing=False)
        if list(actual.size) != [960, 540] or list(reference.size) != [960, 540]:
            raise ValueError("wrong image dimensions")
        stats = compare_pixels(list(actual.pixels), list(reference.pixels), 960, 540)
    except OSError as e:
        print("REFERENCE_NOTFOUND", e)
        return 3
    except (ValueError, TypeError, RuntimeError) as e:
        print("CHECK cycles_parity FAIL", e)
        return 1
    print("CYCLES_PARITY_PASS %s mae=%g max_error=%g reference=bare-metal" % (dev, stats["mae"], stats["max_error"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
