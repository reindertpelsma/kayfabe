/* egl_offscreen.c — headless OpenGL render benchmark through the DRI path.
 *
 * Uses the NVIDIA EGL "device platform" (EGL_EXT_platform_device) so it needs
 * NO X server / Wayland — it binds the GPU directly and renders to an FBO.
 * This exercises the real GL driver (libEGL/libGLESv2 -> nvidia) over nvkvm's
 * forwarded /dev/nvidia* + /dev/dri/renderD128, the same path graphics apps use.
 *
 * Renders many gouraud triangles into a 1024x1024 FBO for N frames. Correctness (2026-11-12; the old
 * check was "no GL error", which a software rasterizer also passes):
 *   1. the renderer string names NVIDIA and no software rasterizer (llvmpipe, swrast, lavapipe, softpipe);
 *   2. an exact-pixel scene, drawn before the benchmark: clear to (0,0,1,1), one flat (1,0,0) triangle over
 *      the lower-left half. Colour components 0 and 1 are exact in every GL pixel format, so the pixels are
 *      derived, not captured: inside (W/4,W/4) = 255,0,0,255; outside (3W/4,3W/4) = 0,0,255,255;
 *   3. no GL error after the benchmark.
 * Emits: METRIC egl_gl_Mtri_s <v> ; RENDERER <string> ; CHECK egl_renderer ; CHECK egl_pixels ;
 *        CHECK egl_gl_Mtri_s <ok|FAIL>
 *
 * Build: gcc egl_offscreen.c -o egl_offscreen -lEGL -lGLESv2
 */
#define _GNU_SOURCE /* strcasestr */
#define EGL_EGLEXT_PROTOTYPES
#define GL_GLEXT_PROTOTYPES
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <time.h>
static double now(){struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return t.tv_sec+t.tv_nsec/1e9;}
#define W 1024
static const char* VS=
 "attribute vec2 p;attribute vec3 c;varying vec3 v;\n"
 "void main(){v=c;gl_Position=vec4(p,0.0,1.0);}\n";
static const char* FS=
 "precision mediump float;varying vec3 v;\n"
 "void main(){gl_FragColor=vec4(v,1.0);}\n";
static GLuint sh(GLenum t,const char*s){GLuint o=glCreateShader(t);glShaderSource(o,1,&s,0);glCompileShader(o);return o;}

