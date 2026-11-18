"""Blender-side inventory and case execution. Importable without bpy for tests."""

import json
from pathlib import Path
import runpy
import sys


def enum(owner, name):
    prop = owner.bl_rna.properties[name]
    items = prop.enum_items or prop.enum_items_static
    return [item.identifier for item in items if item.identifier]


def descendants(cls):
    for child in cls.__subclasses__():
        yield child
        yield from descendants(child)


def inventory(bpy):
    """Read registered RNA types, including unavailable compiled-in device options."""
    scene = bpy.context.scene
    prefs = bpy.context.preferences
    cycles = prefs.addons.get("cycles")
    types = bpy.types
    # Engine enum is dynamic. Include registered engine classes alongside it.
    engines = set(enum(scene.render, "engine"))
    engines.update(cls.bl_idname for cls in descendants(types.RenderEngine)
                   if getattr(cls, "bl_idname", ""))
    features = {
        "engine": engines,
        "gpu_backend": enum(prefs.system, "gpu_backend"),
        "modifier": enum(types.Modifier, "type"),
        "physics": enum(types.FieldSettings, "type"),
        "rigid_body": enum(types.RigidBodyObject, "type"),
        "rigid_body_constraint": enum(types.RigidBodyConstraint, "type"),
        "collision_shape": enum(types.RigidBodyObject, "collision_shape"),
        "fluid": enum(types.FluidDomainSettings, "domain_type"),
        "fluid_flow": enum(types.FluidFlowSettings, "flow_type"),
        "viewport": enum(types.View3DShading, "type"),
        "object": enum(types.Object, "type"),
    }
    if cycles:
        # Python-backed dynamic enums can have no static RNA items. The Cycles
        # preferences callback supplies available compiled device identifiers.
        devices = cycles.preferences.get_device_types(bpy.context)
        features.update(device={"CPU", "HIP", *(item[0] for item in devices)} - {"NONE"},
                        # Keep explicit controls visible if a build omits a backend.
                        denoiser={"OPTIX", "OPENIMAGEDENOISE", *enum(scene.cycles, "denoiser")},
                        bake=enum(scene.cycles, "bake_type"))
    for area, tree_type in (("geometry_node", "GeometryNodeTree"),
                            ("compositor_node", "CompositorNodeTree"),
                            ("shader_node", "ShaderNodeTree")):
        tree = bpy.data.node_groups.new("Tier P inventory", tree_type)
        try:
            features[area] = []
            for name in dir(types):
                cls = getattr(types, name)
                if isinstance(cls, type) and issubclass(cls, types.Node):
                    if cls.is_registered_node_type() and cls.poll(tree):
                        features[area].append(cls.bl_rna.identifier)
        finally:
            bpy.data.node_groups.remove(tree)
    # Effect strip types may be exposed by the effect-add operator rather than a strip enum.
    effect = bpy.ops.sequencer.effect_strip_add.get_rna_type().properties["type"]
    features["sequencer_effect"] = [item.identifier for item in effect.enum_items if item.identifier]
    return features


def case_list(features):
    rows = [{"case": f"{area}/{value}", "area": area, "value": value}
            for area, values in sorted(features.items()) for value in sorted(set(values))]
    for row in rows:
        if row["case"] == "denoiser/OPTIX":
            row["device"] = "OPTIX"
        proof = required_proof(row)
        if proof:
            row["gpu_proof"] = proof
    return rows


def required_proof(case):
    area, value = case["area"], case["value"]
    if case.get("gpu_proof"):
        return case["gpu_proof"]
    if area == "gpu_backend" or case.get("backend"):
        return case.get("backend", value)
    if case.get("denoiser", value if area == "denoiser" else None) == "OPTIX":
        return "OPTIX_DENOISE"
    engine = case.get("engine", value if area == "engine" else None)
    if engine in {"BLENDER_EEVEE", "BLENDER_EEVEE_NEXT", "BLENDER_WORKBENCH"}:
        return engine
    device = case.get("device", value if area == "device" else "CPU")
    return device if device != "CPU" else None


def emit(status, reason="-", **kv):
    print("KF_BLEND_RESULT " + json.dumps(dict(status=status, reason=reason, kv=kv)), flush=True)


