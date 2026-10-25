#version 450

layout(location = 0) out vec2 vUv;

void main() {
    // Fullscreen triangle.
    vec2 pos = vec2(float((gl_VertexIndex << 1) & 2), float(gl_VertexIndex & 2));
    vUv = pos;
    gl_Position = vec4(pos * 2.0 - 1.0, 0.9999, 1.0);
}