int main(int argc,char**argv){
    int frames = argc>1?atoi(argv[1]):2000;
    int tris   = argc>2?atoi(argv[2]):20000;   /* triangles per frame */
    /* ---- EGL device platform (no window system) ---- */
    PFNEGLQUERYDEVICESEXTPROC qd=(void*)eglGetProcAddress("eglQueryDevicesEXT");
    PFNEGLGETPLATFORMDISPLAYEXTPROC gpd=(void*)eglGetProcAddress("eglGetPlatformDisplayEXT");
    if(!qd||!gpd){printf("CHECK egl_gl_Mtri_s FAIL\n[egl] no EGL_EXT_platform_device\n");return 0;}
    EGLDeviceEXT devs[8]; EGLint nd=0;
    if(!qd(8,devs,&nd)||nd<1){printf("CHECK egl_gl_Mtri_s FAIL\n[egl] no EGL devices\n");return 0;}
    EGLDisplay dpy=gpd(EGL_PLATFORM_DEVICE_EXT,devs[0],0);
    if(dpy==EGL_NO_DISPLAY||!eglInitialize(dpy,0,0)){printf("CHECK egl_gl_Mtri_s FAIL\n[egl] eglInitialize failed\n");return 0;}
    eglBindAPI(EGL_OPENGL_ES_API);
    EGLint cfga[]={EGL_SURFACE_TYPE,EGL_PBUFFER_BIT,EGL_RENDERABLE_TYPE,EGL_OPENGL_ES2_BIT,EGL_RED_SIZE,8,EGL_NONE};
    EGLConfig cfg; EGLint nc;
    if(!eglChooseConfig(dpy,cfga,&cfg,1,&nc)||nc<1){printf("CHECK egl_gl_Mtri_s FAIL\n[egl] no config\n");return 0;}
    EGLint ctxa[]={EGL_CONTEXT_CLIENT_VERSION,2,EGL_NONE};
    EGLContext ctx=eglCreateContext(dpy,cfg,EGL_NO_CONTEXT,ctxa);
    if(ctx==EGL_NO_CONTEXT){printf("CHECK egl_gl_Mtri_s FAIL\n[egl] no context\n");return 0;}
    /* surfaceless: render straight to an FBO */
    if(!eglMakeCurrent(dpy,EGL_NO_SURFACE,EGL_NO_SURFACE,ctx)){printf("CHECK egl_gl_Mtri_s FAIL\n[egl] makeCurrent (no surfaceless ext?)\n");return 0;}
    const char*rend=(const char*)glGetString(GL_RENDERER); if(!rend) rend="";
    fprintf(stderr,"[egl] renderer: %s\n",rend);
    printf("RENDERER %s\n",rend);
    {
        static const char*soft[]={"llvmpipe","swrast","lavapipe","softpipe","software","swiftshader",0};
        int bad=!strcasestr(rend,"NVIDIA");
        for(int i=0;soft[i];i++) if(strcasestr(rend,soft[i])) bad=1;
        printf("CHECK egl_renderer %s\n",bad?"FAIL":"ok");
        if(bad){printf("[egl] renderer is not the NVIDIA GPU: '%s'\n",rend);return 0;}
    }

    GLuint tex; glGenTextures(1,&tex); glBindTexture(GL_TEXTURE_2D,tex);
    glTexImage2D(GL_TEXTURE_2D,0,GL_RGBA,W,W,0,GL_RGBA,GL_UNSIGNED_BYTE,0);
    GLuint fbo; glGenFramebuffers(1,&fbo); glBindFramebuffer(GL_FRAMEBUFFER,fbo);
    glFramebufferTexture2D(GL_FRAMEBUFFER,GL_COLOR_ATTACHMENT0,GL_TEXTURE_2D,tex,0);
    if(glCheckFramebufferStatus(GL_FRAMEBUFFER)!=GL_FRAMEBUFFER_COMPLETE){printf("CHECK egl_gl_Mtri_s FAIL\n[egl] FBO incomplete\n");return 0;}
    glViewport(0,0,W,W);

    GLuint pr=glCreateProgram(); glAttachShader(pr,sh(GL_VERTEX_SHADER,VS)); glAttachShader(pr,sh(GL_FRAGMENT_SHADER,FS));
    glBindAttribLocation(pr,0,"p"); glBindAttribLocation(pr,1,"c"); glLinkProgram(pr); glUseProgram(pr);

    int nv=tris*3; float*vd=malloc(nv*5*sizeof(float));
    for(int i=0;i<nv;i++){vd[i*5+0]=(float)((i*131)%2000)/1000.0f-1.0f; vd[i*5+1]=(float)((i*977)%2000)/1000.0f-1.0f;
        vd[i*5+2]=(i%3==0); vd[i*5+3]=(i%3==1); vd[i*5+4]=(i%3==2);}
    GLuint vbo; glGenBuffers(1,&vbo); glBindBuffer(GL_ARRAY_BUFFER,vbo);
    glBufferData(GL_ARRAY_BUFFER,nv*5*sizeof(float),vd,GL_STATIC_DRAW);
    glEnableVertexAttribArray(0); glVertexAttribPointer(0,2,GL_FLOAT,0,5*sizeof(float),0);
    glEnableVertexAttribArray(1); glVertexAttribPointer(1,3,GL_FLOAT,0,5*sizeof(float),(void*)(2*sizeof(float)));

    /* exact-pixel scene (see the header): flat red triangle over the lower-left half on a blue clear */
    {
        static const float tri[3][5]={{-1,-1,1,0,0},{1,-1,1,0,0},{-1,1,1,0,0}};
        GLuint tb; glGenBuffers(1,&tb); glBindBuffer(GL_ARRAY_BUFFER,tb);
        glBufferData(GL_ARRAY_BUFFER,sizeof tri,tri,GL_STATIC_DRAW);
        glEnableVertexAttribArray(0); glVertexAttribPointer(0,2,GL_FLOAT,0,5*sizeof(float),0);
        glEnableVertexAttribArray(1); glVertexAttribPointer(1,3,GL_FLOAT,0,5*sizeof(float),(void*)(2*sizeof(float)));
        glClearColor(0.0f,0.0f,1.0f,1.0f); glClear(GL_COLOR_BUFFER_BIT);
        glDrawArrays(GL_TRIANGLES,0,3); glFinish();
        unsigned char in[4]={9,9,9,9}, out[4]={9,9,9,9};
        glReadPixels(W/4,W/4,1,1,GL_RGBA,GL_UNSIGNED_BYTE,in);
        glReadPixels(3*W/4,3*W/4,1,1,GL_RGBA,GL_UNSIGNED_BYTE,out);
        static const unsigned char ein[4]={255,0,0,255}, eout[4]={0,0,255,255};
        int okp=!memcmp(in,ein,4)&&!memcmp(out,eout,4)&&glGetError()==GL_NO_ERROR;
        printf("PIXELS inside=%u,%u,%u,%u outside=%u,%u,%u,%u expected inside=255,0,0,255 outside=0,0,255,255\n",
               in[0],in[1],in[2],in[3],out[0],out[1],out[2],out[3]);
        printf("CHECK egl_pixels %s\n",okp?"ok":"FAIL");
        if(!okp) return 0;
        glBindBuffer(GL_ARRAY_BUFFER,vbo);
        glVertexAttribPointer(0,2,GL_FLOAT,0,5*sizeof(float),0);
        glVertexAttribPointer(1,3,GL_FLOAT,0,5*sizeof(float),(void*)(2*sizeof(float)));
    }
    glClearColor(0.05f,0.10f,0.20f,1.0f);
    glClear(GL_COLOR_BUFFER_BIT); glDrawArrays(GL_TRIANGLES,0,nv); glFinish(); /* warm */
    double t=now();
    for(int f=0;f<frames;f++){ glClear(GL_COLOR_BUFFER_BIT); glDrawArrays(GL_TRIANGLES,0,nv); }
    glFinish(); t=now()-t;
    /* The renderer and exact-pixel checks ran above; this one is the benchmark's own GL error state. */
    GLenum gerr = glGetError();
    printf("METRIC egl_gl_Mtri_s %.1f\n",(double)tris*frames/t/1e6);
    printf("CHECK egl_gl_Mtri_s %s\n",(gerr==GL_NO_ERROR)?"ok":"FAIL");
    return 0;
}
