/* GPU-free stand-in for the two public storage entry points. */
#include <GL/gl.h>
#include <GL/glext.h>
#include <stdio.h>
void glTexImage2D(GLenum target, GLint level, GLint internalformat, GLsizei width,
                  GLsizei height, GLint border, GLenum format, GLenum type,
                  const void *pixels) {
    printf("MOCK_TEX %u %d %d %d %d %d %u %u %p\n", target, level,
           internalformat, width, height, border, format, type, pixels);
}
void glRenderbufferStorage(GLenum target, GLenum internalformat, GLsizei width,
                           GLsizei height) {
    printf("MOCK_DEPTH %u %u %d %d\n", target, internalformat, width, height);
}
