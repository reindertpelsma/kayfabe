"""Count the cases the upstream Cycles/EEVEE/Workbench runners would run (blocklists applied, NVIDIA + OpenGL).

usage: count_cases.py <path-to-tests-dir>   (run with the checkout's python/ as importable; stdlib only)
"""
import sys, os, importlib.util
sys.dont_write_bytecode=True
tests_root=sys.argv[1]  # a checkout of Blender's tests/ directory (scripts and files)
src=os.path.join(tests_root,"python")
sys.path.insert(0, src)
from modules import render_report as rr
def load(name):
    spec=importlib.util.spec_from_file_location(name, os.path.join(src,name+".py")); m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m); return m
cy=load("cycles_render_tests"); ev=load("eevee_render_tests"); wb=load("workbench_render_tests")
root=os.path.join(tests_root,"files","render")
render_tests="attributes camera colorspace bsdf hair image_colorspace image_data_types image_mapping image_texture_limit integrator light light_group light_linking lightprobe mesh node_inlining pointcloud principled_bsdf raycast shader shadow shadow_catcher sss texture transparency displacement render_layer motion_blur reports volume openvdb instancing denoise denoise_animation".split()
cyc_all=["bake","bake_raytrace"]+render_tests+["osl","image_mipmap","updates"]
gpu_r=[t for t in render_tests+["grease_pencil"] if not any(x in t for x in ["light_group","shadow_catcher","denoise","guiding","reports"])]
wb_t="workbench camera colorspace hair image_colorspace image_data_types image_mapping image_texture_limit mesh pointcloud texture render_layer volume openvdb instancing grease_pencil".split()
def blk_cycles(dev):
    b=list(cy.BLOCKLIST_ALL)+list(cy.BLOCKLIST_OSL_NONE)
    if dev!="CPU": b+=cy.BLOCKLIST_GPU
    if dev=="OPTIX": b+=cy.BLOCKLIST_OPTIX
    return b
def count(dirs,blk):
    tot=0;per={}
    for d in dirs:
        p=os.path.join(root,d)
        if not os.path.isdir(p): continue
        n=0
        for f in rr.blend_list(p,blk,""):
            pl=f.replace(".blend","_permutations.txt")
            n+= sum(1 for _ in open(pl)) if os.path.exists(pl) else 1
        per[d]=n; tot+=n
    return tot,per
for dev in ["CPU","CUDA","OPTIX"]:
    t,per=count(cyc_all,blk_cycles(dev)); print("cycles",dev,t,per)
eb=list(ev.BLOCKLIST)+list(ev.BLOCKLIST_OPENGL)+list(ev.BLOCKLIST_NVIDIA_GL)
t,per=count(gpu_r,eb); print("eevee opengl nvidia",t,per)
t,per=count(gpu_r,list(ev.BLOCKLIST)+list(ev.BLOCKLIST_VULKAN)); print("eevee vulkan nvidia",t)
wbk=["raycast_hit.blend","raycast_normal.blend","raycast_position.blend","raycast_bump.blend"]
t,per=count(wb_t,wbk); print("workbench opengl",t,per)
t,per=count(wb_t,wbk+list(wb.BLOCKLIST_VULKAN)); print("workbench vulkan",t)
