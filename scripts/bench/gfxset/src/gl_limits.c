// gl_limits — the GL implementation as the application sees it, on the EGL device platform (no X):
// renderer/version strings, every extension, and ~70 implementation limits. The caller digests the
// EXT/LIM line sets and compares them with bare metal — a guest whose RM answers differ from the
// host's (a refused control, a different engine/class set) shows up here before any render does.
//   build: gcc -O2 gl_limits.c -o gl_limits -lEGL -lOpenGL
#define EGL_EGLEXT_PROTOTYPES
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GL/gl.h>
#include <GL/glext.h>
#include <stdio.h>
#include <string.h>

typedef const GLubyte *(*PFNGLGETSTRINGI)(GLenum, GLuint);
typedef void (*PFNGETI64)(GLenum, GLint64 *);
typedef void (*PFNGETII)(GLenum, GLuint, GLint *);
#define L(x) { #x, x }
static const struct { const char *n; GLenum e; } LIMS[] = {
    L(GL_MAX_TEXTURE_SIZE), L(GL_MAX_3D_TEXTURE_SIZE), L(GL_MAX_CUBE_MAP_TEXTURE_SIZE), L(GL_MAX_ARRAY_TEXTURE_LAYERS),
    L(GL_MAX_RENDERBUFFER_SIZE), L(GL_MAX_SAMPLES), L(GL_MAX_COLOR_TEXTURE_SAMPLES), L(GL_MAX_DEPTH_TEXTURE_SAMPLES),
    L(GL_MAX_INTEGER_SAMPLES), L(GL_MAX_COLOR_ATTACHMENTS), L(GL_MAX_DRAW_BUFFERS), L(GL_MAX_FRAMEBUFFER_WIDTH),
    L(GL_MAX_FRAMEBUFFER_HEIGHT), L(GL_MAX_FRAMEBUFFER_LAYERS), L(GL_MAX_FRAMEBUFFER_SAMPLES), L(GL_MAX_VIEWPORTS),
    L(GL_MAX_VERTEX_ATTRIBS), L(GL_MAX_VERTEX_UNIFORM_COMPONENTS), L(GL_MAX_VERTEX_UNIFORM_BLOCKS),
    L(GL_MAX_VERTEX_OUTPUT_COMPONENTS), L(GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS), L(GL_MAX_VERTEX_ATOMIC_COUNTERS),
    L(GL_MAX_VERTEX_SHADER_STORAGE_BLOCKS), L(GL_MAX_TESS_GEN_LEVEL), L(GL_MAX_PATCH_VERTICES),
    L(GL_MAX_TESS_CONTROL_UNIFORM_COMPONENTS), L(GL_MAX_TESS_EVALUATION_UNIFORM_COMPONENTS),
    L(GL_MAX_GEOMETRY_OUTPUT_VERTICES), L(GL_MAX_GEOMETRY_TOTAL_OUTPUT_COMPONENTS), L(GL_MAX_GEOMETRY_SHADER_INVOCATIONS),
    L(GL_MAX_GEOMETRY_UNIFORM_COMPONENTS), L(GL_MAX_FRAGMENT_UNIFORM_COMPONENTS), L(GL_MAX_FRAGMENT_UNIFORM_BLOCKS),
    L(GL_MAX_FRAGMENT_INPUT_COMPONENTS), L(GL_MAX_TEXTURE_IMAGE_UNITS), L(GL_MAX_FRAGMENT_ATOMIC_COUNTERS),
    L(GL_MAX_FRAGMENT_SHADER_STORAGE_BLOCKS), L(GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS), L(GL_MAX_COMBINED_UNIFORM_BLOCKS),
    L(GL_MAX_COMBINED_SHADER_STORAGE_BLOCKS), L(GL_MAX_COMBINED_ATOMIC_COUNTERS), L(GL_MAX_COMBINED_IMAGE_UNIFORMS),
    L(GL_MAX_UNIFORM_BUFFER_BINDINGS), L(GL_MAX_UNIFORM_BLOCK_SIZE), L(GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT),
    L(GL_MAX_SHADER_STORAGE_BUFFER_BINDINGS), L(GL_MAX_SHADER_STORAGE_BLOCK_SIZE),
    L(GL_SHADER_STORAGE_BUFFER_OFFSET_ALIGNMENT), L(GL_MAX_ATOMIC_COUNTER_BUFFER_BINDINGS), L(GL_MAX_IMAGE_UNITS),
    L(GL_MAX_COMPUTE_UNIFORM_BLOCKS), L(GL_MAX_COMPUTE_TEXTURE_IMAGE_UNITS), L(GL_MAX_COMPUTE_SHARED_MEMORY_SIZE),
    L(GL_MAX_COMPUTE_WORK_GROUP_INVOCATIONS), L(GL_MAX_COMPUTE_SHADER_STORAGE_BLOCKS),
    L(GL_MAX_TRANSFORM_FEEDBACK_BUFFERS), L(GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS),
    L(GL_MAX_ELEMENTS_VERTICES), L(GL_MAX_ELEMENTS_INDICES), L(GL_MAX_TEXTURE_BUFFER_SIZE),
    L(GL_MAX_RECTANGLE_TEXTURE_SIZE), L(GL_MAX_TEXTURE_LOD_BIAS), L(GL_MAX_SUBROUTINES), L(GL_MAX_CLIP_DISTANCES),
    L(GL_MAX_VARYING_COMPONENTS), L(GL_MAX_SAMPLE_MASK_WORDS), L(GL_MAX_SERVER_WAIT_TIMEOUT), L(GL_MAX_VERTEX_ATTRIB_BINDINGS),
    L(GL_MAX_VERTEX_ATTRIB_RELATIVE_OFFSET), L(GL_MAX_LABEL_LENGTH), L(GL_MAX_DEBUG_MESSAGE_LENGTH),
    L(GL_MAX_UNIFORM_LOCATIONS), L(GL_NUM_SHADING_LANGUAGE_VERSIONS), L(GL_NUM_PROGRAM_BINARY_FORMATS),
};

