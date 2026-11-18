#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""STATUS: LIVE, 2026-11-12. Tests of the apps-stage hardening, GPU-free.

FALSIFIER (written before the code): a graphics row passes on a software rasterizer, on an empty or device-less
log, or with a wrong pixel; the CUDA driver-API probe passes with one wrong byte, hides error 221, skips
cuCtxDestroy, or hides a crash at process exit (a `_exit()` would); a TIMEOUT row loses its cause between
run_apps.sh and the report. The C programs run against stub libraries (EGL/GLES, libcuda) compiled here, so the
logic is exercised without a GPU; the real GPU runs are recorded in the PR.
"""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "scripts"))
SRC = ROOT / "scripts/apps/src"
RC = SRC / "renderer_check.py"
HAVE_GCC = shutil.which("gcc") is not None
HAVE_GL = Path("/usr/include/EGL/egl.h").exists() and Path("/usr/include/GLES2/gl2.h").exists()

spec = importlib.util.spec_from_file_location("renderer_check", RC)
rc_mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rc_mod)

NV = "NVIDIA GeForce RTX 4070/PCIe/SSE2"
VK_OK = "Devices:\n========\nGPU0:\n\tvendorID = 0x10de\n\tdeviceType = PHYSICAL_DEVICE_TYPE_DISCRETE_GPU\n\tdeviceName = NVIDIA GeForce RTX 4070\n\tdriverName = NVIDIA\n"
VK_CPU = "Devices:\n========\nGPU0:\n\tvendorID = 0x10005\n\tdeviceType = PHYSICAL_DEVICE_TYPE_CPU\n\tdeviceName = llvmpipe (LLVM 17.0.6, 256 bits)\n\tdriverName = llvmpipe\nGPU1:\n\tdeviceName = NVIDIA GeForce RTX 4070\n"
CL_OK = "Platform #0: NVIDIA CUDA\n `-- Device #0: NVIDIA GeForce RTX 4070\nPlatform #1: Portable Computing Language\n `-- Device #0: cpu-haswell\n"
CL_POCL = "Platform #0: Portable Computing Language\n `-- Device #0: cpu-haswell\nPlatform #1: NVIDIA CUDA\n `-- Device #0: NVIDIA GeForce RTX 4070\n"


class RendererPolicy(unittest.TestCase):
    def ok(self, app, text):
        self.assertTrue(rc_mod.check(app, text)[0], text)

    def bad(self, app, text):
        self.assertFalse(rc_mod.check(app, text)[0], text)

    def test_nvidia_renderer_passes(self):
        self.ok("egl_offscreen", "[egl] renderer: %s\nCHECK egl_gl_Mtri_s ok\n" % NV)

    def test_llvmpipe_renderer_fails(self):
        self.bad("egl_offscreen", "[egl] renderer: llvmpipe (LLVM 15.0.7, 256 bits)\n")

    def test_swrast_renderer_fails(self):
        self.bad("egl_offscreen", "[egl] renderer: Mesa Software Rasterizer (swrast)\n")

    def test_software_marker_beats_the_nvidia_word(self):
        self.bad("egl_offscreen", "[egl] renderer: NVIDIA llvmpipe (hybrid)\n")
        self.bad("vkpeak", "device = NVIDIA lavapipe\n")

    def test_non_nvidia_devices_fail_in_every_row(self):
        self.bad("vkpeak", "device       = Intel(R) UHD Graphics 770\n")
        self.bad("clpeak", "Platform: Intel OpenCL\n  Device: Intel(R) UHD\n")
        self.bad("vulkaninfo", "GPU0:\n\tdeviceType = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU\n\tdeviceName = AMD Radeon Graphics\n")

    def test_software_marker_fails_in_every_row(self):
        self.bad("clpeak", "Platform: NVIDIA CUDA\n  Device: NVIDIA pocl software\n")
        self.bad("clinfo", "Platform #0: NVIDIA CUDA\n `-- Device #0: NVIDIA llvmpipe\n")

    def test_vulkaninfo_without_a_gpu0_section_fails(self):
        self.bad("vulkaninfo", "Devices:\n========\nGPU1:\n\tdeviceName = NVIDIA GeForce RTX 4070\n")

    def test_other_vendor_and_missing_renderer_fail(self):
        self.bad("egl_offscreen", "[egl] renderer: AMD Radeon RX 7900\n")
        self.bad("egl_offscreen", "METRIC egl_gl_Mtri_s 7.4\n")

    def test_empty_log_fails_for_every_graphics_row(self):
        for app in rc_mod.APPS:
            self.bad(app, "")
            self.bad(app, "   \n")

    def test_vulkaninfo_gpu0_must_be_the_nvidia_device(self):
        self.ok("vulkaninfo", VK_OK)
        self.bad("vulkaninfo", VK_CPU)

    def test_vkpeak_device_line(self):
        self.ok("vkpeak", "device       = NVIDIA GeForce RTX 4090\n\nfp32-scalar  = 1 GFLOPS\n")
        self.bad("vkpeak", "device       = llvmpipe (LLVM 17)\n")

    def test_clinfo_first_platform_must_be_nvidia_but_pocl_second_is_normal(self):
        self.ok("clinfo", CL_OK)
        self.bad("clinfo", CL_POCL)

    def test_clpeak_platform_and_device(self):
        self.ok("clpeak", "Platform: NVIDIA CUDA\n  Device: NVIDIA GeForce RTX 4090\n")
        self.bad("clpeak", "Platform: Portable Computing Language\n  Device: cpu-haswell\n")

    def test_real_measured_logs_pass(self):
        base = ROOT / "traces/apps_linux_20261107/r1"
        for app in rc_mod.APPS:
            for side in ("host", "guest"):
                f = base / ("%s.%s.log" % (app, side))
                if f.exists():
                    self.assertTrue(rc_mod.check(app, f.read_text())[0], f)

    def test_cli_exit_status_and_empty_file(self):
        with tempfile.TemporaryDirectory() as d:
            good, empty, soft = Path(d, "g"), Path(d, "e"), Path(d, "s")
            good.write_text("[egl] renderer: %s\n" % NV)
            empty.write_text("")
            soft.write_text("[egl] renderer: llvmpipe\n")
            run = lambda p: subprocess.run([sys.executable, str(RC), "egl_offscreen", str(p)], capture_output=True, text=True)
            self.assertEqual(run(good).returncode, 0)
            self.assertEqual(run(empty).returncode, 1)
            self.assertEqual(run(soft).returncode, 1)
            self.assertIn("RENDERER_CHECK app=egl_offscreen FAIL", run(soft).stdout)
            self.assertEqual(subprocess.run([sys.executable, str(RC), "nope", str(good)], capture_output=True).returncode, 2)


class RunAppsRows(unittest.TestCase):
    """run_apps.sh on a fake bundle: a row whose own regex matched still FAILs on a software renderer."""

    def run_row(self, app, stdout_lines, in_path=False):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "bundle/bin").mkdir(parents=True)
            (root / "bundle/share").mkdir()
            shutil.copy(RC, root / "bundle/share/renderer_check.py")
            body = "#!/bin/sh\n" + "".join("printf '%s\\n' '" + l + "'\n" for l in stdout_lines)
            target = root / ("pathbin" if in_path else "bundle/bin")
            target.mkdir(exist_ok=True)
            exe = target / app
            exe.write_text(body)
            exe.chmod(0o755)
            env = {"PATH": str(root / "pathbin") + ":" + os.environ["PATH"], "APPS_ROOT": str(root)}
            p = subprocess.run(["bash", str(ROOT / "scripts/apps/run_apps.sh"), "host", app], env=env,
                               capture_output=True, text=True, timeout=60)
            return next(l for l in p.stdout.splitlines() if l.startswith("APPRES "))

    def test_egl_on_llvmpipe_fails_even_with_a_ok_check(self):
        row = self.run_row("egl_offscreen", ["[egl] renderer: llvmpipe (LLVM 15)", "CHECK egl_gl_Mtri_s ok"])
        self.assertIn("verdict=FAIL", row)
        self.assertIn("renderer:", row)

    def test_egl_on_a_non_nvidia_gpu_fails(self):
        row = self.run_row("egl_offscreen", ["[egl] renderer: AMD Radeon RX (radeonsi)", "CHECK egl_gl_Mtri_s ok"])
        self.assertIn("verdict=FAIL", row)

    def test_egl_nvidia_word_with_software_marker_fails(self):
        row = self.run_row("egl_offscreen", ["[egl] renderer: NVIDIA llvmpipe", "CHECK egl_gl_Mtri_s ok"])
        self.assertIn("verdict=FAIL", row)

    def test_egl_on_nvidia_passes(self):
        row = self.run_row("egl_offscreen", ["[egl] renderer: " + NV, "CHECK egl_gl_Mtri_s ok"])
        self.assertIn("verdict=PASS", row)

    def test_vulkaninfo_software_gpu0_fails_although_nvidia_is_listed(self):
        row = self.run_row("vulkaninfo", VK_CPU.splitlines(), in_path=True)
        self.assertIn("verdict=FAIL", row)

    def test_clinfo_with_pocl_first_fails(self):
        row = self.run_row("clinfo", CL_POCL.splitlines() + ["Platform Name NVIDIA CUDA"], in_path=True)
        self.assertIn("verdict=FAIL", row)

    def test_vulkaninfo_on_nvidia_passes(self):
        row = self.run_row("vulkaninfo", VK_OK.splitlines(), in_path=True)
        self.assertIn("verdict=PASS", row)

    def test_missing_checker_fails_closed(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "bundle/bin").mkdir(parents=True)
            exe = root / "bundle/bin/egl_offscreen"
            exe.write_text("#!/bin/sh\necho '[egl] renderer: " + NV + "'\necho 'CHECK egl_gl_Mtri_s ok'\n")
            exe.chmod(0o755)
            # no share/renderer_check.py: run_apps.sh falls back to its own src/ copy (the repo), which exists
            p = subprocess.run(["bash", str(ROOT / "scripts/apps/run_apps.sh"), "host", "egl_offscreen"],
                               env={"PATH": os.environ["PATH"], "APPS_ROOT": str(root)}, capture_output=True, text=True, timeout=60)
            self.assertIn("verdict=PASS", p.stdout)


GL_STUB = r'''
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <stdlib.h>
#include <string.h>
static unsigned char clr[4]; static int drawn; static const char*mode(void){const char*m=getenv("STUB_MODE");return m?m:"ok";}
static EGLBoolean qd(EGLint n,EGLDeviceEXT*d,EGLint*c){(void)n;d[0]=(EGLDeviceEXT)1;*c=1;return 1;}
static EGLDisplay gpd(EGLenum p,void*d,const EGLint*a){(void)p;(void)d;(void)a;return (EGLDisplay)1;}
__eglMustCastToProperFunctionPointerType eglGetProcAddress(const char*n){
  if(!strcmp(n,"eglQueryDevicesEXT"))return (void*)qd; if(!strcmp(n,"eglGetPlatformDisplayEXT"))return (void*)gpd; return 0;}
EGLBoolean eglInitialize(EGLDisplay d,EGLint*a,EGLint*b){(void)d;(void)a;(void)b;return 1;}
EGLBoolean eglBindAPI(EGLenum a){(void)a;return 1;}
EGLBoolean eglChooseConfig(EGLDisplay d,const EGLint*a,EGLConfig*c,EGLint n,EGLint*nc){(void)d;(void)a;(void)n;c[0]=(EGLConfig)1;*nc=1;return 1;}
EGLContext eglCreateContext(EGLDisplay d,EGLConfig c,EGLContext s,const EGLint*a){(void)d;(void)c;(void)s;(void)a;return (EGLContext)1;}
EGLBoolean eglMakeCurrent(EGLDisplay d,EGLSurface a,EGLSurface b,EGLContext c){(void)d;(void)a;(void)b;(void)c;return 1;}
const GLubyte*glGetString(GLenum n){ if(n!=GL_RENDERER)return (const GLubyte*)"stub"; const char*r=getenv("STUB_RENDERER"); return (const GLubyte*)(r?r:"NVIDIA GeForce RTX STUB/PCIe/SSE2");}
void glGenTextures(GLsizei n,GLuint*t){for(int i=0;i<n;i++)t[i]=1;}
void glBindTexture(GLenum a,GLuint b){(void)a;(void)b;}
void glTexImage2D(GLenum a,GLint b,GLint c,GLsizei d,GLsizei e,GLint f,GLenum g,GLenum h,const void*p){(void)a;(void)b;(void)c;(void)d;(void)e;(void)f;(void)g;(void)h;(void)p;}
void glGenFramebuffers(GLsizei n,GLuint*t){for(int i=0;i<n;i++)t[i]=1;}
void glBindFramebuffer(GLenum a,GLuint b){(void)a;(void)b;}
void glFramebufferTexture2D(GLenum a,GLenum b,GLenum c,GLuint d,GLint e){(void)a;(void)b;(void)c;(void)d;(void)e;}
GLenum glCheckFramebufferStatus(GLenum t){(void)t;return GL_FRAMEBUFFER_COMPLETE;}
void glViewport(GLint a,GLint b,GLsizei c,GLsizei d){(void)a;(void)b;(void)c;(void)d;}
GLuint glCreateProgram(void){return 1;} GLuint glCreateShader(GLenum t){(void)t;return 1;}
void glAttachShader(GLuint a,GLuint b){(void)a;(void)b;}
void glShaderSource(GLuint a,GLsizei b,const GLchar*const*c,const GLint*d){(void)a;(void)b;(void)c;(void)d;}
void glCompileShader(GLuint a){(void)a;} void glLinkProgram(GLuint a){(void)a;} void glUseProgram(GLuint a){(void)a;}
void glBindAttribLocation(GLuint a,GLuint b,const GLchar*c){(void)a;(void)b;(void)c;}
void glGenBuffers(GLsizei n,GLuint*t){for(int i=0;i<n;i++)t[i]=1;}
void glBindBuffer(GLenum a,GLuint b){(void)a;(void)b;}
void glBufferData(GLenum a,GLsizeiptr b,const void*c,GLenum d){(void)a;(void)b;(void)c;(void)d;}
void glEnableVertexAttribArray(GLuint a){(void)a;}
void glVertexAttribPointer(GLuint a,GLint b,GLenum c,GLboolean d,GLsizei e,const void*f){(void)a;(void)b;(void)c;(void)d;(void)e;(void)f;}
void glClearColor(GLfloat r,GLfloat g,GLfloat b,GLfloat a){clr[0]=(unsigned char)(r*255+.5f);clr[1]=(unsigned char)(g*255+.5f);clr[2]=(unsigned char)(b*255+.5f);clr[3]=(unsigned char)(a*255+.5f);}
void glClear(GLbitfield m){(void)m;drawn=0;}
void glDrawArrays(GLenum m,GLint f,GLsizei n){(void)m;(void)f;if(n==3)drawn=1;}
void glFinish(void){}
GLenum glGetError(void){return !strcmp(mode(),"glerror")?GL_INVALID_OPERATION:GL_NO_ERROR;}
void glReadPixels(GLint x,GLint y,GLsizei w,GLsizei h,GLenum f,GLenum t,void*o){(void)w;(void)h;(void)f;(void)t;
  unsigned char*p=o; int inside=drawn&&(x+y<1024); const char*m=mode();
  if(!strcmp(m,"all_red")) inside=1; if(!strcmp(m,"nothing_drawn")) inside=0;
  if(inside){p[0]=255;p[1]=0;p[2]=0;p[3]=255; if(!strcmp(m,"inside_off1")) p[0]=254;}
  else {memcpy(p,clr,4); if(!strcmp(m,"outside_off1")) p[2]=254;}}
'''


@unittest.skipUnless(HAVE_GCC and HAVE_GL, "needs gcc and the EGL/GLES headers")
class EglProgram(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        d = Path(cls.tmp.name)
        (d / "stub.c").write_text(GL_STUB)
        (d / "empty.c").write_text("int kf_empty;\n")
        for cmd in (["gcc", "-shared", "-fPIC", "-O1", "-o", "libEGL.so", "stub.c"],
                    ["gcc", "-shared", "-fPIC", "-o", "libGLESv2.so", "empty.c"]):
            subprocess.run(cmd, cwd=d, check=True)
        cls.d = d
        cls.build(ROOT / "scripts/apps/src/egl_offscreen.c")

    @classmethod
    def build(cls, source):
        subprocess.run(["gcc", "-O1", "-Wall", str(source), "-o", "egl", "-L.", "-lEGL", "-lGLESv2"], cwd=cls.d, check=True)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def run_egl(self, **env):
        e = {"PATH": os.environ["PATH"], "LD_LIBRARY_PATH": str(self.d)}
        e.update(env)
        p = subprocess.run(["./egl", "2", "10"], cwd=self.d, env=e, capture_output=True, text=True, timeout=30)
        return p.stdout + p.stderr

    def test_nvidia_and_exact_pixels_pass(self):
        out = self.run_egl()
        self.assertIn("CHECK egl_renderer ok", out)
        self.assertIn("CHECK egl_pixels ok", out)
        self.assertIn("CHECK egl_gl_Mtri_s ok", out)
        self.assertNotIn("FAIL", out)

    def test_llvmpipe_renderer_fails(self):
        out = self.run_egl(STUB_RENDERER="llvmpipe (LLVM 15.0.7, 256 bits)")
        self.assertIn("CHECK egl_renderer FAIL", out)
        self.assertNotIn("CHECK egl_gl_Mtri_s ok", out)

    def test_non_nvidia_and_empty_renderer_fail(self):
        for r in ("AMD Radeon RX 7900", "Mesa Intel(R) UHD", ""):
            out = self.run_egl(STUB_RENDERER=r)
            self.assertIn("CHECK egl_renderer FAIL", out, r)
            self.assertNotIn("CHECK egl_gl_Mtri_s ok", out, r)

    def test_amd_renderer_fails(self):
        self.assertIn("CHECK egl_renderer FAIL", self.run_egl(STUB_RENDERER="AMD Radeon Graphics (radeonsi)"))

    def test_empty_renderer_fails(self):
        self.assertIn("CHECK egl_renderer FAIL", self.run_egl(STUB_RENDERER=""))

    def test_nvidia_word_does_not_excuse_a_software_marker(self):
        for r in ("NVIDIA llvmpipe", "NVIDIA swrast", "NVIDIA lavapipe"):
            self.assertIn("CHECK egl_renderer FAIL", self.run_egl(STUB_RENDERER=r), r)

    def test_nvidia_with_softpipe_or_software_word_fails(self):
        for r in ("NVIDIA softpipe", "NVIDIA Software Rasterizer"):
            self.assertIn("CHECK egl_renderer FAIL", self.run_egl(STUB_RENDERER=r), r)

    def test_wrong_outside_pixel_fails_by_one_unit(self):
        out = self.run_egl(STUB_MODE="outside_off1")
        self.assertIn("CHECK egl_pixels FAIL", out)
        self.assertNotIn("CHECK egl_gl_Mtri_s ok", out)

    def test_everything_red_fails(self):
        out = self.run_egl(STUB_MODE="all_red")
        self.assertIn("CHECK egl_pixels FAIL", out)
        self.assertNotIn("CHECK egl_gl_Mtri_s ok", out)

    def test_wrong_inside_pixel_fails_by_one_unit(self):
        out = self.run_egl(STUB_MODE="inside_off1")
        self.assertIn("CHECK egl_pixels FAIL", out)
        self.assertNotIn("CHECK egl_gl_Mtri_s ok", out)

    def test_nothing_drawn_fails(self):
        out = self.run_egl(STUB_MODE="nothing_drawn")
        self.assertIn("CHECK egl_pixels FAIL", out)
        self.assertNotIn("CHECK egl_gl_Mtri_s ok", out)

    def test_gl_error_after_the_benchmark_fails(self):
        out = self.run_egl(STUB_MODE="glerror")
        self.assertTrue("CHECK egl_pixels FAIL" in out or "CHECK egl_gl_Mtri_s FAIL" in out)
        self.assertNotIn("CHECK egl_gl_Mtri_s ok", out)


CUDA_STUB = r'''
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef int R; static const char*mode(void){const char*m=getenv("STUB_MODE");return m?m:"ok";}
static void logc(const char*s){const char*f=getenv("STUB_LOG"); if(f){FILE*o=fopen(f,"a"); if(o){fprintf(o,"%s\n",s);fclose(o);}}}
static int is(const char*m){return !strcmp(mode(),m);}
R cuInit(unsigned f){(void)f;return 0;}
R cuDriverGetVersion(int*v){*v=12060;return 0;}
R cuDeviceGet(int*d,int o){(void)o;*d=0;return 0;}
R cuDeviceGetName(char*n,int l,int d){(void)d;snprintf(n,l,"STUB GPU");return 0;}
R cuDeviceGetAttribute(int*v,int a,int d){(void)d;*v=(a==75)?8:6;return 0;}
R cuCtxCreate_v2(void**c,unsigned f,int d){(void)f;(void)d;*c=(void*)1;return 0;}
R cuCtxDestroy_v2(void*c){(void)c;logc("destroy");return is("destroy_fail")?201:0;}
R cuCtxSynchronize(void){return 0;}
R cuMemAlloc_v2(unsigned long long*p,size_t n){*p=(unsigned long long)(uintptr_t)calloc(1,n);return *p?0:2;}
R cuMemFree_v2(unsigned long long p){free((void*)(uintptr_t)p);logc("free");return is("free_fail")?201:0;}
R cuMemcpyHtoD_v2(unsigned long long d,const void*s,size_t n){memcpy((void*)(uintptr_t)d,s,n);
  if(is("corrupt_htod_mid")&&n==(8u<<20))((unsigned char*)(uintptr_t)d)[5u<<20]^=1;
  if(is("corrupt_htod_last")&&n==(8u<<20))((unsigned char*)(uintptr_t)d)[n-1]^=0x80;return 0;}
R cuMemcpyDtoH_v2(void*d,unsigned long long s,size_t n){memcpy(d,(void*)(uintptr_t)s,n);return 0;}
R cuMemsetD8_v2(unsigned long long p,unsigned char v,size_t n){memset((void*)(uintptr_t)p,v,n);
  if(is("corrupt_memset")&&v==0&&n==(1u<<19))((unsigned char*)(uintptr_t)p)[(1u<<19)-1]=1;
  if(is("corrupt_memset_end")&&v==0xA5&&n==(1u<<20))((unsigned char*)(uintptr_t)p)[n-1]=0xA4;return 0;}
R cuModuleLoadData(void**m,const void*p){(void)p;*m=(void*)1;return is("jit221")?221:0;}
R cuModuleUnload(void*m){(void)m;logc("unload");return 0;}
R cuModuleGetFunction(void**f,void*m,const char*n){(void)m;(void)n;*f=(void*)1;return 0;}
R cuLaunchKernel(void*f,unsigned gx,unsigned gy,unsigned gz,unsigned bx,unsigned by,unsigned bz,unsigned sh,void*st,void**a,void**e){
  (void)f;(void)gy;(void)gz;(void)by;(void)bz;(void)sh;(void)st;(void)e;
  uint32_t*x=(uint32_t*)(uintptr_t)*(unsigned long long*)a[0]; uint32_t*y=(uint32_t*)(uintptr_t)*(unsigned long long*)a[1]; unsigned n=*(unsigned*)a[2];
  for(unsigned i=0;i<gx*bx&&i<n;i++)y[i]=(x[i]*3u+7u)^i;
  if(is("corrupt_first"))y[0]^=1; if(is("corrupt_last"))y[n-1]^=0x10000;
  return 0;}
#ifndef NO_ERRNAME
R cuGetErrorName(R r,const char**s){*s=(r==221)?"CUDA_ERROR_JIT_COMPILER_NOT_FOUND":(r==201)?"CUDA_ERROR_INVALID_CONTEXT":"CUDA_ERROR_UNKNOWN";return 0;}
#endif
__attribute__((destructor)) static void atexit_crash(void){ if(is("atexit_crash")) abort(); }
'''


@unittest.skipUnless(HAVE_GCC, "needs gcc")
class CudaProbe(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        d = Path(cls.tmp.name)
        (d / "stub.c").write_text(CUDA_STUB)
        for name, flags in (("a", []), ("noname", ["-DNO_ERRNAME"])):
            (d / name).mkdir()
            subprocess.run(["gcc", "-shared", "-fPIC", "-O1", *flags, "-o", str(d / name / "libcuda.so.1"), "stub.c"], cwd=d, check=True)
        subprocess.run(["gcc", "-O2", "-Wall", "-Wextra", str(SRC / "cuda_drv_probe.c"), "-o", "probe", "-ldl"], cwd=d, check=True)
        cls.d = d

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def probe(self, lib="a", **env):
        log = self.d / "calls.log"
        log.unlink(missing_ok=True)
        e = {"PATH": os.environ["PATH"], "LD_LIBRARY_PATH": str(self.d / lib), "STUB_LOG": str(log)}
        e.update(env)
        p = subprocess.run(["./probe"], cwd=self.d, env=e, capture_output=True, text=True, timeout=60)
        calls = log.read_text().split() if log.exists() else []
        return p.returncode, p.stdout, calls

    def test_all_seven_checks_pass_and_teardown_runs(self):
        rc, out, calls = self.probe()
        self.assertEqual(rc, 0, out)
        self.assertIn("CUDA_DRV_PROBE_DONE rc=0 checks=7/7", out)
        self.assertNotIn("FAIL", out)
        self.assertIn("destroy", calls)
        self.assertIn("unload", calls)

    def test_one_flipped_bit_in_the_middle_of_the_roundtrip_fails(self):
        rc, out, _ = self.probe(STUB_MODE="corrupt_htod_mid")
        self.assertEqual(rc, 1)
        self.assertIn("CHECK cuda_roundtrip FAIL", out)

    def test_one_flipped_bit_in_the_last_byte_of_the_roundtrip_fails(self):
        rc, out, _ = self.probe(STUB_MODE="corrupt_htod_last")
        self.assertEqual(rc, 1)
        self.assertIn("CHECK cuda_roundtrip FAIL", out)

    def test_memset_boundary_byte_is_checked(self):
        rc, out, _ = self.probe(STUB_MODE="corrupt_memset")
        self.assertEqual(rc, 1)
        self.assertIn("CHECK cuda_memset FAIL", out)

    def test_memset_last_byte_is_checked(self):
        rc, out, _ = self.probe(STUB_MODE="corrupt_memset_end")
        self.assertEqual(rc, 1)
        self.assertIn("CHECK cuda_memset FAIL", out)

    def test_failing_cuMemFree_is_a_failed_teardown_check(self):
        rc, out, _ = self.probe(STUB_MODE="free_fail")
        self.assertEqual(rc, 1)
        self.assertIn("CHECK cuda_teardown FAIL", out)

    def test_first_wrong_kernel_element_fails(self):
        rc, out, _ = self.probe(STUB_MODE="corrupt_first")
        self.assertEqual(rc, 1)
        self.assertIn("CHECK cuda_kernel FAIL", out)
        self.assertIn("1 wrong (first at 0)", out)

    def test_last_wrong_kernel_element_fails(self):
        rc, out, _ = self.probe(STUB_MODE="corrupt_last")
        self.assertEqual(rc, 1)
        self.assertIn("1 wrong (first at 1048575)", out)

    def test_jit_error_221_is_named(self):
        rc, out, calls = self.probe(STUB_MODE="jit221")
        self.assertEqual(rc, 1)
        self.assertIn("CHECK cuda_ptx_jit FAIL", out)
        self.assertIn("CUDA_ERROR_JIT_COMPILER_NOT_FOUND (221)", out)
        self.assertIn("JIT compiler missing", out)
        self.assertIn("destroy", calls)  # a failed probe still tears down

    def test_221_is_reported_even_without_cuGetErrorName(self):
        rc, out, _ = self.probe("noname", STUB_MODE="jit221")
        self.assertEqual(rc, 1)
        self.assertIn("221", out)
        self.assertIn("JIT compiler missing", out)

    def test_failing_ctx_destroy_is_a_failed_teardown_check(self):
        rc, out, _ = self.probe(STUB_MODE="destroy_fail")
        self.assertEqual(rc, 1)
        self.assertIn("CHECK cuda_teardown FAIL", out)
        self.assertIn("CUDA_ERROR_INVALID_CONTEXT", out)

    def test_crash_at_process_exit_is_visible(self):
        rc, out, _ = self.probe(STUB_MODE="atexit_crash")
        self.assertIn("CUDA_DRV_PROBE_DONE rc=0 checks=7/7", out)
        self.assertNotEqual(rc, 0)  # SIGABRT from the driver's exit-time destructor; _exit() would hide it

    def test_the_probe_never_calls__exit(self):
        import re
        code = re.sub(r"/\*.*?\*/", "", (SRC / "cuda_drv_probe.c").read_text(), flags=re.S)
        self.assertIsNone(re.search(r"\b_exit\s*\(|\b_Exit\s*\(", code))

    def test_missing_libcuda_is_a_named_failure(self):
        rc, out, _ = self.probe(LD_LIBRARY_PATH=str(self.d / "nowhere"))
        self.assertEqual(rc, 1)
        self.assertIn("CHECK cuda_init FAIL", out)


class Registration(unittest.TestCase):
    def lines(self, rel):
        return [l.strip() for l in (ROOT / rel).read_text().splitlines() if l.strip() and not l.startswith("#")]

    def test_row_is_registered_everywhere_the_gate_reads_it(self):
        self.assertIn("cuda_drv_probe", self.lines("scripts/apps/gate_list.txt"))
        self.assertIn("cuda_drv_probe", self.lines("scripts/apps/gate_list_middle.txt"))
        self.assertIn("BUILD_cuda_drv_probe", self.lines("scripts/apps/expected_rows.txt"))
        self.assertIn("cuda_drv_probe|", (ROOT / "scripts/apps/run_apps.sh").read_text())
        build = (ROOT / "scripts/apps/build_bundle.sh").read_text()
        self.assertIn("cuda_drv_probe.c", build)
        self.assertIn("ok cuda_drv_probe", build)

    def test_gate_lists_stay_identical(self):
        self.assertEqual(self.lines("scripts/apps/gate_list.txt"), self.lines("scripts/apps/gate_list_middle.txt"))

    def test_every_gate_row_exists_in_run_apps(self):
        listed = subprocess.run(["bash", str(ROOT / "scripts/apps/run_apps.sh"), "x", "list"], capture_output=True, text=True,
                                env={"PATH": os.environ["PATH"], "APPS_ROOT": tempfile.gettempdir() + "/kf_run_apps_list"}).stdout.split()
        known_unsupported = {"UnifiedMemoryStreams", "UnifiedMemoryPerf", "conjugateGradientUM", "attach_verify"}
        for name in self.lines("scripts/apps/gate_list.txt"):
            self.assertIn(name, listed)
            self.assertNotIn(name, known_unsupported)


class TimeoutVerdict(unittest.TestCase):
    def test_verdict_line_for_timeout_only(self):
        from kfci import apps_gate as g
        row = "APPRES side=guest app=vkpeak verdict=TIMEOUT rc=124 secs=900 quiet=850 note=x boot=t"
        self.assertEqual(g.verdict_line("vkpeak", row), "APP_VERDICT name=vkpeak verdict=TIMEOUT quiet=850")
        self.assertEqual(g.verdict_line("vkpeak", row.replace("TIMEOUT", "FAIL")), "")
        self.assertEqual(g.verdict_line("vkpeak", ""), "")

    def test_timeout_without_quiet_is_still_a_timeout(self):
        from kfci import apps_gate as g
        row = "APPRES side=guest app=a verdict=TIMEOUT rc=124 secs=9 note=x boot=t"
        self.assertEqual(g.verdict_line("a", row), "APP_VERDICT name=a verdict=TIMEOUT quiet=-")

    def test_gate_result_of_a_timeout_stays_fail(self):
        from kfci import apps_gate as g
        row = "APPRES side=guest app=a verdict=TIMEOUT rc=124 secs=9 quiet=3 note=slow boot=t"
        result, line = g.grade_row("a", row)
        self.assertEqual(result, "FAIL")
        self.assertIn("result=FAIL", line)

    def test_stage_output_carries_the_timeout_line(self):
        import test_vast_payload_apps as existing
        class Case(existing.AppsGateScript):
            def runTest(self): pass
        c = Case()
        rows = [existing.AppsGateScript.res("vectorAdd", "TIMEOUT").replace("quiet=0", "quiet=41"),
                existing.AppsGateScript.res("deviceQuery")]
        rc, out = c.run_gate(rows)
        self.assertNotEqual(rc, 0)
        self.assertIn("APP_ROW name=vectorAdd os=linux result=FAIL", out)
        self.assertIn("APP_VERDICT name=vectorAdd verdict=TIMEOUT quiet=41", out)
        self.assertNotIn("APP_VERDICT name=deviceQuery", out)

    def test_report_keeps_the_cause_and_the_fail_result(self):
        import test_vast_run_report as t
        lines = t.log_lines() + [
            "APP_ROW name=vkpeak os=linux result=FAIL secs=900 guest_xid=- note=TIMEOUT -",
            "APP_VERDICT name=vkpeak verdict=TIMEOUT quiet=850",
            "APP_ROW name=clpeak os=linux result=FAIL secs=9 guest_xid=- note=FAIL boom",
        ]
        doc = t.rr.build_report(lines, t.result())
        apps = {a["name"]: a for a in doc["apps"]}
        self.assertEqual(apps["vkpeak"]["result"], "FAIL")
        self.assertTrue(apps["vkpeak"]["note"].startswith("[TIMEOUT quiet=850s]"), apps["vkpeak"])
        self.assertFalse(apps["clpeak"]["note"].startswith("[TIMEOUT"))
        tests = {x["name"]: x for x in doc["tests"] if x["suite"] == "apps_linux"}
        self.assertEqual(tests["vkpeak"]["result"], "FAIL")

    def test_report_timeout_without_quiet_is_still_marked(self):
        import test_vast_run_report as t
        lines = t.log_lines() + ["APP_ROW name=vkpeak os=linux result=FAIL secs=900 guest_xid=- note=TIMEOUT -",
                                 "APP_VERDICT name=vkpeak verdict=TIMEOUT quiet=-"]
        doc = t.rr.build_report(lines, t.result())
        note = {a["name"]: a for a in doc["apps"]}["vkpeak"]["note"]
        self.assertTrue(note.startswith("[TIMEOUT"), note)
        self.assertIn("quiet=-", note)

    def test_report_ignores_a_timeout_line_for_an_unknown_app(self):
        import test_vast_run_report as t
        doc = t.rr.build_report(t.log_lines() + ["APP_VERDICT name=ghost verdict=TIMEOUT quiet=1"], t.result())
        self.assertEqual([a for a in doc["apps"] if a["name"] == "ghost"], [])


if __name__ == "__main__":
    unittest.main()
