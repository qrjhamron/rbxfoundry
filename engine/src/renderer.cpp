// Headless Vulkan offscreen renderer (Lavapipe-friendly, no surface).
#include "blox/renderer.hpp"

#include <chrono>
#include <cstring>
#include <fstream>
#include <iostream>
#include <map>

#include <png.h>
#include <vulkan/vulkan.h>

#include "shaders_spv.h"

namespace blox {
namespace {

#define VK_CHECK(expr, msg)                                                                            \
    do {                                                                                               \
        VkResult r_ = (expr);                                                                          \
        if (r_ != VK_SUCCESS) {                                                                        \
            error = std::string(msg) + " (VkResult " + std::to_string(static_cast<int>(r_)) + ")";     \
            return false;                                                                              \
        }                                                                                              \
    } while (0)

struct Vertex {
    float pos[3];
    float normal[3];
    float uv[2];
};

struct UboData {
    float view_proj[16];
    float light_view_proj[16];
    float sun_dir[4];
    float sun_color[4];
    float ambient[4];
    float outdoor_ambient[4];
    float fog_color[4];
    float fog_params[4]; // start, end, exposure, shadows on
    float camera_pos[4];
    float sky_top[4];
    float sky_bottom[4];
    float dyn_pos[8][4];
    float dyn_color[8][4];
    float dyn_dir[8][4]; // dyn_dir[0][3] = light count
    float cam_right[4];
    float cam_up[4];
};

struct PushData {
    float model[16];
    float color[4];
    float params[4]; // material, useTexture, receiveShadow, unused
};

std::array<float, 16> mat_identity() {
    return {1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1};
}
std::array<float, 16> mat_mul(const std::array<float, 16>& a, const std::array<float, 16>& b) {
    std::array<float, 16> c{};
    for (int col = 0; col < 4; ++col)
        for (int row = 0; row < 4; ++row) {
            float s = 0;
            for (int k = 0; k < 4; ++k) s += a[k * 4 + row] * b[col * 4 + k];
            c[col * 4 + row] = s;
        }
    return c;
}
std::array<float, 3> vsub(std::array<float, 3> a, std::array<float, 3> b) {
    return {a[0] - b[0], a[1] - b[1], a[2] - b[2]};
}
std::array<float, 3> vcross(std::array<float, 3> a, std::array<float, 3> b) {
    return {a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]};
}
std::array<float, 3> vnorm(std::array<float, 3> a) {
    float l = std::sqrt(a[0] * a[0] + a[1] * a[1] + a[2] * a[2]);
    if (l <= 0) return {0, 1, 0};
    return {a[0] / l, a[1] / l, a[2] / l};
}
float vdot(std::array<float, 3> a, std::array<float, 3> b) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }

std::array<float, 16> look_at(const std::array<float, 3>& eye, const std::array<float, 3>& target,
                              const std::array<float, 3>& up_in) {
    auto f = vnorm(vsub(target, eye));
    auto s = vnorm(vcross(f, up_in));
    auto u = vcross(s, f);
    // Column-major view matrix (RH, camera looks down -Z in view space).
    return {
        s[0], u[0], -f[0], 0,
        s[1], u[1], -f[1], 0,
        s[2], u[2], -f[2], 0,
        -vdot(s, eye), -vdot(u, eye), vdot(f, eye), 1,
    };
}

std::array<float, 16> perspective(float fov_deg, float aspect, float near_p, float far_p) {
    float f = 1.0f / std::tan(fov_deg * 3.14159265f / 360.0f);
    std::array<float, 16> m{};
    m[0] = f / aspect;
    m[5] = -f; // Vulkan Y-flip
    m[10] = far_p / (near_p - far_p);
    m[11] = -1;
    m[14] = near_p * far_p / (near_p - far_p);
    return m;
}

std::array<float, 16> orthographic(float height, float aspect, float near_p, float far_p) {
    float w = height * aspect, h = height;
    std::array<float, 16> m{};
    m[0] = 2.0f / w;
    m[5] = -2.0f / h;
    m[10] = 1.0f / (near_p - far_p);
    m[14] = near_p / (near_p - far_p);
    m[15] = 1;
    return m;
}

} // namespace

struct Renderer::Impl {
    VkInstance instance = VK_NULL_HANDLE;
    VkPhysicalDevice physical = VK_NULL_HANDLE;
    VkDevice device = VK_NULL_HANDLE;
    VkQueue queue = VK_NULL_HANDLE;
    uint32_t queue_family = 0;
    VkCommandPool pool = VK_NULL_HANDLE;
    VkDescriptorSetLayout set_layout = VK_NULL_HANDLE;
    VkPipelineLayout pipeline_layout = VK_NULL_HANDLE;
    VkRenderPass render_pass = VK_NULL_HANDLE;
    VkRenderPass shadow_pass = VK_NULL_HANDLE;
    VkPipeline scene_pipeline = VK_NULL_HANDLE;
    VkPipeline scene_pipeline_blend = VK_NULL_HANDLE;
    VkPipeline sky_pipeline = VK_NULL_HANDLE;
    VkPipeline shadow_pipeline = VK_NULL_HANDLE;
    VkSampler sampler = VK_NULL_HANDLE;

    uint32_t find_memory(uint32_t type_filter, VkMemoryPropertyFlags props, bool& ok) {
        VkPhysicalDeviceMemoryProperties mem;
        vkGetPhysicalDeviceMemoryProperties(physical, &mem);
        for (uint32_t i = 0; i < mem.memoryTypeCount; ++i) {
            if ((type_filter & (1u << i)) && (mem.memoryTypes[i].propertyFlags & props) == props) {
                ok = true;
                return i;
            }
        }
        ok = false;
        return 0;
    }

