#version 450

layout(binding = 0) uniform UBO {
    mat4 viewProj;
    mat4 lightViewProj;
    vec4 sunDir;
    vec4 sunColor;
    vec4 ambient;
    vec4 outdoorAmbient;
    vec4 fogColor;
    vec4 fogParams; // x start, y end, z exposure, w shadowsEnabled
    vec4 cameraPos;
    vec4 skyTop;
    vec4 skyBottom;
    vec4 dynPos[8];   // xyz position, w range
    vec4 dynColor[8]; // rgb linear (brightness-scaled), w coneCos (-2 = point)
    vec4 dynDir[8];   // xyz direction, w lightCount (in dynDir[0].w)
    vec4 camRight;
    vec4 camUp;
} ubo;

layout(binding = 1) uniform sampler2D shadowMap;
layout(binding = 2) uniform sampler2D texSampler;
layout(binding = 3) uniform sampler2D normalSampler;
layout(binding = 4) uniform sampler2D roughSampler;
layout(binding = 5) uniform sampler2D metalSampler;

layout(location = 0) in vec3 vWorldPos;
layout(location = 1) in vec3 vNormal;
layout(location = 2) in vec2 vUv;
layout(location = 3) in vec4 vColor;
layout(location = 4) in vec4 vParams;
layout(location = 5) in vec4 vLightSpace;

layout(location = 0) out vec4 outColor;

// Enum.Material values
const float MAT_NEON = 288.0;
const float MAT_GLASS = 1568.0;
const float MAT_FORCEFIELD = 1584.0;
const float MAT_WOOD = 512.0;
const float MAT_WOODPLANKS = 528.0;
const float MAT_GRASS = 1280.0;
const float MAT_LEAFYGRASS = 1284.0;
const float MAT_METAL = 1088.0;
const float MAT_DIAMONDPLATE = 1056.0;
const float MAT_FOIL = 1072.0;
const float MAT_CORRODEDMETAL = 1040.0;
const float MAT_ICE = 1536.0;
const float MAT_GLACIER = 1540.0;
const float MAT_WATER = 2048.0;
const float MAT_CRACKEDLAVA = 804.0;

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float shadowFactor(vec4 ls, vec3 normal) {
    if (ubo.fogParams.w < 0.5) return 1.0;
    vec3 proj = ls.xyz / max(ls.w, 0.0001);
    vec2 uv = proj.xy * 0.5 + 0.5;
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || proj.z > 1.0) return 1.0;
    float bias = max(0.0015 * (1.0 - normal.y), 0.0004);
    float sum = 0.0;
    vec2 texel = 1.0 / textureSize(shadowMap, 0);
    for (int x = -1; x <= 1; ++x) {
        for (int y = -1; y <= 1; ++y) {
            float depth = texture(shadowMap, uv + vec2(x, y) * texel).r;
            sum += (proj.z - bias > depth) ? 0.0 : 1.0;
        }
    }
    return sum / 9.0;
}

vec3 aces(vec3 x) {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0);
}

