#version 450

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

layout(location = 0) in vec2 vUv;
layout(location = 0) out vec4 outColor;

vec3 aces(vec3 x) {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0);
}

void main() {
    vec3 c = mix(ubo.skyBottom.rgb, ubo.skyTop.rgb, smoothstep(0.42, 0.85, vUv.y));
    c = aces(c * ubo.fogParams.z);
    outColor = vec4(pow(c, vec3(1.0 / 2.2)), 1.0);
}
