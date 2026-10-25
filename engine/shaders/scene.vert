#version 450

layout(location = 0) in vec3 inPos;
layout(location = 1) in vec3 inNormal;
layout(location = 2) in vec2 inUv;

layout(binding = 0) uniform UBO {
    mat4 viewProj;
    mat4 lightViewProj;
    vec4 sunDir;      // xyz direction TO the sun (negated travel direction), w unused
    vec4 sunColor;    // rgb linear, pre-scaled
    vec4 ambient;     // rgb linear
    vec4 outdoorAmbient; // rgb linear
    vec4 fogColor;    // rgb (sRGB-ish, converted like part colours)
    vec4 fogParams;   // x start, y end, z exposure, w shadowsEnabled
    vec4 cameraPos;   // xyz
    vec4 skyTop;      // rgb linear
    vec4 skyBottom;   // rgb linear
    vec4 dynPos[8];   // xyz position, w range
    vec4 dynColor[8]; // rgb linear (brightness-scaled), w coneCos (-2 = point)
    vec4 dynDir[8];   // xyz direction, w lightCount (x component of [0])
    vec4 camRight;    // xyz camera right (billboards)
    vec4 camUp;       // xyz camera up (billboards)
} ubo;

layout(push_constant) uniform Push {
    mat4 model;
    vec4 color;   // rgb linear, a = alpha (1 - transparency)
    vec4 params;  // x material id, y useTexture, z shadowReceive, w unused
} push;

layout(location = 0) out vec3 vWorldPos;
layout(location = 1) out vec3 vNormal;
layout(location = 2) out vec2 vUv;
layout(location = 3) out vec4 vColor;
layout(location = 4) out vec4 vParams;
layout(location = 5) out vec4 vLightSpace;

void main() {
    vec4 world;
    if (push.params.w > 7.5) { // billboard flag (bit 3 of params.w)
        // Billboard: expand local XY around the model translation,
        // scaled by the model X axis length.
        vec3 centre = vec3(push.model[3]);
        float scale = length(vec3(push.model[0]));
        world = vec4(centre + (ubo.camRight.xyz * inPos.x + ubo.camUp.xyz * inPos.y) * scale, 1.0);
    } else {
        world = push.model * vec4(inPos, 1.0);
    }
    vWorldPos = world.xyz;
    vNormal = normalize(mat3(push.model) * inNormal);
    vUv = inUv;
    vColor = push.color;
    vParams = push.params;
    vLightSpace = ubo.lightViewProj * world;
    gl_Position = ubo.viewProj * world;
}
