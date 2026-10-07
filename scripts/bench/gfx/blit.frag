#version 450
// Samples the block-linear (OPTIMAL) colour target of pass 1 through the texture unit,
// transposed and linearly filtered — exercises the texture path over a GENERIC-kind surface.
layout(set = 0, binding = 0) uniform sampler2D t;
layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 o;
void main() { o = texture(t, uv.yx * 0.97 + 0.01) * vec4(1.0, 0.5, 1.0, 1.0) + vec4(0.0, 0.0, 0.1, 0.0); }
