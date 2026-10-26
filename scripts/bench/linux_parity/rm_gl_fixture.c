#define GL_GLEXT_PROTOTYPES 1
#include <GL/gl.h>
#include <GL/glext.h>
#include <stdlib.h>
#include <string.h>
int main(int argc, char **argv) {
    if (argc != 3) return 2;
    int size = atoi(argv[2]);
    if (!strcmp(argv[1], "tex"))
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, size, size + 1, 0,
                     GL_RGBA, GL_UNSIGNED_BYTE, NULL);
    else if (!strcmp(argv[1], "depth"))
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT24, size, size + 1);
    else return 2;
    return 0;
}