    bool create_buffer(VkDeviceSize size, VkBufferUsageFlags usage, VkMemoryPropertyFlags props,
                       VkBuffer& buffer, VkDeviceMemory& memory, std::string& error) {
        VkBufferCreateInfo info{VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO};
        info.size = size;
        info.usage = usage;
        info.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
        VK_CHECK(vkCreateBuffer(device, &info, nullptr, &buffer), "vkCreateBuffer");
        VkMemoryRequirements req;
        vkGetBufferMemoryRequirements(device, buffer, &req);
        bool ok = false;
        uint32_t type = find_memory(req.memoryTypeBits, props, ok);
        if (!ok) { error = "no suitable memory type"; return false; }
        VkMemoryAllocateInfo alloc{VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
        alloc.allocationSize = req.size;
        alloc.memoryTypeIndex = type;
        VK_CHECK(vkAllocateMemory(device, &alloc, nullptr, &memory), "vkAllocateMemory");
        vkBindBufferMemory(device, buffer, memory, 0);
        return true;
    }

    bool create_image(uint32_t w, uint32_t h, VkFormat format, VkImageUsageFlags usage,
                      VkImageAspectFlags aspect, VkImage& image, VkDeviceMemory& memory,
                      VkImageView& view, std::string& error) {
        VkImageCreateInfo info{VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO};
        info.imageType = VK_IMAGE_TYPE_2D;
        info.format = format;
        info.extent = {w, h, 1};
        info.mipLevels = 1;
        info.arrayLayers = 1;
        info.samples = VK_SAMPLE_COUNT_1_BIT;
        info.tiling = VK_IMAGE_TILING_OPTIMAL;
        info.usage = usage;
        info.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
        info.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
        VK_CHECK(vkCreateImage(device, &info, nullptr, &image), "vkCreateImage");
        VkMemoryRequirements req;
        vkGetImageMemoryRequirements(device, image, &req);
        bool ok = false;
        uint32_t type = find_memory(req.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, ok);
        if (!ok) { error = "no device-local memory"; return false; }
        VkMemoryAllocateInfo alloc{VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
        alloc.allocationSize = req.size;
        alloc.memoryTypeIndex = type;
        VK_CHECK(vkAllocateMemory(device, &alloc, nullptr, &memory), "vkAllocateMemory(image)");
        vkBindImageMemory(device, image, memory, 0);
        VkImageViewCreateInfo vinfo{VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO};
        vinfo.image = image;
        vinfo.viewType = VK_IMAGE_VIEW_TYPE_2D;
        vinfo.format = format;
        vinfo.subresourceRange = {aspect, 0, 1, 0, 1};
        VK_CHECK(vkCreateImageView(device, &vinfo, nullptr, &view), "vkCreateImageView");
        return true;
    }

    VkShaderModule make_shader(const uint32_t* spirv, size_t words, std::string& error) {
        VkShaderModuleCreateInfo info{VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO};
        info.codeSize = words * 4;
        info.pCode = spirv;
        VkShaderModule mod = VK_NULL_HANDLE;
        if (vkCreateShaderModule(device, &info, nullptr, &mod) != VK_SUCCESS) {
            error = "vkCreateShaderModule failed";
            return VK_NULL_HANDLE;
        }
        return mod;
    }
};

Renderer::Renderer() = default;
Renderer::~Renderer() {
    if (impl_) {
        VkDevice d = impl_->device;
        if (d) {
            vkDestroyPipeline(d, impl_->scene_pipeline, nullptr);
            vkDestroyPipeline(d, impl_->scene_pipeline_blend, nullptr);
            vkDestroyPipeline(d, impl_->sky_pipeline, nullptr);
            vkDestroyPipeline(d, impl_->shadow_pipeline, nullptr);
            vkDestroyPipelineLayout(d, impl_->pipeline_layout, nullptr);
            vkDestroyDescriptorSetLayout(d, impl_->set_layout, nullptr);
            vkDestroyRenderPass(d, impl_->render_pass, nullptr);
            vkDestroyRenderPass(d, impl_->shadow_pass, nullptr);
            vkDestroySampler(d, impl_->sampler, nullptr);
            vkDestroyCommandPool(d, impl_->pool, nullptr);
            vkDestroyDevice(d, nullptr);
        }
        if (impl_->instance) vkDestroyInstance(impl_->instance, nullptr);
        delete impl_;
    }
}

bool Renderer::init(std::string& error) {
    if (initialized_) return true;
    impl_ = new Impl();

    VkApplicationInfo app{VK_STRUCTURE_TYPE_APPLICATION_INFO};
    app.pApplicationName = "blox-engine";
    app.apiVersion = VK_API_VERSION_1_1;
    VkInstanceCreateInfo ici{VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
    ici.pApplicationInfo = &app;
    VK_CHECK(vkCreateInstance(&ici, nullptr, &impl_->instance), "vkCreateInstance (is a Vulkan driver installed?)");

    uint32_t count = 0;
    vkEnumeratePhysicalDevices(impl_->instance, &count, nullptr);
    if (count == 0) { error = "no Vulkan physical devices found"; return false; }
    std::vector<VkPhysicalDevice> devices(count);
    vkEnumeratePhysicalDevices(impl_->instance, &count, devices.data());
    // Deterministic preference: discrete > integrated > virtual > cpu.
    int best_score = -1;
    for (auto dev : devices) {
        VkPhysicalDeviceProperties props;
        vkGetPhysicalDeviceProperties(dev, &props);
        int score = props.deviceType == VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU ? 4
                    : props.deviceType == VK_PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU ? 3
                    : props.deviceType == VK_PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU ? 2
                    : props.deviceType == VK_PHYSICAL_DEVICE_TYPE_CPU ? 1 : 0;
        if (score > best_score) {
            best_score = score;
            impl_->physical = dev;
            device_name_ = props.deviceName;
        }
    }

    uint32_t qcount = 0;
    vkGetPhysicalDeviceQueueFamilyProperties(impl_->physical, &qcount, nullptr);
    std::vector<VkQueueFamilyProperties> qprops(qcount);
    vkGetPhysicalDeviceQueueFamilyProperties(impl_->physical, &qcount, qprops.data());
    bool found = false;
    for (uint32_t i = 0; i < qcount; ++i) {
        if (qprops[i].queueFlags & VK_QUEUE_GRAPHICS_BIT) {
            impl_->queue_family = i;
            found = true;
            break;
        }
    }
    if (!found) { error = "no graphics queue family"; return false; }

    float priority = 1.0f;
    VkDeviceQueueCreateInfo qinfo{VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO};
    qinfo.queueFamilyIndex = impl_->queue_family;
    qinfo.queueCount = 1;
    qinfo.pQueuePriorities = &priority;
    VkDeviceCreateInfo dinfo{VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO};
    dinfo.queueCreateInfoCount = 1;
    dinfo.pQueueCreateInfos = &qinfo;
    VK_CHECK(vkCreateDevice(impl_->physical, &dinfo, nullptr, &impl_->device), "vkCreateDevice");
    vkGetDeviceQueue(impl_->device, impl_->queue_family, 0, &impl_->queue);

    VkCommandPoolCreateInfo pinfo{VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};
    pinfo.flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT;
    pinfo.queueFamilyIndex = impl_->queue_family;
    VK_CHECK(vkCreateCommandPool(impl_->device, &pinfo, nullptr, &impl_->pool), "vkCreateCommandPool");

    VkDevice d = impl_->device;
    // Descriptor layout: UBO + shadow map + color/normal/rough/metal maps.
    VkDescriptorSetLayoutBinding bindings[6]{};
    bindings[0] = {0, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, 1, VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT, nullptr};
    for (uint32_t i = 1; i < 6; ++i)
        bindings[i] = {i, VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, 1, VK_SHADER_STAGE_FRAGMENT_BIT, nullptr};
    VkDescriptorSetLayoutCreateInfo linfo{VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO};
    linfo.bindingCount = 6;
    linfo.pBindings = bindings;
    VK_CHECK(vkCreateDescriptorSetLayout(d, &linfo, nullptr, &impl_->set_layout), "vkCreateDescriptorSetLayout");

    VkPushConstantRange push_range{VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT, 0, sizeof(PushData)};
    VkPipelineLayoutCreateInfo plinfo{VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO};
    plinfo.setLayoutCount = 1;
    plinfo.pSetLayouts = &impl_->set_layout;
    plinfo.pushConstantRangeCount = 1;
    plinfo.pPushConstantRanges = &push_range;
    VK_CHECK(vkCreatePipelineLayout(d, &plinfo, nullptr, &impl_->pipeline_layout), "vkCreatePipelineLayout");

    VkSamplerCreateInfo sinfo{VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO};
    sinfo.magFilter = VK_FILTER_LINEAR;
    sinfo.minFilter = VK_FILTER_LINEAR;
    sinfo.mipmapMode = VK_SAMPLER_MIPMAP_MODE_LINEAR;
    sinfo.addressModeU = sinfo.addressModeV = sinfo.addressModeW = VK_SAMPLER_ADDRESS_MODE_REPEAT;
    sinfo.maxLod = 16.0f;
    VK_CHECK(vkCreateSampler(d, &sinfo, nullptr, &impl_->sampler), "vkCreateSampler");

    // Main render pass: colour + depth.
    VkAttachmentDescription attachments[2]{};
    attachments[0].format = VK_FORMAT_R8G8B8A8_UNORM;
    attachments[0].samples = VK_SAMPLE_COUNT_1_BIT;
    attachments[0].loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR;
    attachments[0].storeOp = VK_ATTACHMENT_STORE_OP_STORE;
    attachments[0].initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    attachments[0].finalLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    attachments[1].format = VK_FORMAT_D32_SFLOAT;
    attachments[1].samples = VK_SAMPLE_COUNT_1_BIT;
    attachments[1].loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR;
    attachments[1].storeOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
    attachments[1].initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    attachments[1].finalLayout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
    VkAttachmentReference colour_ref{0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
    VkAttachmentReference depth_ref{1, VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL};
    VkSubpassDescription subpass{};
    subpass.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
    subpass.colorAttachmentCount = 1;
    subpass.pColorAttachments = &colour_ref;
    subpass.pDepthStencilAttachment = &depth_ref;
    VkRenderPassCreateInfo rinfo{VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO};
    rinfo.attachmentCount = 2;
    rinfo.pAttachments = attachments;
    rinfo.subpassCount = 1;
    rinfo.pSubpasses = &subpass;
    VK_CHECK(vkCreateRenderPass(d, &rinfo, nullptr, &impl_->render_pass), "vkCreateRenderPass");

    // Shadow render pass: depth only.
    VkAttachmentDescription sh_att{};
    sh_att.format = VK_FORMAT_D32_SFLOAT;
    sh_att.samples = VK_SAMPLE_COUNT_1_BIT;
    sh_att.loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR;
    sh_att.storeOp = VK_ATTACHMENT_STORE_OP_STORE;
    sh_att.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    sh_att.finalLayout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;
    VkAttachmentReference sh_ref{0, VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL};
    VkSubpassDescription sh_sub{};
    sh_sub.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
    sh_sub.pDepthStencilAttachment = &sh_ref;
    VkRenderPassCreateInfo sh_info{VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO};
    sh_info.attachmentCount = 1;
    sh_info.pAttachments = &sh_att;
    sh_info.subpassCount = 1;
    sh_info.pSubpasses = &sh_sub;
    VK_CHECK(vkCreateRenderPass(d, &sh_info, nullptr, &impl_->shadow_pass), "vkCreateRenderPass(shadow)");

    initialized_ = true;
    return true;
}

namespace {

struct TextureEntry {
    VkImage image = VK_NULL_HANDLE;
    VkDeviceMemory memory = VK_NULL_HANDLE;
    VkImageView view = VK_NULL_HANDLE;
};

bool load_png_rgba(const std::string& path, uint32_t& w, uint32_t& h, std::vector<uint8_t>& out,
                   std::string& error);

} // namespace

bool Renderer::ensure_pipelines(std::string& error) {
    Impl& impl = *impl_;
    if (impl.scene_pipeline != VK_NULL_HANDLE) return true;
    VkDevice d = impl.device;
    VkShaderModule scene_vert = impl.make_shader(reinterpret_cast<const uint32_t*>(kSceneVertSpv), kSceneVertSpvWords, error);
    VkShaderModule scene_frag = impl.make_shader(reinterpret_cast<const uint32_t*>(kSceneFragSpv), kSceneFragSpvWords, error);
    VkShaderModule sky_vert = impl.make_shader(reinterpret_cast<const uint32_t*>(kSkyVertSpv), kSkyVertSpvWords, error);
    VkShaderModule sky_frag = impl.make_shader(reinterpret_cast<const uint32_t*>(kSkyFragSpv), kSkyFragSpvWords, error);
    VkShaderModule shadow_vert = impl.make_shader(reinterpret_cast<const uint32_t*>(kShadowVertSpv), kShadowVertSpvWords, error);
    if (!scene_vert || !scene_frag || !sky_vert || !sky_frag || !shadow_vert) return false;

    VkVertexInputBindingDescription binding{0, sizeof(Vertex), VK_VERTEX_INPUT_RATE_VERTEX};
    VkVertexInputAttributeDescription attrs[3]{
        {0, 0, VK_FORMAT_R32G32B32_SFLOAT, offsetof(Vertex, pos)},
        {1, 0, VK_FORMAT_R32G32B32_SFLOAT, offsetof(Vertex, normal)},
        {2, 0, VK_FORMAT_R32G32_SFLOAT, offsetof(Vertex, uv)},
    };
    VkPipelineVertexInputStateCreateInfo vin{VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO};
    vin.vertexBindingDescriptionCount = 1;
    vin.pVertexBindingDescriptions = &binding;
    vin.vertexAttributeDescriptionCount = 3;
    vin.pVertexAttributeDescriptions = attrs;
    VkPipelineInputAssemblyStateCreateInfo assembly{VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO};
    assembly.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
    VkPipelineViewportStateCreateInfo viewport_state{VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO};
    viewport_state.viewportCount = 1;
    viewport_state.scissorCount = 1;
    VkDynamicState dynamics[2] = {VK_DYNAMIC_STATE_VIEWPORT, VK_DYNAMIC_STATE_SCISSOR};
    VkPipelineDynamicStateCreateInfo dyn{VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO};
    dyn.dynamicStateCount = 2;
    dyn.pDynamicStates = dynamics;
    VkPipelineRasterizationStateCreateInfo raster{VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO};
    raster.polygonMode = VK_POLYGON_MODE_FILL;
    // Two-sided rendering: the projection's Y-flip reverses winding, and
    // imported Roblox meshes do not guarantee winding either. Outward
    // normals (guaranteed by the geometry generators / mesh files) plus
    // the depth buffer keep shading and occlusion correct.
    raster.cullMode = VK_CULL_MODE_NONE;
    raster.frontFace = VK_FRONT_FACE_COUNTER_CLOCKWISE;
    raster.lineWidth = 1.0f;
    raster.depthBiasEnable = VK_TRUE;
    raster.depthBiasConstantFactor = 1.25f;
    raster.depthBiasSlopeFactor = 1.75f;
    VkPipelineMultisampleStateCreateInfo msaa{VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO};
    msaa.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;

    auto make_pipeline = [&](VkShaderModule vs, VkShaderModule fs, VkRenderPass pass, bool blend,
                             bool depth_write, bool no_vertex_input, VkPipeline& out) -> bool {
        VkPipelineShaderStageCreateInfo stages[2]{};
        stages[0] = {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, nullptr, 0, VK_SHADER_STAGE_VERTEX_BIT, vs, "main", nullptr};
        uint32_t stage_count = 1;
        if (fs) {
            stages[1] = {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, nullptr, 0, VK_SHADER_STAGE_FRAGMENT_BIT, fs, "main", nullptr};
            stage_count = 2;
        }
        VkPipelineDepthStencilStateCreateInfo depth{VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO};
        depth.depthTestEnable = VK_TRUE;
        depth.depthWriteEnable = depth_write ? VK_TRUE : VK_FALSE;
        depth.depthCompareOp = VK_COMPARE_OP_LESS_OR_EQUAL;
        VkPipelineColorBlendAttachmentState attach{};
        attach.colorWriteMask = VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT |
                                VK_COLOR_COMPONENT_B_BIT | VK_COLOR_COMPONENT_A_BIT;
        if (blend) {
            attach.blendEnable = VK_TRUE;
            attach.srcColorBlendFactor = VK_BLEND_FACTOR_SRC_ALPHA;
            attach.dstColorBlendFactor = VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA;
            attach.colorBlendOp = VK_BLEND_OP_ADD;
            attach.srcAlphaBlendFactor = VK_BLEND_FACTOR_ONE;
            attach.dstAlphaBlendFactor = VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA;
            attach.alphaBlendOp = VK_BLEND_OP_ADD;
        }
        VkPipelineColorBlendStateCreateInfo blending{VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO};
        blending.attachmentCount = pass == impl.shadow_pass ? 0 : 1;
        blending.pAttachments = &attach;
        VkGraphicsPipelineCreateInfo info{VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO};
        info.stageCount = stage_count;
        info.pStages = stages;
        static const VkPipelineVertexInputStateCreateInfo empty_vin{
            VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO};
        info.pVertexInputState = no_vertex_input ? &empty_vin : &vin;
        info.pInputAssemblyState = &assembly;
        info.pViewportState = &viewport_state;
        info.pRasterizationState = &raster;
        info.pMultisampleState = &msaa;
        info.pDepthStencilState = &depth;
        info.pColorBlendState = &blending;
        info.pDynamicState = &dyn;
        info.layout = impl.pipeline_layout;
        info.renderPass = pass;
        info.subpass = 0;
        VkResult r = vkCreateGraphicsPipelines(d, VK_NULL_HANDLE, 1, &info, nullptr, &out);
        if (r != VK_SUCCESS) { error = "vkCreateGraphicsPipelines failed"; return false; }
        return true;
    };

    bool ok = make_pipeline(scene_vert, scene_frag, impl.render_pass, false, true, false, impl.scene_pipeline) &&
              make_pipeline(scene_vert, scene_frag, impl.render_pass, true, false, false, impl.scene_pipeline_blend) &&
              make_pipeline(sky_vert, sky_frag, impl.render_pass, false, false, true, impl.sky_pipeline) &&
              make_pipeline(shadow_vert, VK_NULL_HANDLE, impl.shadow_pass, false, true, false, impl.shadow_pipeline);
    vkDestroyShaderModule(d, scene_vert, nullptr);
    vkDestroyShaderModule(d, scene_frag, nullptr);
    vkDestroyShaderModule(d, sky_vert, nullptr);
    vkDestroyShaderModule(d, sky_frag, nullptr);
    vkDestroyShaderModule(d, shadow_vert, nullptr);
    return ok;
}

RenderResult Renderer::render(const Scene& scene, const RenderRequest& request) {
    RenderResult result;
    auto start = std::chrono::steady_clock::now();
    std::string error;
    if (!initialized_ && !init(error)) { result.error = error; return result; }
    if (!ensure_pipelines(error)) { result.error = error; return result; }
    if (request.width == 0 || request.height == 0 || request.width > 8192 || request.height > 8192) {
        result.error = "invalid render size";
        return result;
    }
    VkDevice d = impl_->device;
    result.device_name = device_name_;
    result.width = request.width;
    result.height = request.height;

    // ---- merge geometry, compute bounds ----
    std::vector<Vertex> vertices;
    std::vector<uint32_t> indices;
    struct Range { uint32_t first_index; uint32_t count; size_t draw; };
    std::vector<Range> ranges;
    std::array<float, 3> bmin{1e30f, 1e30f, 1e30f}, bmax{-1e30f, -1e30f, -1e30f};
    for (size_t di = 0; di < scene.draws.size(); ++di) {
        const DrawItem& draw = scene.draws[di];
        if (draw.indices.empty()) continue;
        uint32_t base = static_cast<uint32_t>(vertices.size());
        for (size_t i = 0; i < draw.positions.size(); ++i) {
            Vertex v{};
            std::memcpy(v.pos, draw.positions[i].data(), sizeof(v.pos));
            std::memcpy(v.normal, draw.normals[i].data(), sizeof(v.normal));
            if (i < draw.uvs.size()) std::memcpy(v.uv, draw.uvs[i].data(), sizeof(v.uv));
            vertices.push_back(v);
            // world-space bounds (apply model matrix)
            const auto& m = draw.model;
            float x = m[0] * v.pos[0] + m[4] * v.pos[1] + m[8] * v.pos[2] + m[12];
            float y = m[1] * v.pos[0] + m[5] * v.pos[1] + m[9] * v.pos[2] + m[13];
            float z = m[2] * v.pos[0] + m[6] * v.pos[1] + m[10] * v.pos[2] + m[14];
            bmin = {std::min(bmin[0], x), std::min(bmin[1], y), std::min(bmin[2], z)};
            bmax = {std::max(bmax[0], x), std::max(bmax[1], y), std::max(bmax[2], z)};
        }
        uint32_t first = static_cast<uint32_t>(indices.size());
        for (uint32_t idx : draw.indices) indices.push_back(base + idx);
        ranges.push_back({first, static_cast<uint32_t>(draw.indices.size()), di});
        result.triangles += static_cast<uint32_t>(draw.indices.size()) / 3;
    }
    bool have_bounds = !ranges.empty();
    if (!have_bounds) { bmin = {-10, -10, -10}; bmax = {10, 10, 10}; }

    // Order: opaque first (as stored), transparent far-to-near.
    std::vector<Range> ordered;
    std::vector<Range> transparent;
    for (const Range& r : ranges) {
        if (scene.draws[r.draw].transparency > 0.0f) transparent.push_back(r);
        else ordered.push_back(r);
    }
    auto centre_of = [&](const DrawItem& draw) {
        return std::array<float, 3>{draw.model[12], draw.model[13], draw.model[14]};
    };
    std::sort(transparent.begin(), transparent.end(), [&](const Range& a, const Range& b) {
        auto ca = centre_of(scene.draws[a.draw]);
        auto cb = centre_of(scene.draws[b.draw]);
        auto dist2 = [&](std::array<float, 3> c) {
            float dx = c[0] - request.camera.position[0];
            float dy = c[1] - request.camera.position[1];
            float dz = c[2] - request.camera.position[2];
            return dx * dx + dy * dy + dz * dz;
        };
        return dist2(ca) > dist2(cb);
    });
    ordered.insert(ordered.end(), transparent.begin(), transparent.end());

    // ---- cameras / light matrices ----
    float aspect = static_cast<float>(request.width) / static_cast<float>(request.height);
    auto view = look_at(request.camera.position, request.camera.look_at, request.camera.up);
    auto proj = request.camera.orthographic
                    ? orthographic(request.camera.ortho_height, aspect, request.camera.near_plane, request.camera.far_plane)
                    : perspective(request.camera.fov_degrees, aspect, request.camera.near_plane, request.camera.far_plane);
    auto view_proj = mat_mul(proj, view);

    std::array<float, 3> centre{(bmin[0] + bmax[0]) / 2, (bmin[1] + bmax[1]) / 2, (bmin[2] + bmax[2]) / 2};
    float radius = 0.5f * std::sqrt((bmax[0] - bmin[0]) * (bmax[0] - bmin[0]) +
                                    (bmax[1] - bmin[1]) * (bmax[1] - bmin[1]) +
                                    (bmax[2] - bmin[2]) * (bmax[2] - bmin[2]));
    radius = std::max(radius, 1.0f);
    auto sun_dir = scene.lighting.sun_direction; // direction light travels
    std::array<float, 3> to_sun{-sun_dir[0], -sun_dir[1], -sun_dir[2]};
    std::array<float, 3> light_pos{centre[0] + to_sun[0] * radius * 2, centre[1] + to_sun[1] * radius * 2,
                                   centre[2] + to_sun[2] * radius * 2};
    auto light_view = look_at(light_pos, centre, {0, 1, 0});
    auto light_proj = orthographic(radius * 2, 1.0f, 0.1f, radius * 4);
    auto light_view_proj = mat_mul(light_proj, light_view);

    // ---- Vulkan resources for this render ----
    VkImage color_image, depth_image, shadow_image;
    VkDeviceMemory color_mem, depth_mem, shadow_mem;
    VkImageView color_view, depth_view, shadow_view;
    VkFramebuffer framebuffer, shadow_fb;
    VkBuffer vbuf, ibuf, ubo_buf, readback;
    VkDeviceMemory vbuf_mem, ibuf_mem, ubo_mem, readback_mem;
    VkDescriptorPool desc_pool;
    VkDescriptorSet desc_set;
    auto cleanup = [&]() {
        vkDestroyFramebuffer(d, framebuffer, nullptr);
        vkDestroyFramebuffer(d, shadow_fb, nullptr);
        vkDestroyImageView(d, color_view, nullptr);
        vkDestroyImageView(d, depth_view, nullptr);
        vkDestroyImageView(d, shadow_view, nullptr);
        vkDestroyImage(d, color_image, nullptr);
        vkDestroyImage(d, depth_image, nullptr);
        vkDestroyImage(d, shadow_image, nullptr);
        vkFreeMemory(d, color_mem, nullptr);
        vkFreeMemory(d, depth_mem, nullptr);
        vkFreeMemory(d, shadow_mem, nullptr);
        vkDestroyBuffer(d, vbuf, nullptr);
        vkDestroyBuffer(d, ibuf, nullptr);
        vkDestroyBuffer(d, ubo_buf, nullptr);
        vkDestroyBuffer(d, readback, nullptr);
        vkFreeMemory(d, vbuf_mem, nullptr);
        vkFreeMemory(d, ibuf_mem, nullptr);
        vkFreeMemory(d, ubo_mem, nullptr);
        vkFreeMemory(d, readback_mem, nullptr);
        vkDestroyDescriptorPool(d, desc_pool, nullptr);
    };

    uint32_t shadow_size = request.shadows ? request.shadow_map_size : 1;
    if (!impl_->create_image(request.width, request.height, VK_FORMAT_R8G8B8A8_UNORM,
                             VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
                             VK_IMAGE_ASPECT_COLOR_BIT, color_image, color_mem, color_view, error) ||
        !impl_->create_image(request.width, request.height, VK_FORMAT_D32_SFLOAT,
                             VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT, VK_IMAGE_ASPECT_DEPTH_BIT,
                             depth_image, depth_mem, depth_view, error) ||
        !impl_->create_image(shadow_size, shadow_size, VK_FORMAT_D32_SFLOAT,
                             VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT | VK_IMAGE_USAGE_SAMPLED_BIT,
                             VK_IMAGE_ASPECT_DEPTH_BIT, shadow_image, shadow_mem, shadow_view, error)) {
        result.error = error;
        return result;
    }
    {
        VkImageView atts[2] = {color_view, depth_view};
        VkFramebufferCreateInfo info{VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO};
        info.renderPass = impl_->render_pass;
        info.attachmentCount = 2;
        info.pAttachments = atts;
        info.width = request.width;
        info.height = request.height;
        info.layers = 1;
        if (vkCreateFramebuffer(d, &info, nullptr, &framebuffer) != VK_SUCCESS) { result.error = "framebuffer"; cleanup(); return result; }
        VkFramebufferCreateInfo sinfo{VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO};
        sinfo.renderPass = impl_->shadow_pass;
        sinfo.attachmentCount = 1;
        sinfo.pAttachments = &shadow_view;
        sinfo.width = shadow_size;
        sinfo.height = shadow_size;
        sinfo.layers = 1;
        if (vkCreateFramebuffer(d, &sinfo, nullptr, &shadow_fb) != VK_SUCCESS) { result.error = "shadow framebuffer"; cleanup(); return result; }
    }

    VkDeviceSize vsize = std::max<VkDeviceSize>(sizeof(Vertex) * vertices.size(), 16);
    VkDeviceSize isize = std::max<VkDeviceSize>(sizeof(uint32_t) * indices.size(), 16);
    const VkMemoryPropertyFlags host = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
    if (!impl_->create_buffer(vsize, VK_BUFFER_USAGE_VERTEX_BUFFER_BIT, host, vbuf, vbuf_mem, error) ||
        !impl_->create_buffer(isize, VK_BUFFER_USAGE_INDEX_BUFFER_BIT, host, ibuf, ibuf_mem, error) ||
        !impl_->create_buffer(sizeof(UboData), VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT, host, ubo_buf, ubo_mem, error) ||
        !impl_->create_buffer(static_cast<VkDeviceSize>(request.width) * request.height * 4,
                              VK_BUFFER_USAGE_TRANSFER_DST_BIT, host, readback, readback_mem, error)) {
        result.error = error;
        cleanup();
        return result;
    }
    auto upload = [&](VkBuffer buf, VkDeviceMemory mem, const void* data, VkDeviceSize bytes) {
        void* mapped = nullptr;
        vkMapMemory(d, mem, 0, bytes, 0, &mapped);
        std::memcpy(mapped, data, static_cast<size_t>(bytes));
        vkUnmapMemory(d, mem);
    };
    if (!vertices.empty()) upload(vbuf, vbuf_mem, vertices.data(), sizeof(Vertex) * vertices.size());
    if (!indices.empty()) upload(ibuf, ibuf_mem, indices.data(), sizeof(uint32_t) * indices.size());

    UboData ubo{};
    std::memcpy(ubo.view_proj, view_proj.data(), sizeof(ubo.view_proj));
    std::memcpy(ubo.light_view_proj, light_view_proj.data(), sizeof(ubo.light_view_proj));
    auto set4 = [](float* dst, std::array<float, 3> v, float w) { dst[0] = v[0]; dst[1] = v[1]; dst[2] = v[2]; dst[3] = w; };
    set4(ubo.sun_dir, to_sun, 0);
    set4(ubo.sun_color, scene.lighting.sun_color, 0);
    set4(ubo.ambient, scene.lighting.ambient, 0);
    set4(ubo.outdoor_ambient, scene.lighting.outdoor_ambient, 0);
    set4(ubo.fog_color, srgb_to_linear(scene.lighting.fog_color), 0);
    ubo.fog_params[0] = scene.lighting.fog_start;
    ubo.fog_params[1] = std::max(scene.lighting.fog_end, scene.lighting.fog_start + 1.0f);
    ubo.fog_params[2] = scene.lighting.exposure;
    ubo.fog_params[3] = (request.shadows && scene.lighting.shadows_enabled) ? 1.0f : 0.0f;
    set4(ubo.camera_pos, request.camera.position, 0);
    set4(ubo.sky_top, srgb_to_linear(scene.lighting.sky_top), 0);
    set4(ubo.sky_bottom, srgb_to_linear(scene.lighting.sky_bottom), 0);
    {
        size_t count = std::min<size_t>(scene.lighting.dynamic_lights.size(), 8);
        for (size_t i = 0; i < 8; ++i) {
            if (i < count) {
                const DynamicLight& light = scene.lighting.dynamic_lights[i];
                ubo.dyn_pos[i][0] = light.position[0];
                ubo.dyn_pos[i][1] = light.position[1];
                ubo.dyn_pos[i][2] = light.position[2];
                ubo.dyn_pos[i][3] = light.range;
                ubo.dyn_color[i][0] = light.color[0];
                ubo.dyn_color[i][1] = light.color[1];
                ubo.dyn_color[i][2] = light.color[2];
                ubo.dyn_color[i][3] = light.cone_cos;
                ubo.dyn_dir[i][0] = light.direction[0];
                ubo.dyn_dir[i][1] = light.direction[1];
                ubo.dyn_dir[i][2] = light.direction[2];
            }
            ubo.dyn_dir[i][3] = 0;
        }
        ubo.dyn_dir[0][3] = static_cast<float>(count);
        // Camera basis for billboards.
        std::array<float, 3> fwd{
            request.camera.look_at[0] - request.camera.position[0],
            request.camera.look_at[1] - request.camera.position[1],
            request.camera.look_at[2] - request.camera.position[2]};
        float fl = std::sqrt(fwd[0] * fwd[0] + fwd[1] * fwd[1] + fwd[2] * fwd[2]);
        if (fl > 0) { fwd[0] /= fl; fwd[1] /= fl; fwd[2] /= fl; }
        std::array<float, 3> right{
            fwd[2] * request.camera.up[1] - fwd[1] * request.camera.up[2],
            fwd[0] * request.camera.up[2] - fwd[2] * request.camera.up[0],
            fwd[1] * request.camera.up[0] - fwd[0] * request.camera.up[1]};
        // right = up x fwd? Use cross(fwd, up) corrected below:
        right = {fwd[1] * request.camera.up[2] - fwd[2] * request.camera.up[1],
                 fwd[2] * request.camera.up[0] - fwd[0] * request.camera.up[2],
                 fwd[0] * request.camera.up[1] - fwd[1] * request.camera.up[0]};
        float rl = std::sqrt(right[0] * right[0] + right[1] * right[1] + right[2] * right[2]);
        if (rl > 0) { right[0] /= rl; right[1] /= rl; right[2] /= rl; }
        std::array<float, 3> upv{
            right[1] * fwd[2] - right[2] * fwd[1],
            right[2] * fwd[0] - right[0] * fwd[2],
            right[0] * fwd[1] - right[1] * fwd[0]};
        set4(ubo.cam_right, right, 0);
        set4(ubo.cam_up, upv, 0);
    }
    upload(ubo_buf, ubo_mem, &ubo, sizeof(ubo));

    // Descriptor set.
    {
        VkDescriptorPoolSize sizes[2]{{VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, 1},
                                      {VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, 8}};
        VkDescriptorPoolCreateInfo info{VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO};
        info.maxSets = 9;
        info.poolSizeCount = 2;
        info.pPoolSizes = sizes;
        if (vkCreateDescriptorPool(d, &info, nullptr, &desc_pool) != VK_SUCCESS) { result.error = "descriptor pool"; cleanup(); return result; }
        VkDescriptorSetAllocateInfo ainfo{VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO};
        ainfo.descriptorPool = desc_pool;
        ainfo.descriptorSetCount = 1;
        ainfo.pSetLayouts = &impl_->set_layout;
        if (vkAllocateDescriptorSets(d, &ainfo, &desc_set) != VK_SUCCESS) { result.error = "descriptor set"; cleanup(); return result; }
    }
    // Default 1x1 white texture for untextured draws.
    static std::map<std::string, TextureEntry> texture_cache;
    auto get_texture = [&](const std::string& path) -> VkImageView {
        if (path.empty()) return VK_NULL_HANDLE;
        auto it = texture_cache.find(path);
        if (it != texture_cache.end()) return it->second.view;
        uint32_t tw = 0, th = 0;
        std::vector<uint8_t> pixels;
        std::string tex_error;
        TextureEntry entry;
        if (!load_png_rgba(path, tw, th, pixels, tex_error)) return VK_NULL_HANDLE;
        if (!impl_->create_image(tw, th, VK_FORMAT_R8G8B8A8_UNORM,
                                 VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
                                 VK_IMAGE_ASPECT_COLOR_BIT, entry.image, entry.memory, entry.view, tex_error)) {
            return VK_NULL_HANDLE;
        }
        // Upload pixels with a one-shot command buffer.
        VkBuffer staging;
        VkDeviceMemory staging_mem;
        if (!impl_->create_buffer(std::max<VkDeviceSize>(pixels.size(), 16), VK_BUFFER_USAGE_TRANSFER_SRC_BIT,
                                  host, staging, staging_mem, tex_error)) return VK_NULL_HANDLE;
        upload(staging, staging_mem, pixels.data(), pixels.size());
        VkCommandBufferAllocateInfo cinfo{VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};
        cinfo.commandPool = impl_->pool;
        cinfo.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
        cinfo.commandBufferCount = 1;
        VkCommandBuffer cmd;
        vkAllocateCommandBuffers(d, &cinfo, &cmd);
        VkCommandBufferBeginInfo begin{VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
        begin.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
        vkBeginCommandBuffer(cmd, &begin);
        VkImageMemoryBarrier barrier{VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER};
        barrier.oldLayout = VK_IMAGE_LAYOUT_UNDEFINED;
        barrier.newLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
        barrier.srcQueueFamilyIndex = barrier.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
        barrier.image = entry.image;
        barrier.subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
        barrier.srcAccessMask = 0;
        barrier.dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
        vkCmdPipelineBarrier(cmd, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT,
                             0, 0, nullptr, 0, nullptr, 1, &barrier);
        VkBufferImageCopy region{};
        region.imageSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1};
        region.imageExtent = {tw, th, 1};
        vkCmdCopyBufferToImage(cmd, staging, entry.image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &region);
        barrier.oldLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
        barrier.newLayout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;
        barrier.srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
        barrier.dstAccessMask = VK_ACCESS_SHADER_READ_BIT;
        vkCmdPipelineBarrier(cmd, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                             0, 0, nullptr, 0, nullptr, 1, &barrier);
        vkEndCommandBuffer(cmd);
        VkSubmitInfo submit{VK_STRUCTURE_TYPE_SUBMIT_INFO};
        submit.commandBufferCount = 1;
        submit.pCommandBuffers = &cmd;
        vkQueueSubmit(impl_->queue, 1, &submit, VK_NULL_HANDLE);
        vkQueueWaitIdle(impl_->queue);
        vkFreeCommandBuffers(d, impl_->pool, 1, &cmd);
        vkDestroyBuffer(d, staging, nullptr);
        vkFreeMemory(d, staging_mem, nullptr);
        texture_cache[path] = entry;
        return entry.view;
    };

    // White fallback texture view: build once via a 1x1 image stored in cache[""].
    VkImageView white_view = get_texture(std::string());
    if (white_view == VK_NULL_HANDLE) {
        // Create it through the same path using an in-memory temp PNG file.
        std::string tmp = "/tmp/blox-white.png";
        std::vector<uint8_t> white = {255, 255, 255, 255};
        std::string werr;
        if (write_png(tmp, 1, 1, white, werr)) white_view = get_texture(tmp);
    }

    // Flat-normal and black fallbacks (same temp-PNG trick as white).
    VkImageView flat_normal_view = VK_NULL_HANDLE;
    VkImageView black_view = VK_NULL_HANDLE;
    {
        std::string werr;
        std::vector<uint8_t> flat = {128, 128, 255, 255};
        if (write_png("/tmp/blox-flatnormal.png", 1, 1, flat, werr))
            flat_normal_view = get_texture("/tmp/blox-flatnormal.png");
        std::vector<uint8_t> black = {0, 0, 0, 255};
        if (write_png("/tmp/blox-black.png", 1, 1, black, werr))
            black_view = get_texture("/tmp/blox-black.png");
    }
    if (flat_normal_view == VK_NULL_HANDLE) flat_normal_view = white_view;
    if (black_view == VK_NULL_HANDLE) black_view = white_view;

    auto write_desc = [&](VkImageView tex_view, VkImageView normal_view, VkImageView rough_view,
                          VkImageView metal_view) {
        VkDescriptorBufferInfo binfo{ubo_buf, 0, sizeof(UboData)};
        VkDescriptorImageInfo shadow_info{impl_->sampler, shadow_view, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL};
        VkDescriptorImageInfo tex_info{impl_->sampler, tex_view ? tex_view : white_view,
                                       VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL};
        VkDescriptorImageInfo normal_info{impl_->sampler, normal_view ? normal_view : flat_normal_view,
                                          VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL};
        VkDescriptorImageInfo rough_info{impl_->sampler, rough_view ? rough_view : white_view,
                                         VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL};
        VkDescriptorImageInfo metal_info{impl_->sampler, metal_view ? metal_view : black_view,
                                         VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL};
        VkWriteDescriptorSet writes[6]{};
        writes[0] = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, nullptr, desc_set, 0, 0, 1,
                     VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, nullptr, &binfo, nullptr};
        writes[1] = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, nullptr, desc_set, 1, 0, 1,
                     VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, &shadow_info, nullptr, nullptr};
        writes[2] = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, nullptr, desc_set, 2, 0, 1,
                     VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, &tex_info, nullptr, nullptr};
        writes[3] = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, nullptr, desc_set, 3, 0, 1,
                     VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, &normal_info, nullptr, nullptr};
        writes[4] = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, nullptr, desc_set, 4, 0, 1,
                     VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, &rough_info, nullptr, nullptr};
        writes[5] = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, nullptr, desc_set, 5, 0, 1,
                     VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, &metal_info, nullptr, nullptr};
        vkUpdateDescriptorSets(d, 6, writes, 0, nullptr);
    };
    write_desc(white_view, flat_normal_view, white_view, black_view);

    // ---- record commands ----
    VkCommandBufferAllocateInfo cinfo{VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};
    cinfo.commandPool = impl_->pool;
    cinfo.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
    cinfo.commandBufferCount = 1;
    VkCommandBuffer cmd;
    vkAllocateCommandBuffers(d, &cinfo, &cmd);
    VkCommandBufferBeginInfo begin{VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
    vkBeginCommandBuffer(cmd, &begin);

    VkDeviceSize offsets = 0;
    auto bind_geometry = [&]() {
        vkCmdBindVertexBuffers(cmd, 0, 1, &vbuf, &offsets);
        vkCmdBindIndexBuffer(cmd, ibuf, 0, VK_INDEX_TYPE_UINT32);
    };
    auto push_draw = [&](const Range& range) {
        const DrawItem& draw = scene.draws[range.draw];
        PushData push{};
        std::memcpy(push.model, draw.model.data(), sizeof(push.model));
        push.color[0] = draw.color[0];
        push.color[1] = draw.color[1];
        push.color[2] = draw.color[2];
        push.color[3] = 1.0f - draw.transparency;
        push.params[0] = static_cast<float>(draw.material);
        push.params[1] = draw.texture_path.empty() ? 0.0f : 1.0f;
        push.params[2] = draw.receive_shadow ? 1.0f : 0.0f;
        float flags = (draw.normal_path.empty() ? 0.0f : 1.0f) +
                      (draw.roughness_path.empty() ? 0.0f : 2.0f) +
                      (draw.metalness_path.empty() ? 0.0f : 4.0f) +
                      (draw.billboard ? 8.0f : 0.0f);
        push.params[3] = flags;
        vkCmdPushConstants(cmd, impl_->pipeline_layout,
                           VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT, 0, sizeof(push), &push);
        vkCmdDrawIndexed(cmd, range.count, 1, range.first_index, 0, 0);
        result.draw_calls++;
    };

    // Shadow pass.
    if (request.shadows && scene.lighting.shadows_enabled) {
        VkClearValue clear{};
        clear.depthStencil = {1.0f, 0};
        VkRenderPassBeginInfo info{VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO};
        info.renderPass = impl_->shadow_pass;
        info.framebuffer = shadow_fb;
        info.renderArea = {{0, 0}, {shadow_size, shadow_size}};
        info.clearValueCount = 1;
        info.pClearValues = &clear;
        vkCmdBeginRenderPass(cmd, &info, VK_SUBPASS_CONTENTS_INLINE);
        VkViewport vp{0, 0, static_cast<float>(shadow_size), static_cast<float>(shadow_size), 0, 1};
        VkRect2D sc{{0, 0}, {shadow_size, shadow_size}};
        vkCmdSetViewport(cmd, 0, 1, &vp);
        vkCmdSetScissor(cmd, 0, 1, &sc);
        vkCmdBindPipeline(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, impl_->shadow_pipeline);
        vkCmdBindDescriptorSets(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, impl_->pipeline_layout, 0, 1, &desc_set, 0, nullptr);
        bind_geometry();
        for (const Range& range : ranges) {
            const DrawItem& draw = scene.draws[range.draw];
            if (draw.transparency > 0.0f || !draw.cast_shadow) continue;
            push_draw(range);
        }
        vkCmdEndRenderPass(cmd);
    }

    // Main pass.
    {
        VkClearValue clears[2]{};
        clears[0].color = {{scene.lighting.background[0], scene.lighting.background[1],
                            scene.lighting.background[2], 1.0f}};
        clears[1].depthStencil = {1.0f, 0};
        VkRenderPassBeginInfo info{VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO};
        info.renderPass = impl_->render_pass;
        info.framebuffer = framebuffer;
        info.renderArea = {{0, 0}, {request.width, request.height}};
        info.clearValueCount = 2;
        info.pClearValues = clears;
        vkCmdBeginRenderPass(cmd, &info, VK_SUBPASS_CONTENTS_INLINE);
        VkViewport vp{0, 0, static_cast<float>(request.width), static_cast<float>(request.height), 0, 1};
        VkRect2D sc{{0, 0}, {request.width, request.height}};
        vkCmdSetViewport(cmd, 0, 1, &vp);
        vkCmdSetScissor(cmd, 0, 1, &sc);
        vkCmdBindDescriptorSets(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, impl_->pipeline_layout, 0, 1, &desc_set, 0, nullptr);
        // Sky gradient.
        vkCmdBindPipeline(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, impl_->sky_pipeline);
        vkCmdDraw(cmd, 3, 1, 0, 0);
        // Scene geometry.
        vkCmdBindPipeline(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, impl_->scene_pipeline);
        bind_geometry();
        bool blend_bound = false;
        VkImageView current_tex = white_view;
        VkImageView current_normal = flat_normal_view;
        VkImageView current_rough = white_view;
        VkImageView current_metal = black_view;
        for (const Range& range : ordered) {
            const DrawItem& draw = scene.draws[range.draw];
            if (draw.transparency > 0.0f && !blend_bound) {
                vkCmdBindPipeline(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, impl_->scene_pipeline_blend);
                blend_bound = true;
            }
            VkImageView want_tex = draw.texture_path.empty() ? white_view : get_texture(draw.texture_path);
            VkImageView want_normal = draw.normal_path.empty() ? flat_normal_view : get_texture(draw.normal_path);
            VkImageView want_rough = draw.roughness_path.empty() ? white_view : get_texture(draw.roughness_path);
            VkImageView want_metal = draw.metalness_path.empty() ? black_view : get_texture(draw.metalness_path);
            if (want_tex == VK_NULL_HANDLE) want_tex = white_view;
            if (want_normal == VK_NULL_HANDLE) want_normal = flat_normal_view;
            if (want_rough == VK_NULL_HANDLE) want_rough = white_view;
            if (want_metal == VK_NULL_HANDLE) want_metal = black_view;
            if (want_tex != current_tex || want_normal != current_normal ||
                want_rough != current_rough || want_metal != current_metal) {
                current_tex = want_tex;
                current_normal = want_normal;
                current_rough = want_rough;
                current_metal = want_metal;
                write_desc(current_tex, current_normal, current_rough, current_metal);
                vkCmdBindDescriptorSets(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, impl_->pipeline_layout,
                                        0, 1, &desc_set, 0, nullptr);
            }
            push_draw(range);
        }
        vkCmdEndRenderPass(cmd);
    }

    // Readback.
    {
        VkBufferImageCopy region{};
        region.imageSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1};
        region.imageExtent = {request.width, request.height, 1};
        vkCmdCopyImageToBuffer(cmd, color_image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, readback, 1, &region);
    }
    vkEndCommandBuffer(cmd);

    VkSubmitInfo submit{VK_STRUCTURE_TYPE_SUBMIT_INFO};
    submit.commandBufferCount = 1;
    submit.pCommandBuffers = &cmd;
    VkFence fence;
    VkFenceCreateInfo finfo{VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
    vkCreateFence(d, &finfo, nullptr, &fence);
    vkQueueSubmit(impl_->queue, 1, &submit, fence);
    vkWaitForFences(d, 1, &fence, VK_TRUE, 5'000'000'000ULL);
    vkDestroyFence(d, fence, nullptr);
    vkFreeCommandBuffers(d, impl_->pool, 1, &cmd);

    // Read pixels.
    result.rgba.resize(static_cast<size_t>(request.width) * request.height * 4);
    {
        void* mapped = nullptr;
        vkMapMemory(d, readback_mem, 0, result.rgba.size(), 0, &mapped);
        std::memcpy(result.rgba.data(), mapped, result.rgba.size());
        vkUnmapMemory(d, readback_mem);
    }
    auto end = std::chrono::steady_clock::now();
    result.render_ms = std::chrono::duration<double, std::milli>(end - start).count();

    if (!request.output_path.empty()) {
        std::string png_error;
        if (!write_png(request.output_path, request.width, request.height, result.rgba, png_error)) {
            result.error = png_error;
            cleanup();
            return result;
        }
    }
    if (!request.return_pixels) result.rgba.clear();
    result.ok = true;
    cleanup();
    return result;
}

namespace {

bool load_png_rgba(const std::string& path, uint32_t& w, uint32_t& h, std::vector<uint8_t>& out,
                   std::string& error) {
    FILE* fp = fopen(path.c_str(), "rb");
    if (!fp) { error = "cannot open texture '" + path + "'"; return false; }
    unsigned char header[8];
    if (fread(header, 1, 8, fp) != 8 || png_sig_cmp(header, 0, 8)) {
        fclose(fp);
        error = "texture '" + path + "' is not a PNG (only PNG textures are supported)";
        return false;
    }
    png_structp png_ptr = png_create_read_struct(PNG_LIBPNG_VER_STRING, nullptr, nullptr, nullptr);
    png_infop info_ptr = png_create_info_struct(png_ptr);
    if (setjmp(png_jmpbuf(png_ptr))) {
        png_destroy_read_struct(&png_ptr, &info_ptr, nullptr);
        fclose(fp);
        error = "failed to decode PNG texture '" + path + "'";
        return false;
    }
    png_init_io(png_ptr, fp);
    png_set_sig_bytes(png_ptr, 8);
    png_read_info(png_ptr, info_ptr);
    w = png_get_image_width(png_ptr, info_ptr);
    h = png_get_image_height(png_ptr, info_ptr);
    png_set_expand(png_ptr);
    png_set_strip_16(png_ptr);
    png_set_add_alpha(png_ptr, 0xFF, PNG_FILLER_AFTER);
    png_read_update_info(png_ptr, info_ptr);
    out.resize(static_cast<size_t>(w) * h * 4);
    std::vector<png_bytep> rows(h);
    for (uint32_t y = 0; y < h; ++y) rows[y] = out.data() + static_cast<size_t>(y) * w * 4;
    png_read_image(png_ptr, rows.data());
    png_destroy_read_struct(&png_ptr, &info_ptr, nullptr);
    fclose(fp);
    return true;
}

} // namespace

bool write_png(const std::string& path, uint32_t width, uint32_t height,
               const std::vector<uint8_t>& rgba, std::string& error) {
    if (rgba.size() < static_cast<size_t>(width) * height * 4) {
        error = "pixel buffer too small for PNG";
        return false;
    }
    FILE* fp = fopen(path.c_str(), "wb");
    if (!fp) { error = "cannot open '" + path + "' for writing"; return false; }
    png_structp png_ptr = png_create_write_struct(PNG_LIBPNG_VER_STRING, nullptr, nullptr, nullptr);
    if (!png_ptr) { fclose(fp); error = "png_create_write_struct failed"; return false; }
    png_infop info_ptr = png_create_info_struct(png_ptr);
    if (setjmp(png_jmpbuf(png_ptr))) {
        png_destroy_write_struct(&png_ptr, &info_ptr);
        fclose(fp);
        error = "PNG encode failed for '" + path + "'";
        return false;
    }
    png_init_io(png_ptr, fp);
    png_set_IHDR(png_ptr, info_ptr, width, height, 8, PNG_COLOR_TYPE_RGBA,
                 PNG_INTERLACE_NONE, PNG_COMPRESSION_TYPE_DEFAULT, PNG_FILTER_TYPE_DEFAULT);
    png_write_info(png_ptr, info_ptr);
    for (uint32_t y = 0; y < height; ++y) {
        png_write_row(png_ptr, rgba.data() + static_cast<size_t>(y) * width * 4);
    }
    png_write_end(png_ptr, nullptr);
    png_destroy_write_struct(&png_ptr, &info_ptr);
    fclose(fp);
    return true;
}

} // namespace blox

