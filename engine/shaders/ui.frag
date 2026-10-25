#version 450

// One texture convention for both images and text: textures store RGBA;
// glyph atlases store white RGB with coverage in alpha. Tint multiplies.
layout(binding = 1) uniform sampler2D texSampler;

layout(location = 0) in vec2 vUv;
layout(location = 1) in vec4 vColor;
layout(location = 2) in vec4 vClip;
layout(location = 3) in vec2 vLocal;
layout(location = 4) in vec3 vMisc;

layout(location = 0) out vec4 outColor;

void main() {
    if (gl_FragCoord.x < vClip.x || gl_FragCoord.y < vClip.y ||
        gl_FragCoord.x >= vClip.x + vClip.z || gl_FragCoord.y >= vClip.y + vClip.w) {
        discard;
    }
    float radius = vMisc.x;
    if (radius > 0.0) {
        vec2 halfSize = vMisc.yz * 0.5;
        vec2 q = abs(vLocal - halfSize) - (halfSize - radius);
        float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
        if (d > 0.0) discard;
    }
    vec4 tex = texture(texSampler, vUv);
    outColor = vec4(vColor.rgb * tex.rgb, vColor.a * tex.a);
}
