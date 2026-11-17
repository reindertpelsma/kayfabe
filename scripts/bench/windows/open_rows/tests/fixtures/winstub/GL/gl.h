/* Declarations-only stand-in for the OpenGL 1.1 <GL/gl.h> (see ../windows.h). */
#ifndef KF_GLSTUB_H
#define KF_GLSTUB_H
typedef unsigned int GLenum, GLuint, GLbitfield;
typedef int GLint, GLsizei;
typedef float GLfloat;
typedef unsigned char GLubyte, GLboolean;
#define GL_FALSE 0
#define GL_FLOAT 0x1406
#define GL_TEXTURE_2D 0x0DE1
#define GL_TEXTURE_MIN_FILTER 0x2801
#define GL_RGBA 0x1908
#define GL_UNSIGNED_BYTE 0x1401
#define GL_COLOR_BUFFER_BIT 0x4000
#define GL_DEPTH_BUFFER_BIT 0x100
#define GL_DEPTH_TEST 0x0B71
#define GL_NEAREST 0x2600
#define GL_RENDERER 0x1F01
#define GL_VERSION 0x1F02
const GLubyte *glGetString(GLenum);
void glGenTextures(GLsizei, GLuint *);
void glBindTexture(GLenum, GLuint);
void glTexImage2D(GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, const void *);
void glTexParameteri(GLenum, GLenum, GLint);
void glViewport(GLint, GLint, GLsizei, GLsizei);
void glEnable(GLenum);
void glClearColor(GLfloat, GLfloat, GLfloat, GLfloat);
void glClear(GLbitfield);
void glDrawArrays(GLenum, GLint, GLsizei);
void glReadPixels(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, void *);
void glFinish(void);
GLenum glGetError(void);
#endif