void main() {
    vec3 base = vColor.rgb;
    float alpha = vColor.a;
    if (vParams.y > 0.5) {
        base *= texture(texSampler, vUv).rgb;
    }
    float material = vParams.x;

    // Procedural material pattern factor (documented approximation).
    float pattern = 1.0;
    if (material == MAT_WOOD || material == MAT_WOODPLANKS) {
        float plank = step(0.92, fract(vWorldPos.x * 0.5 + vWorldPos.z * 0.13));
        float grain = 0.92 + 0.08 * sin(vWorldPos.z * 14.0 + sin(vWorldPos.x * 6.0));
        pattern = grain * (1.0 - plank * 0.35);
    } else if (material == MAT_GRASS || material == MAT_LEAFYGRASS) {
        pattern = 0.85 + 0.3 * hash(floor(vWorldPos.xz * 6.0));
    } else if (material == MAT_DIAMONDPLATE) {
        vec2 g = abs(fract(vWorldPos.xz * 2.0) - 0.5);
        pattern = 0.85 + 0.3 * step(0.42, max(g.x, g.y));
    } else if (material == MAT_CORRODEDMETAL) {
        pattern = 0.75 + 0.45 * hash(floor(vWorldPos.xy * 3.0) + floor(vWorldPos.z * 3.0));
    }
    base *= pattern;

    if (material == MAT_NEON) {
        vec3 c = aces(base * 2.2 * ubo.fogParams.z);
        outColor = vec4(pow(c, vec3(1.0 / 2.2)), alpha);
        return;
    }
    if (material == MAT_CRACKEDLAVA) {
        float cracks = step(0.82, hash(floor(vWorldPos.xz * 2.5)));
        base = mix(base, vec3(1.0, 0.42, 0.08) * 1.8, cracks * 0.85);
    }

    vec3 n = normalize(vNormal);
    // SurfaceAppearance maps: vParams.w flags (1 = normal, 2 = rough,
    // 4 = metal). Normal perturbation uses screen-space derivatives
    // (no authored tangents required).
    float roughness = 0.85;
    float metalness = 0.0;
    float mapFlags = mod(vParams.w, 8.0);
    if (mapFlags > 0.5) {
        if (mod(mapFlags, 2.0) >= 1.0) {
            vec3 mn = texture(normalSampler, vUv).rgb * 2.0 - 1.0;
            vec3 q0 = dFdx(vWorldPos), q1 = dFdy(vWorldPos);
            vec2 s0 = dFdx(vUv), s1 = dFdy(vUv);
            vec3 t = normalize(q0 * s1.t - q1 * s0.t);
            vec3 b = normalize(-q0 * s1.s + q1 * s0.s);
            n = normalize(t * mn.x + b * mn.y + n * mn.z);
        }
        if (mod(floor(mapFlags / 2.0), 2.0) >= 1.0) roughness = texture(roughSampler, vUv).g;
        if (mod(floor(mapFlags / 4.0), 2.0) >= 1.0) metalness = texture(metalSampler, vUv).b;
    }

    vec3 sunDir = normalize(ubo.sunDir.xyz);
    float ndl = max(dot(n, sunDir), 0.0);
    float shadow = (vParams.z > 0.5) ? shadowFactor(vLightSpace, n) : 1.0;

    // Hemisphere ambient: outdoor ambient from above, plain ambient below.
    vec3 hemi = mix(ubo.ambient.rgb, ubo.outdoorAmbient.rgb, n.y * 0.5 + 0.5);
    vec3 lit = base * (hemi + ubo.sunColor.rgb * ndl * shadow * (1.0 - metalness * 0.6));

    // Dynamic PointLight / SpotLight / SurfaceLight instances.
    int lightCount = int(ubo.dynDir[0].w + 0.5);
    for (int i = 0; i < 8; ++i) {
        if (i >= lightCount) break;
        vec3 toLight = ubo.dynPos[i].xyz - vWorldPos;
        float dist = length(toLight);
        float range = ubo.dynPos[i].w;
        if (dist > range) continue;
        vec3 ldir = toLight / max(dist, 0.0001);
        float att = 1.0 - dist / range;
        att *= att;
        float cone = ubo.dynColor[i].w;
        if (cone > -1.5) {
            float spot = dot(-ldir, normalize(ubo.dynDir[i].xyz));
            att *= smoothstep(cone, cone + 0.08, spot);
        }
        float diff = max(dot(n, ldir), 0.0);
        lit += base * ubo.dynColor[i].rgb * diff * att;
        vec3 viewDir2 = normalize(ubo.cameraPos.xyz - vWorldPos);
        vec3 halfDir2 = normalize(ldir + viewDir2);
        float specPow = mix(64.0, 6.0, roughness);
        float spec = pow(max(dot(n, halfDir2), 0.0), specPow) * mix(0.04, 1.0, metalness) * att;
        lit += ubo.dynColor[i].rgb * spec;
    }

    // Simple specular for shiny materials.
    float shininess = 0.0;
    if (material == MAT_METAL || material == MAT_FOIL || material == MAT_DIAMONDPLATE) shininess = 0.55;
    else if (material == MAT_ICE || material == MAT_GLACIER || material == MAT_GLASS) shininess = 0.7;
    else if (material == 272.0 /* SmoothPlastic */) shininess = 0.22;
    if (shininess > 0.0) {
        vec3 viewDir = normalize(ubo.cameraPos.xyz - vWorldPos);
        vec3 halfDir = normalize(sunDir + viewDir);
        float spec = pow(max(dot(n, halfDir), 0.0), 48.0) * shininess * shadow;
        lit += ubo.sunColor.rgb * spec;
    }
    if (material == MAT_FORCEFIELD) {
        vec3 viewDir = normalize(ubo.cameraPos.xyz - vWorldPos);
        float fresnel = pow(1.0 - abs(dot(n, viewDir)), 2.0);
        lit = base * 1.6 + fresnel * vec3(0.4, 0.7, 1.0);
        alpha = max(alpha, 0.25 + fresnel * 0.55);
    }
    if (material == MAT_GLASS) {
        alpha = min(alpha, 0.65);
    }

    // Fog
    float dist = distance(ubo.cameraPos.xyz, vWorldPos);
    float fogF = smoothstep(ubo.fogParams.x, ubo.fogParams.y, dist);
    lit = mix(lit, ubo.fogColor.rgb, fogF);

    vec3 toned = aces(lit * ubo.fogParams.z);
    outColor = vec4(pow(toned, vec3(1.0 / 2.2)), alpha);
}
