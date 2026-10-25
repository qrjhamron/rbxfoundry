#version 450

layout(location = 0) in vec2 inPos;   // pixel coordinates
layout(location = 1) in vec2 inUv;
layout(location = 2) in vec4 inColor; // straight-alpha RGBA
layout(location = 3) in vec4 inClip;  // xywh pixels
layout(location = 4) in vec2 inLocal; // pixel offset inside the quad
layout(location = 5) in vec3 inMisc;  // x corner radius, yz quad size

layout(binding = 0) uniform UiUBO {
    vec2 viewport; // pixels
    vec2 unused;
} ubo;

layout(location = 0) out vec2 vUv;
layout(location = 1) out vec4 vColor;
layout(location = 2) out vec4 vClip;
layout(location = 3) out vec2 vLocal;
layout(location = 4) out vec3 vMisc;

void main() {
    vUv = inUv;
    vColor = inColor;
    vClip = inClip;
    vLocal = inLocal;
    vMisc = inMisc;
    vec2 ndc = vec2(inPos.x / ubo.viewport.x * 2.0 - 1.0, inPos.y / ubo.viewport.y * 2.0 - 1.0);
    gl_Position = vec4(ndc, 0.0, 1.0);
}