// ===================== UI pipeline (Phase 4) =====================
#include "blox/ui.hpp"

namespace blox {

namespace {

struct UiVertex {
    float pos[2];
    float uv[2];
    float color[4];
    float clip[4];
    float local[2];
    float misc[3]; // radius, w, h
};

} // namespace

struct UiPipeline {
    VkDescriptorSetLayout layout = VK_NULL_HANDLE;
    VkPipelineLayout pipe_layout = VK_NULL_HANDLE;
    VkRenderPass pass = VK_NULL_HANDLE;
    VkPipeline pipeline = VK_NULL_HANDLE;
};

namespace {
bool dummy_anchor_unused() { return true; }
} // namespace

bool Renderer::ensure_ui_pipeline(UiPipeline& ui, std::string& error) {
    Impl& impl = *impl_;
    if (ui.pipeline != VK_NULL_HANDLE) return true;
    VkDevice d = impl.device;
    VkDescriptorSetLayoutBinding bindings[2]{};
    bindings[0] = {0, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, 1, VK_SHADER_STAGE_VERTEX_BIT, nullptr};
    bindings[1] = {1, VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, 1, VK_SHADER_STAGE_FRAGMENT_BIT, nullptr};
    VkDescriptorSetLayoutCreateInfo linfo{VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO};
    linfo.bindingCount = 2;
    linfo.pBindings = bindings;
    if (vkCreateDescriptorSetLayout(d, &linfo, nullptr, &ui.layout) != VK_SUCCESS) { error = "ui descriptor layout"; return false; }
    VkPipelineLayoutCreateInfo plinfo{VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO};
    plinfo.setLayoutCount = 1;
    plinfo.pSetLayouts = &ui.layout;
    if (vkCreatePipelineLayout(d, &plinfo, nullptr, &ui.pipe_layout) != VK_SUCCESS) { error = "ui pipeline layout"; return false; }
    VkAttachmentDescription att{};
    att.format = VK_FORMAT_R8G8B8A8_UNORM;
    att.samples = VK_SAMPLE_COUNT_1_BIT;
    att.loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR;
    att.storeOp = VK_ATTACHMENT_STORE_OP_STORE;
    att.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    att.finalLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    VkAttachmentReference ref{0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
    VkSubpassDescription sub{};
    sub.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
    sub.colorAttachmentCount = 1;
    sub.pColorAttachments = &ref;
    VkRenderPassCreateInfo rinfo{VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO};
    rinfo.attachmentCount = 1;
    rinfo.pAttachments = &att;
    rinfo.subpassCount = 1;
    rinfo.pSubpasses = &sub;
    if (vkCreateRenderPass(d, &rinfo, nullptr, &ui.pass) != VK_SUCCESS) { error = "ui render pass"; return false; }

    VkShaderModule vs = impl.make_shader(reinterpret_cast<const uint32_t*>(kUiVertSpv), kUiVertSpvWords, error);
    VkShaderModule fs = impl.make_shader(reinterpret_cast<const uint32_t*>(kUiFragSpv), kUiFragSpvWords, error);
    if (!vs || !fs) return false;
    VkPipelineShaderStageCreateInfo stages[2]{};
    stages[0] = {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, nullptr, 0, VK_SHADER_STAGE_VERTEX_BIT, vs, "main", nullptr};
    stages[1] = {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, nullptr, 0, VK_SHADER_STAGE_FRAGMENT_BIT, fs, "main", nullptr};
    VkVertexInputBindingDescription binding{0, sizeof(UiVertex), VK_VERTEX_INPUT_RATE_VERTEX};
    VkVertexInputAttributeDescription attrs[6]{
        {0, 0, VK_FORMAT_R32G32_SFLOAT, offsetof(UiVertex, pos)},
        {1, 0, VK_FORMAT_R32G32_SFLOAT, offsetof(UiVertex, uv)},
        {2, 0, VK_FORMAT_R32G32B32A32_SFLOAT, offsetof(UiVertex, color)},
        {3, 0, VK_FORMAT_R32G32B32A32_SFLOAT, offsetof(UiVertex, clip)},
        {4, 0, VK_FORMAT_R32G32_SFLOAT, offsetof(UiVertex, local)},
        {5, 0, VK_FORMAT_R32G32B32_SFLOAT, offsetof(UiVertex, misc)},
    };
    VkPipelineVertexInputStateCreateInfo vin{VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO};
    vin.vertexBindingDescriptionCount = 1;
    vin.pVertexBindingDescriptions = &binding;
    vin.vertexAttributeDescriptionCount = 6;
    vin.pVertexAttributeDescriptions = attrs;
    VkPipelineInputAssemblyStateCreateInfo assembly{VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO};
    assembly.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
    VkPipelineViewportStateCreateInfo vps{VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO};
    vps.viewportCount = 1;
    vps.scissorCount = 1;
    VkDynamicState dyn_states[2] = {VK_DYNAMIC_STATE_VIEWPORT, VK_DYNAMIC_STATE_SCISSOR};
    VkPipelineDynamicStateCreateInfo dyn{VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO};
    dyn.dynamicStateCount = 2;
    dyn.pDynamicStates = dyn_states;
    VkPipelineRasterizationStateCreateInfo raster{VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO};
    raster.polygonMode = VK_POLYGON_MODE_FILL;
    raster.cullMode = VK_CULL_MODE_NONE;
    raster.frontFace = VK_FRONT_FACE_COUNTER_CLOCKWISE;
    raster.lineWidth = 1.0f;
    VkPipelineMultisampleStateCreateInfo msaa{VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO};
    msaa.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
    VkPipelineDepthStencilStateCreateInfo depth{VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO};
    depth.depthTestEnable = VK_FALSE;
    depth.depthWriteEnable = VK_FALSE;
    VkPipelineColorBlendAttachmentState blend{};
    blend.colorWriteMask = VK_COLOR_COMPONENT_R_BIT | VK_COLOR_COMPONENT_G_BIT | VK_COLOR_COMPONENT_B_BIT | VK_COLOR_COMPONENT_A_BIT;
    blend.blendEnable = VK_TRUE;
    blend.srcColorBlendFactor = VK_BLEND_FACTOR_SRC_ALPHA;
    blend.dstColorBlendFactor = VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA;
    blend.colorBlendOp = VK_BLEND_OP_ADD;
    blend.srcAlphaBlendFactor = VK_BLEND_FACTOR_ONE;
    blend.dstAlphaBlendFactor = VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA;
    blend.alphaBlendOp = VK_BLEND_OP_ADD;
    VkPipelineColorBlendStateCreateInfo blending{VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO};
    blending.attachmentCount = 1;
    blending.pAttachments = &blend;
    VkGraphicsPipelineCreateInfo info{VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO};
    info.stageCount = 2;
    info.pStages = stages;
    info.pVertexInputState = &vin;
    info.pInputAssemblyState = &assembly;
    info.pViewportState = &vps;
    info.pRasterizationState = &raster;
    info.pMultisampleState = &msaa;
    info.pDepthStencilState = &depth;
    info.pColorBlendState = &blending;
    info.pDynamicState = &dyn;
    info.layout = ui.pipe_layout;
    info.renderPass = ui.pass;
    VkResult r = vkCreateGraphicsPipelines(d, VK_NULL_HANDLE, 1, &info, nullptr, &ui.pipeline);
    vkDestroyShaderModule(d, vs, nullptr);
    vkDestroyShaderModule(d, fs, nullptr);
    if (r != VK_SUCCESS) { error = "ui pipeline creation failed"; return false; }
    return true;
}

std::array<float, 4> gradient_color(const UiQuad& quad, float cx, float cy) {
    if (!quad.has_gradient) return quad.color;
    float rad = quad.gradient_degrees * 3.14159265f / 180.0f;
    float gx = std::cos(rad), gy = std::sin(rad);
    float denom = std::abs(quad.width * gx) + std::abs(quad.height * gy);
    float t = denom > 0 ? 0.5f + ((cx - quad.width / 2) * gx + (cy - quad.height / 2) * gy) / denom : 0.0f;
    t = std::clamp(t, 0.0f, 1.0f);
    std::array<float, 4> out;
    for (int c = 0; c < 4; ++c) out[c] = quad.color[c] + (quad.gradient_end[c] - quad.color[c]) * t;
    return out;
}


RenderResult Renderer::render_ui(const UiScene& scene, const UiRenderRequest& request) {
    RenderResult result;
    auto start = std::chrono::steady_clock::now();
    std::string error;
    if (!initialized_ && !init(error)) { result.error = error; return result; }
    static UiPipeline ui;
    if (!ensure_ui_pipeline(ui, error)) { result.error = error; return result; }
    VkDevice d = impl_->device;
    result.device_name = device_name_;
    result.width = request.width;
    result.height = request.height;

    // ---- build vertices ----
    std::vector<UiVertex> vertices;
    std::vector<uint32_t> indices;
    struct BatchRange { std::string key; uint32_t first; uint32_t count; };
    std::vector<BatchRange> batches;
    auto add_quad_verts = [&](const UiQuad& q, const std::string& key) {
        uint32_t base = static_cast<uint32_t>(vertices.size());
        const float xs[4] = {q.x, q.x + q.width, q.x + q.width, q.x};
        const float ys[4] = {q.y, q.y, q.y + q.height, q.y + q.height};
        const float us[4] = {q.u0, q.u1, q.u1, q.u0};
        const float vs[4] = {q.v0, q.v0, q.v1, q.v1};
        const float lxs[4] = {0, q.width, q.width, 0};
        const float lys[4] = {0, 0, q.height, q.height};
        for (int i = 0; i < 4; ++i) {
            UiVertex v{};
            v.pos[0] = xs[i]; v.pos[1] = ys[i];
            v.uv[0] = us[i]; v.uv[1] = vs[i];
            auto col = gradient_color(q, lxs[i], lys[i]);
            std::memcpy(v.color, col.data(), sizeof(v.color));
            v.clip[0] = q.clip_x; v.clip[1] = q.clip_y; v.clip[2] = q.clip_w; v.clip[3] = q.clip_h;
            v.local[0] = lxs[i]; v.local[1] = lys[i];
            v.misc[0] = q.corner_radius; v.misc[1] = q.width; v.misc[2] = q.height;
            vertices.push_back(v);
        }
        uint32_t first = static_cast<uint32_t>(indices.size());
        indices.insert(indices.end(), {base, base + 1, base + 2, base, base + 2, base + 3});
        if (!batches.empty() && batches.back().key == key) batches.back().count += 6;
        else batches.push_back({key, first, 6});
    };
    if (!request.background_rgba.empty()) {
        UiQuad bg;
        bg.x = 0; bg.y = 0;
        bg.width = static_cast<float>(request.width);
        bg.height = static_cast<float>(request.height);
        bg.color = {1, 1, 1, 1};
        bg.texture_path = "__background__";
        add_quad_verts(bg, "__background__");
    }
    for (const auto& quad : scene.quads) {
        std::string key = quad.use_atlas ? "__atlas__" : (quad.texture_path.empty() ? "__white__" : quad.texture_path);
        add_quad_verts(quad, key);
    }

    // ---- textures ----
    std::map<std::string, TextureEntry> textures;
    auto upload_rgba_texture = [&](const std::string& key, const uint8_t* pixels, uint32_t w, uint32_t h) -> VkImageView {
        auto it = textures.find(key);
        if (it != textures.end()) return it->second.view;
        TextureEntry entry;
        if (!impl_->create_image(w, h, VK_FORMAT_R8G8B8A8_UNORM,
                                 VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT,
                                 VK_IMAGE_ASPECT_COLOR_BIT, entry.image, entry.memory, entry.view, error)) {
            return VK_NULL_HANDLE;
        }
        VkBuffer staging;
        VkDeviceMemory staging_mem;
        const VkMemoryPropertyFlags host = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
        size_t bytes = static_cast<size_t>(w) * h * 4;
        if (!impl_->create_buffer(std::max<size_t>(bytes, 16), VK_BUFFER_USAGE_TRANSFER_SRC_BIT, host, staging, staging_mem, error)) return VK_NULL_HANDLE;
        void* mapped = nullptr;
        vkMapMemory(d, staging_mem, 0, bytes, 0, &mapped);
        std::memcpy(mapped, pixels, bytes);
        vkUnmapMemory(d, staging_mem);
        VkCommandBufferAllocateInfo cinfo{VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};
        cinfo.commandPool = impl_->pool;
        cinfo.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
        cinfo.commandBufferCount = 1;
        VkCommandBuffer cmd;
        vkAllocateCommandBuffers(d, &cinfo, &cmd);
        VkCommandBufferBeginInfo begin{VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
        begin.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
        vkBeginCommandBuffer(cmd, &begin);
        VkImageMemoryBarrier barrier{VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER};
        barrier.oldLayout = VK_IMAGE_LAYOUT_UNDEFINED;
        barrier.newLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
        barrier.srcQueueFamilyIndex = barrier.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
        barrier.image = entry.image;
        barrier.subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
        barrier.dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
        vkCmdPipelineBarrier(cmd, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, nullptr, 0, nullptr, 1, &barrier);
        VkBufferImageCopy region{};
        region.imageSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1};
        region.imageExtent = {w, h, 1};
        vkCmdCopyBufferToImage(cmd, staging, entry.image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &region);
        barrier.oldLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
        barrier.newLayout = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;
        barrier.srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
        barrier.dstAccessMask = VK_ACCESS_SHADER_READ_BIT;
        vkCmdPipelineBarrier(cmd, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, 0, 0, nullptr, 0, nullptr, 1, &barrier);
        vkEndCommandBuffer(cmd);
        VkSubmitInfo submit{VK_STRUCTURE_TYPE_SUBMIT_INFO};
        submit.commandBufferCount = 1;
        submit.pCommandBuffers = &cmd;
        vkQueueSubmit(impl_->queue, 1, &submit, VK_NULL_HANDLE);
        vkQueueWaitIdle(impl_->queue);
        vkFreeCommandBuffers(d, impl_->pool, 1, &cmd);
        vkDestroyBuffer(d, staging, nullptr);
        vkFreeMemory(d, staging_mem, nullptr);
        textures[key] = entry;
        return entry.view;
    };
    auto texture_view = [&](const std::string& key) -> VkImageView {
        if (key == "__atlas__") {
            if (scene.atlas_w == 0 || scene.atlas_rgba.empty()) return VK_NULL_HANDLE;
            return upload_rgba_texture(key, scene.atlas_rgba.data(), scene.atlas_w, scene.atlas_h);
        }
        if (key == "__background__") {
            return upload_rgba_texture(key, request.background_rgba.data(), request.width, request.height);
        }
        if (key == "__white__") {
            static const uint8_t white[4] = {255, 255, 255, 255};
            return upload_rgba_texture(key, white, 1, 1);
        }
        uint32_t w = 0, h = 0;
        std::vector<uint8_t> pixels;
        std::string tex_error;
        if (!load_png_rgba(key, w, h, pixels, tex_error)) return VK_NULL_HANDLE;
        return upload_rgba_texture(key, pixels.data(), w, h);
    };

    // ---- frame resources ----
    VkImage color_image;
    VkDeviceMemory color_mem;
    VkImageView color_view;
    VkFramebuffer fb;
    VkBuffer vbuf, ibuf, ubo_buf, readback;
    VkDeviceMemory vbuf_mem, ibuf_mem, ubo_mem, readback_mem;
    VkDescriptorPool pool;
    auto cleanup = [&]() {
        vkDestroyFramebuffer(d, fb, nullptr);
        vkDestroyImageView(d, color_view, nullptr);
        vkDestroyImage(d, color_image, nullptr);
        vkFreeMemory(d, color_mem, nullptr);
        vkDestroyBuffer(d, vbuf, nullptr);
        vkDestroyBuffer(d, ibuf, nullptr);
        vkDestroyBuffer(d, ubo_buf, nullptr);
        vkDestroyBuffer(d, readback, nullptr);
        vkFreeMemory(d, vbuf_mem, nullptr);
        vkFreeMemory(d, ibuf_mem, nullptr);
        vkFreeMemory(d, ubo_mem, nullptr);
        vkFreeMemory(d, readback_mem, nullptr);
        vkDestroyDescriptorPool(d, pool, nullptr);
        for (auto& [key, entry] : textures) {
            vkDestroyImageView(d, entry.view, nullptr);
            vkDestroyImage(d, entry.image, nullptr);
            vkFreeMemory(d, entry.memory, nullptr);
        }
    };
    if (!impl_->create_image(request.width, request.height, VK_FORMAT_R8G8B8A8_UNORM,
                             VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
                             VK_IMAGE_ASPECT_COLOR_BIT, color_image, color_mem, color_view, error)) {
        result.error = error;
        return result;
    }
    {
        VkFramebufferCreateInfo info{VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO};
        info.renderPass = ui.pass;
        info.attachmentCount = 1;
        info.pAttachments = &color_view;
        info.width = request.width;
        info.height = request.height;
        info.layers = 1;
        if (vkCreateFramebuffer(d, &info, nullptr, &fb) != VK_SUCCESS) { result.error = "ui framebuffer"; cleanup(); return result; }
    }
    const VkMemoryPropertyFlags host = VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT;
    if (!impl_->create_buffer(std::max<size_t>(sizeof(UiVertex) * vertices.size(), 16), VK_BUFFER_USAGE_VERTEX_BUFFER_BIT, host, vbuf, vbuf_mem, error) ||
        !impl_->create_buffer(std::max<size_t>(sizeof(uint32_t) * indices.size(), 16), VK_BUFFER_USAGE_INDEX_BUFFER_BIT, host, ibuf, ibuf_mem, error) ||
        !impl_->create_buffer(16, VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT, host, ubo_buf, ubo_mem, error) ||
        !impl_->create_buffer(static_cast<VkDeviceSize>(request.width) * request.height * 4, VK_BUFFER_USAGE_TRANSFER_DST_BIT, host, readback, readback_mem, error)) {
        result.error = error;
        cleanup();
        return result;
    }
    auto upload = [&](VkBuffer buf, VkDeviceMemory mem, const void* data, size_t bytes) {
        void* mapped = nullptr;
        vkMapMemory(d, mem, 0, bytes, 0, &mapped);
        std::memcpy(mapped, data, bytes);
        vkUnmapMemory(d, mem);
    };
    if (!vertices.empty()) upload(vbuf, vbuf_mem, vertices.data(), sizeof(UiVertex) * vertices.size());
    if (!indices.empty()) upload(ibuf, ibuf_mem, indices.data(), sizeof(uint32_t) * indices.size());
    float viewport[4] = {static_cast<float>(request.width), static_cast<float>(request.height), 0, 0};
    upload(ubo_buf, ubo_mem, viewport, sizeof(viewport));

    // Descriptor sets: one per batch key.
    {
        VkDescriptorPoolSize sizes[2]{{VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, 1},
                                      {VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, 64}};
        VkDescriptorPoolCreateInfo info{VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO};
        info.maxSets = 65;
        info.poolSizeCount = 2;
        info.pPoolSizes = sizes;
        if (vkCreateDescriptorPool(d, &info, nullptr, &pool) != VK_SUCCESS) { result.error = "ui descriptor pool"; cleanup(); return result; }
    }
    std::map<std::string, VkDescriptorSet> sets;
    auto get_set = [&](const std::string& key) -> VkDescriptorSet {
        auto it = sets.find(key);
        if (it != sets.end()) return it->second;
        VkDescriptorSet set;
        VkDescriptorSetAllocateInfo ainfo{VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO};
        ainfo.descriptorPool = pool;
        ainfo.descriptorSetCount = 1;
        ainfo.pSetLayouts = &ui.layout;
        if (vkAllocateDescriptorSets(d, &ainfo, &set) != VK_SUCCESS) return VK_NULL_HANDLE;
        VkImageView view = texture_view(key);
        if (view == VK_NULL_HANDLE) view = texture_view("__white__");
        VkDescriptorBufferInfo binfo{ubo_buf, 0, 16};
        VkDescriptorImageInfo iinfo{impl_->sampler, view, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL};
        VkWriteDescriptorSet writes[2]{};
        writes[0] = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, nullptr, set, 0, 0, 1, VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER, nullptr, &binfo, nullptr};
        writes[1] = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET, nullptr, set, 1, 0, 1, VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, &iinfo, nullptr, nullptr};
        vkUpdateDescriptorSets(d, 2, writes, 0, nullptr);
        sets[key] = set;
        return set;
    };