int main(void) {
    PFNEGLQUERYDEVICESEXTPROC qd = (void *)eglGetProcAddress("eglQueryDevicesEXT");
    PFNEGLGETPLATFORMDISPLAYEXTPROC gpd = (void *)eglGetProcAddress("eglGetPlatformDisplayEXT");
    PFNEGLQUERYDEVICESTRINGEXTPROC qs = (void *)eglGetProcAddress("eglQueryDeviceStringEXT");
    EGLDeviceEXT devs[8]; EGLint nd = 0;
    if (!qd || !gpd || !qd(8, devs, &nd) || nd < 1) { printf("GL_LIMITS_FAIL no EGL device\n"); return 3; }
    EGLDisplay d = EGL_NO_DISPLAY;
    for (int i = 0; i < nd && d == EGL_NO_DISPLAY; i++) {   // the NVIDIA device (not a software one)
        const char *ext = qs ? qs(devs[i], EGL_EXTENSIONS) : "";
        if (ext && strstr(ext, "EGL_NV_device_cuda")) d = gpd(EGL_PLATFORM_DEVICE_EXT, devs[i], NULL);
    }
    if (d == EGL_NO_DISPLAY) d = gpd(EGL_PLATFORM_DEVICE_EXT, devs[0], NULL);
    if (!eglInitialize(d, NULL, NULL)) { printf("GL_LIMITS_FAIL eglInitialize 0x%x\n", eglGetError()); return 3; }
    eglBindAPI(EGL_OPENGL_API);
    EGLint ca[] = { EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT, EGL_NONE };
    EGLConfig c; EGLint n;
    if (!eglChooseConfig(d, ca, &c, 1, &n) || n < 1) { printf("GL_LIMITS_FAIL no config\n"); return 3; }
    EGLint xa[] = { EGL_CONTEXT_MAJOR_VERSION, 4, EGL_CONTEXT_MINOR_VERSION, 6,
                    EGL_CONTEXT_OPENGL_PROFILE_MASK, EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT, EGL_NONE };
    EGLContext x = eglCreateContext(d, c, EGL_NO_CONTEXT, xa);
    if (x == EGL_NO_CONTEXT || !eglMakeCurrent(d, EGL_NO_SURFACE, EGL_NO_SURFACE, x)) {
        printf("GL_LIMITS_FAIL context 0x%x\n", eglGetError()); return 3;
    }
    printf("GL_RENDERER=%s\nGL_VERSION=%s\nGL_SLV=%s\n", glGetString(GL_RENDERER), glGetString(GL_VERSION),
           glGetString(GL_SHADING_LANGUAGE_VERSION));
    PFNGLGETSTRINGI gsi = (PFNGLGETSTRINGI)eglGetProcAddress("glGetStringi");
    PFNGETI64 gi64 = (PFNGETI64)eglGetProcAddress("glGetInteger64v");
    PFNGETII gii = (PFNGETII)eglGetProcAddress("glGetIntegeri_v");
    if (!gsi || !gi64 || !gii) { printf("GL_LIMITS_FAIL no GL 3.x/4.x entry points\n"); return 3; }
    GLint ne = 0; glGetIntegerv(GL_NUM_EXTENSIONS, &ne);
    for (GLint i = 0; i < ne; i++) printf("EXT %s\n", gsi(GL_EXTENSIONS, i));
    for (size_t i = 0; i < sizeof LIMS / sizeof LIMS[0]; i++) {
        GLint64 v[2] = { -1, -1 };
        gi64(LIMS[i].e, v);
        printf("LIM %s %lld\n", LIMS[i].n, (long long)v[0]);
    }
    GLint wg[3]; for (int i = 0; i < 3; i++) gii(GL_MAX_COMPUTE_WORK_GROUP_COUNT, i, &wg[i]);
    printf("LIM GL_MAX_COMPUTE_WORK_GROUP_COUNT %d,%d,%d\n", wg[0], wg[1], wg[2]);
    GLenum e = glGetError();
    printf("GL_LIMITS_DONE glerr=0x%x\n", e);
    return e ? 1 : 0;
}