def configure_cycles(scene, prefs, device, denoiser):
    scene.cycles.seed = 1
    scene.cycles.use_animated_seed = False
    scene.cycles.device = "CPU" if device == "CPU" else "GPU"
    if device != "CPU":
        try:
            prefs.compute_device_type = device
        except (TypeError, ValueError):
            return False
        prefs.refresh_devices()
        for candidate in prefs.devices:
            candidate.use = candidate.type == device
        if not any(candidate.use for candidate in prefs.devices):
            return False
    scene.cycles.use_denoising = denoiser is not None
    if denoiser is not None:
        try:
            scene.cycles.denoiser = denoiser
        except (TypeError, ValueError):
            return False
        scene.cycles.denoising_use_gpu = denoiser == "OPTIX"
    return True


def render(bpy, case, scene, out):
    area, value = case["area"], case["value"]
    engine = case.get("engine", value if area == "engine" else "CYCLES")
    try:
        scene.render.engine = engine
    except (TypeError, ValueError):
        emit("not_supported", "requested render engine unavailable", engine=engine)
        return
    denoiser = value if area == "denoiser" else case.get("denoiser")
    device = case.get("device", value if area == "device" else "OPTIX" if denoiser == "OPTIX" else "CPU")
    if engine == "CYCLES":
        cycles = bpy.context.preferences.addons.get("cycles")
        if cycles is None or not configure_cycles(scene, cycles.preferences, device, denoiser):
            emit("not_supported", "requested Cycles device or denoiser unavailable",
                 device=device, denoiser=denoiser or "-")
            return
        if "samples" in case:
            scene.cycles.samples = case["samples"]
        if "adaptive_sampling" in case:
            scene.cycles.use_adaptive_sampling = case["adaptive_sampling"]
    if "resolution" in case:
        scene.render.resolution_x, scene.render.resolution_y = case["resolution"]
        scene.render.resolution_percentage = 100
    if "frame" in case:
        scene.frame_set(case["frame"])
    scene.render.image_settings.file_format = "TARGA_RAW"
    scene.render.image_settings.color_mode = "RGB"
    scene.render.image_settings.color_depth = "8"
    scene.render.filepath = str(out / "render.tga")
    animation = "frames" in case
    if animation:
        scene.frame_start, scene.frame_end = case["frames"]
        scene.frame_step = 1
        scene.render.filepath = str(out / "frame-")
    if "subdivision" in case or "instances" in case:
        grow_scene(scene, case)
    result = bpy.ops.render.render(animation=animation, write_still=not animation)
    if "FINISHED" not in result:
        emit("fail", "render operator did not finish", error_class="operator")
        return
    if area == "gpu_backend" or case.get("backend"):
        import gpu

        expected = case.get("backend", value)
        if gpu.platform.backend_type_get() != expected:
            emit("fail", "requested GPU backend was not active", error_class="backend")
            return
    emit("pass", engine=engine, device=device if engine == "CYCLES" else "-",
         denoiser=denoiser or "-")


def grow_scene(scene, case):
    """Scale an explicitly named mesh in the in-memory production project."""
    original = scene.objects[case["mesh"]]
    if original.type != "MESH":
        raise ValueError("pressure source must be a mesh")
    if "subdivision" in case:
        modifier = original.modifiers.new("Tier P subdivision", "SUBSURF")
        modifier.render_levels = case["subdivision"]
    for index in range(case.get("instances", 0)):
        duplicate = original.copy()
        scene.collection.objects.link(duplicate)
        duplicate.location.x += (index + 1) * case.get("spacing", 1.0)
        if case.get("unique_meshes"):
            duplicate.data = original.data.copy()


def execute(bpy, case, out):
    # A trusted operation script can drive Blender contexts that need asset-specific setup.
    # It receives the same case and artifact directory for both sides and emits its own result.
    if "operation" in case:
        runpy.run_path(case["operation"], init_globals={"bpy": bpy, "case": case, "out": out, "emit": emit})
        return
    area = case["area"]
    if area not in {"engine", "device", "denoiser", "gpu_backend", "render"}:
        emit("not_run", "requires a production asset operation")
        return
    if area == "gpu_backend":
        # Use a raster engine so the requested frontend processes the render.
        case = {"engine": "BLENDER_EEVEE", **case}
    render(bpy, case, bpy.context.scene, out)
    if case.get("gui"):
        bpy.ops.wm.quit_blender()


def main(args):
    import bpy

    if args[0] == "inventory":
        rows = case_list(inventory(bpy))
        Path(args[1]).write_text(json.dumps(rows, indent=2) + "\n")
    else:
        execute(bpy, json.loads(Path(args[1]).read_text()), Path(args[2]))


if __name__ == "__main__":
    main(sys.argv[sys.argv.index("--") + 1:])