    VkCommandBufferAllocateInfo cinfo{VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};
    cinfo.commandPool = impl_->pool;
    cinfo.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY;
    cinfo.commandBufferCount = 1;
    VkCommandBuffer cmd;
    vkAllocateCommandBuffers(d, &cinfo, &cmd);
    VkCommandBufferBeginInfo begin{VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
    vkBeginCommandBuffer(cmd, &begin);
    VkClearValue clear{};
    clear.color = {{request.background[0], request.background[1], request.background[2], request.background[3]}};
    VkRenderPassBeginInfo info{VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO};
    info.renderPass = ui.pass;
    info.framebuffer = fb;
    info.renderArea = {{0, 0}, {request.width, request.height}};
    info.clearValueCount = 1;
    info.pClearValues = &clear;
    vkCmdBeginRenderPass(cmd, &info, VK_SUBPASS_CONTENTS_INLINE);
    VkViewport vp{0, 0, static_cast<float>(request.width), static_cast<float>(request.height), 0, 1};
    VkRect2D sc{{0, 0}, {request.width, request.height}};
    vkCmdSetViewport(cmd, 0, 1, &vp);
    vkCmdSetScissor(cmd, 0, 1, &sc);
    vkCmdBindPipeline(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, ui.pipeline);
    VkDeviceSize offsets = 0;
    vkCmdBindVertexBuffers(cmd, 0, 1, &vbuf, &offsets);
    vkCmdBindIndexBuffer(cmd, ibuf, 0, VK_INDEX_TYPE_UINT32);
    for (const auto& batch : batches) {
        VkDescriptorSet set = get_set(batch.key);
        if (set == VK_NULL_HANDLE) continue;
        vkCmdBindDescriptorSets(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, ui.pipe_layout, 0, 1, &set, 0, nullptr);
        vkCmdDrawIndexed(cmd, batch.count, 1, batch.first, 0, 0);
        result.draw_calls++;
    }
    result.triangles = static_cast<uint32_t>(indices.size()) / 3;
    vkCmdEndRenderPass(cmd);
    VkBufferImageCopy region{};
    region.imageSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1};
    region.imageExtent = {request.width, request.height, 1};
    vkCmdCopyImageToBuffer(cmd, color_image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, readback, 1, &region);
    vkEndCommandBuffer(cmd);
    VkSubmitInfo submit{VK_STRUCTURE_TYPE_SUBMIT_INFO};
    submit.commandBufferCount = 1;
    submit.pCommandBuffers = &cmd;
    VkFence fence;
    VkFenceCreateInfo finfo{VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
    vkCreateFence(d, &finfo, nullptr, &fence);
    vkQueueSubmit(impl_->queue, 1, &submit, fence);
    vkWaitForFences(d, 1, &fence, VK_TRUE, 5'000'000'000ULL);
    vkDestroyFence(d, fence, nullptr);
    vkFreeCommandBuffers(d, impl_->pool, 1, &cmd);

    result.rgba.resize(static_cast<size_t>(request.width) * request.height * 4);
    void* mapped = nullptr;
    vkMapMemory(d, readback_mem, 0, result.rgba.size(), 0, &mapped);
    std::memcpy(result.rgba.data(), mapped, result.rgba.size());
    vkUnmapMemory(d, readback_mem);
    result.render_ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - start).count();
    if (!request.output_path.empty()) {
        std::string png_error;
        if (!write_png(request.output_path, request.width, request.height, result.rgba, png_error)) {
            result.error = png_error;
            cleanup();
            return result;
        }
    }
    if (!request.return_pixels) result.rgba.clear();
    result.ok = true;
    cleanup();
    return result;
}

} // namespace blox
