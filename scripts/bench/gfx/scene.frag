#version 450
layout(location = 0) in vec3 col;
layout(location = 0) out vec4 o;
void main() { o = vec4(col, 1.0); }
