#version 450

layout(location = 0) in vec3 inPos;
layout(location = 1) in vec3 inNormal;
layout(location = 2) in vec2 inUv;

layout(binding = 0) uniform UBO {
    mat4 viewProj;
    mat4 lightViewProj;
    vec4 sunDir;
    vec4 sunColor;
    vec4 ambient;
    vec4 outdoorAmbient;
    vec4 fogColor;
    vec4 fogParams;
    vec4 cameraPos;
    vec4 skyTop;
    vec4 skyBottom;
} ubo;

layout(push_constant) uniform Push {
    mat4 model;
    vec4 color;
    vec4 params;
} push;

void main() {
    gl_Position = ubo.lightViewProj * push.model * vec4(inPos, 1.0);
}
