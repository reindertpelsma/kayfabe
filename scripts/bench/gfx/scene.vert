#version 450
// Three interpenetrating triangles: A and B cross in depth, C pokes through both.
// Same geometry as egl_render.c (GL maps z to 2z-1; ordering is identical).
layout(location = 0) out vec3 col;
const vec3 P[9] = vec3[](
    vec3(-0.9, -0.8, 0.2), vec3( 0.9, -0.6, 0.8), vec3( 0.0,  0.9, 0.5),
    vec3(-0.9, -0.6, 0.8), vec3( 0.9, -0.8, 0.2), vec3( 0.0,  0.8, 0.5),
    vec3(-0.3, -0.3, 0.4), vec3( 0.3, -0.3, 0.6), vec3( 0.0,  0.4, 0.1));
const vec3 C[3] = vec3[](vec3(1.0, 0.2, 0.1), vec3(0.1, 1.0, 0.2), vec3(0.2, 0.3, 1.0));
void main() {
    vec3 p = P[gl_VertexIndex];
    gl_Position = vec4(p, 1.0);
    col = C[gl_VertexIndex / 3] * (0.6 + 0.4 * p.z);
}
