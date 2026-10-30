// Original host Vulkan diagnostics. This does not decode or emulate Switch GPU work.
#include "gpu_lab.h"
#include "shaders.h"
#include <vulkan/vulkan.h>
#include <algorithm>
#include <array>
#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <sstream>
#include <stdexcept>
#include <vector>

namespace {
constexpr uint32_t SIDE = 64;
constexpr size_t PIXEL_BYTES = SIDE * SIDE * 4;
constexpr std::array<float, 36> VERTICES = {
    -0.8f,-0.8f,0.2f, 0,1,0, 0.8f,-0.8f,0.2f, 0,1,0, 0,0.8f,0.2f, 0,1,0,
    -0.8f,-0.8f,0.7f, 1,0,0, 0.8f,-0.8f,0.7f, 1,0,0, 0,0.8f,0.7f, 1,0,0
};

template<class T> T structure(VkStructureType type) { T value{}; value.sType = type; return value; }
void check(VkResult result, const char* operation) {
    if (result != VK_SUCCESS) throw std::runtime_error(std::string(operation) + ": VkResult " + std::to_string(result));
}
std::string quoted(const std::string& text) {
    const char* hex = "0123456789abcdef";
    std::string output = "\"";
    for (unsigned char character : text) {
        if (character == '"' || character == '\\') { output += '\\'; output += static_cast<char>(character); }
        else if (character < 32) { output += "\\u00"; output += hex[character >> 4]; output += hex[character & 15]; }
        else output += static_cast<char>(character);
    }
    return output + '"';
}

struct Buffer { VkBuffer handle{}; VkDeviceMemory memory{}; VkDeviceSize size{}; bool coherent{}; };
struct Image { VkImage handle{}; VkImageView view{}; };
struct Test { std::string name; bool passed; std::string detail; };

class Context {
public:
    bool validation_enabled = false;
    std::atomic<uint32_t> validation_errors{0};
    VkDebugUtilsMessengerEXT debug_messenger{};
    VkInstance instance{};
    VkPhysicalDevice physical{};
    VkPhysicalDeviceProperties properties{};
    VkPhysicalDeviceFeatures features{};
    VkPhysicalDeviceMemoryProperties memory_properties{};
    VkDevice device{};
    VkQueue queue{};
    VkCommandPool command_pool{};
    uint32_t family{};
    std::vector<VkBuffer> buffers;
    std::vector<VkImage> images;
    std::vector<VkImageView> views;
    std::vector<VkDeviceMemory> allocations;
    std::vector<VkShaderModule> shaders;
    std::vector<VkPipeline> pipelines;
    std::vector<VkPipelineLayout> pipeline_layouts;
    std::vector<VkRenderPass> render_passes;
    std::vector<VkFramebuffer> framebuffers;
    std::vector<VkDescriptorPool> descriptor_pools;
    std::vector<VkDescriptorSetLayout> descriptor_layouts;
    std::vector<VkFence> fences;

    ~Context() { shutdown(); }

    void shutdown() {
        if (device) {
            vkDeviceWaitIdle(device);
            for (auto handle : fences) vkDestroyFence(device, handle, nullptr);
            for (auto handle : pipelines) vkDestroyPipeline(device, handle, nullptr);
            for (auto handle : framebuffers) vkDestroyFramebuffer(device, handle, nullptr);
            for (auto handle : render_passes) vkDestroyRenderPass(device, handle, nullptr);
            for (auto handle : pipeline_layouts) vkDestroyPipelineLayout(device, handle, nullptr);
            for (auto handle : descriptor_pools) vkDestroyDescriptorPool(device, handle, nullptr);
            for (auto handle : descriptor_layouts) vkDestroyDescriptorSetLayout(device, handle, nullptr);
            for (auto handle : shaders) vkDestroyShaderModule(device, handle, nullptr);
            for (auto handle : views) vkDestroyImageView(device, handle, nullptr);
            for (auto handle : images) vkDestroyImage(device, handle, nullptr);
            for (auto handle : buffers) vkDestroyBuffer(device, handle, nullptr);
            for (auto handle : allocations) vkFreeMemory(device, handle, nullptr);
            if (command_pool) vkDestroyCommandPool(device, command_pool, nullptr);
            vkDestroyDevice(device, nullptr);
            device = VK_NULL_HANDLE;
        }
        if (instance) {
            if (debug_messenger) {
                const auto destroy = reinterpret_cast<PFN_vkDestroyDebugUtilsMessengerEXT>(vkGetInstanceProcAddr(instance, "vkDestroyDebugUtilsMessengerEXT"));
                if (destroy) destroy(instance, debug_messenger, nullptr);
                debug_messenger = VK_NULL_HANDLE;
            }
            vkDestroyInstance(instance, nullptr);
            instance = VK_NULL_HANDLE;
        }
    }

    static VKAPI_ATTR VkBool32 VKAPI_CALL validation_message(VkDebugUtilsMessageSeverityFlagBitsEXT severity,
            VkDebugUtilsMessageTypeFlagsEXT, const VkDebugUtilsMessengerCallbackDataEXT* message, void* user) {
        auto* context = static_cast<Context*>(user);
        if (severity & VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT) ++context->validation_errors;
        std::fprintf(stderr, "Vulkan validation: %s\n", message->pMessage);
        return VK_FALSE;
    }

    void initialize() {
        auto app = structure<VkApplicationInfo>(VK_STRUCTURE_TYPE_APPLICATION_INFO);
        app.pApplicationName = "Original Emulator GPU Lab";
        app.apiVersion = VK_API_VERSION_1_1;
        auto instance_info = structure<VkInstanceCreateInfo>(VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO);
        instance_info.pApplicationInfo = &app;
        const char* layer = "VK_LAYER_KHRONOS_validation";
        const char* debug_extension = VK_EXT_DEBUG_UTILS_EXTENSION_NAME;
        auto debug = structure<VkDebugUtilsMessengerCreateInfoEXT>(VK_STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT);
        if (const char* requested = std::getenv("EMULATOR_GPU_VALIDATION"); requested && std::string(requested) == "1") {
            uint32_t layer_count = 0;
            check(vkEnumerateInstanceLayerProperties(&layer_count, nullptr), "count validation layers");
            std::vector<VkLayerProperties> layers(layer_count);
            check(vkEnumerateInstanceLayerProperties(&layer_count, layers.data()), "enumerate validation layers");
            if (std::none_of(layers.begin(), layers.end(), [layer](const auto& candidate) { return std::strcmp(candidate.layerName, layer) == 0; }))
                throw std::runtime_error("Requested Khronos validation layer is unavailable");
            validation_enabled = true;
            instance_info.enabledLayerCount = 1; instance_info.ppEnabledLayerNames = &layer;
            instance_info.enabledExtensionCount = 1; instance_info.ppEnabledExtensionNames = &debug_extension;
            debug.messageSeverity = VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT;
            debug.messageType = VK_DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT;
            debug.pfnUserCallback = validation_message; debug.pUserData = this;
            instance_info.pNext = &debug;
        }
        check(vkCreateInstance(&instance_info, nullptr, &instance), "create instance");
        if (validation_enabled) {
            const auto create = reinterpret_cast<PFN_vkCreateDebugUtilsMessengerEXT>(vkGetInstanceProcAddr(instance, "vkCreateDebugUtilsMessengerEXT"));
            if (!create) throw std::runtime_error("Validation debug messenger unavailable");
            check(create(instance, &debug, nullptr, &debug_messenger), "create validation messenger");
        }
        uint32_t count = 0;
        check(vkEnumeratePhysicalDevices(instance, &count, nullptr), "count physical devices");
        if (!count) throw std::runtime_error("No Vulkan physical device");
        std::vector<VkPhysicalDevice> devices(count);
        check(vkEnumeratePhysicalDevices(instance, &count, devices.data()), "enumerate physical devices");
        physical = devices.front();
        for (auto candidate : devices) {
            VkPhysicalDeviceProperties props{};
            vkGetPhysicalDeviceProperties(candidate, &props);
            if (props.deviceType != VK_PHYSICAL_DEVICE_TYPE_CPU) { physical = candidate; break; }
        }
        vkGetPhysicalDeviceProperties(physical, &properties);
        vkGetPhysicalDeviceFeatures(physical, &features);
        vkGetPhysicalDeviceMemoryProperties(physical, &memory_properties);
        if (properties.apiVersion < VK_API_VERSION_1_1) throw std::runtime_error("Vulkan 1.1 is required for this test backend");
        uint32_t queue_count = 0;
        vkGetPhysicalDeviceQueueFamilyProperties(physical, &queue_count, nullptr);
        std::vector<VkQueueFamilyProperties> queues(queue_count);
        vkGetPhysicalDeviceQueueFamilyProperties(physical, &queue_count, queues.data());
        bool found = false;
        for (uint32_t index = 0; index < queue_count; ++index) {
            constexpr VkQueueFlags required = VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT;
            if (queues[index].queueCount && (queues[index].queueFlags & required) == required) { family = index; found = true; break; }
        }
        if (!found) throw std::runtime_error("No combined graphics/compute queue for this test backend");
        const float priority = 1.0f;
        auto queue_info = structure<VkDeviceQueueCreateInfo>(VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO);
        queue_info.queueFamilyIndex = family; queue_info.queueCount = 1; queue_info.pQueuePriorities = &priority;
        auto device_info = structure<VkDeviceCreateInfo>(VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO);
        device_info.queueCreateInfoCount = 1; device_info.pQueueCreateInfos = &queue_info;
        check(vkCreateDevice(physical, &device_info, nullptr, &device), "create logical device");
        vkGetDeviceQueue(device, family, 0, &queue);
        auto pool = structure<VkCommandPoolCreateInfo>(VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO);
        pool.queueFamilyIndex = family;
        check(vkCreateCommandPool(device, &pool, nullptr, &command_pool), "create command pool");
    }

    uint32_t memory_type(uint32_t bits, VkMemoryPropertyFlags required, VkMemoryPropertyFlags preferred = 0) {
        for (int pass = 0; pass < 2; ++pass) {
            for (uint32_t index = 0; index < memory_properties.memoryTypeCount; ++index) {
                const auto flags = memory_properties.memoryTypes[index].propertyFlags;
                if ((bits & (1u << index)) && (flags & required) == required &&
                    (pass || (flags & preferred) == preferred)) return index;
            }
        }
        throw std::runtime_error("No compatible Vulkan memory type");
    }

    Buffer buffer(VkDeviceSize size, VkBufferUsageFlags usage) {
        Buffer output{}; output.size = size;
        auto info = structure<VkBufferCreateInfo>(VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO);
        info.size = size; info.usage = usage; info.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
        check(vkCreateBuffer(device, &info, nullptr, &output.handle), "create buffer");
        buffers.push_back(output.handle);
        VkMemoryRequirements requirements{};
        vkGetBufferMemoryRequirements(device, output.handle, &requirements);
        const auto type = memory_type(requirements.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT, VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);
        output.coherent = (memory_properties.memoryTypes[type].propertyFlags & VK_MEMORY_PROPERTY_HOST_COHERENT_BIT) != 0;
        auto allocation = structure<VkMemoryAllocateInfo>(VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO);
        allocation.allocationSize = requirements.size; allocation.memoryTypeIndex = type;
        check(vkAllocateMemory(device, &allocation, nullptr, &output.memory), "allocate buffer memory");
        allocations.push_back(output.memory);
        check(vkBindBufferMemory(device, output.handle, output.memory, 0), "bind buffer memory");
        return output;
    }

    void write(const Buffer& target, const void* data, size_t size) {
        if (size > target.size) throw std::runtime_error("Host buffer write overflow");
        void* mapped = nullptr;
        check(vkMapMemory(device, target.memory, 0, VK_WHOLE_SIZE, 0, &mapped), "map upload");
        std::memcpy(mapped, data, size);
        auto range = structure<VkMappedMemoryRange>(VK_STRUCTURE_TYPE_MAPPED_MEMORY_RANGE);
        range.memory = target.memory; range.size = VK_WHOLE_SIZE;
        const auto result = target.coherent ? VK_SUCCESS : vkFlushMappedMemoryRanges(device, 1, &range);
        vkUnmapMemory(device, target.memory);
        check(result, "flush host writes");
    }

    std::vector<uint8_t> read(const Buffer& source) {
        void* mapped = nullptr;
        check(vkMapMemory(device, source.memory, 0, VK_WHOLE_SIZE, 0, &mapped), "map readback");
        auto range = structure<VkMappedMemoryRange>(VK_STRUCTURE_TYPE_MAPPED_MEMORY_RANGE);
        range.memory = source.memory; range.size = VK_WHOLE_SIZE;
        const auto result = source.coherent ? VK_SUCCESS : vkInvalidateMappedMemoryRanges(device, 1, &range);
        if (result != VK_SUCCESS) { vkUnmapMemory(device, source.memory); check(result, "invalidate readback"); }
        std::vector<uint8_t> bytes(static_cast<size_t>(source.size));
        std::memcpy(bytes.data(), mapped, bytes.size());
        vkUnmapMemory(device, source.memory);
        return bytes;
    }

    Image image(VkFormat format, VkImageUsageFlags usage, VkImageAspectFlags aspect) {
        VkImageFormatProperties supported{};
        check(vkGetPhysicalDeviceImageFormatProperties(physical, format, VK_IMAGE_TYPE_2D,
              VK_IMAGE_TILING_OPTIMAL, usage, 0, &supported), "query image format/usage");
        if (supported.maxExtent.width < SIDE || supported.maxExtent.height < SIDE) throw std::runtime_error("Image extent unsupported");
        Image output{};
        auto info = structure<VkImageCreateInfo>(VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO);
        info.imageType = VK_IMAGE_TYPE_2D; info.format = format; info.extent = {SIDE, SIDE, 1};
        info.mipLevels = 1; info.arrayLayers = 1; info.samples = VK_SAMPLE_COUNT_1_BIT;
        info.tiling = VK_IMAGE_TILING_OPTIMAL; info.usage = usage; info.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
        check(vkCreateImage(device, &info, nullptr, &output.handle), "create image");
        images.push_back(output.handle);
        VkMemoryRequirements requirements{};
        vkGetImageMemoryRequirements(device, output.handle, &requirements);
        auto allocation = structure<VkMemoryAllocateInfo>(VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO);
        allocation.allocationSize = requirements.size;
        allocation.memoryTypeIndex = memory_type(requirements.memoryTypeBits, 0, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
        VkDeviceMemory memory{};
        check(vkAllocateMemory(device, &allocation, nullptr, &memory), "allocate image memory");
        allocations.push_back(memory);
        check(vkBindImageMemory(device, output.handle, memory, 0), "bind image memory");
        if (usage & (VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT)) {
            auto view = structure<VkImageViewCreateInfo>(VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO);
            view.image = output.handle; view.viewType = VK_IMAGE_VIEW_TYPE_2D; view.format = format;
            view.subresourceRange = {aspect, 0, 1, 0, 1};
            check(vkCreateImageView(device, &view, nullptr, &output.view), "create image view");
            views.push_back(output.view);
        }
        return output;
    }

    VkCommandBuffer begin() {
        auto info = structure<VkCommandBufferAllocateInfo>(VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO);
        info.commandPool = command_pool; info.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY; info.commandBufferCount = 1;
        VkCommandBuffer command{};
        check(vkAllocateCommandBuffers(device, &info, &command), "allocate command buffer");
        auto start = structure<VkCommandBufferBeginInfo>(VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO);
        start.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
        check(vkBeginCommandBuffer(command, &start), "begin command buffer");
        return command;
    }

    void submit(VkCommandBuffer command) {
        check(vkEndCommandBuffer(command), "end command buffer");
        auto fence_info = structure<VkFenceCreateInfo>(VK_STRUCTURE_TYPE_FENCE_CREATE_INFO);
        VkFence fence{};
        check(vkCreateFence(device, &fence_info, nullptr, &fence), "create completion fence");
        fences.push_back(fence);
        auto submit_info = structure<VkSubmitInfo>(VK_STRUCTURE_TYPE_SUBMIT_INFO);
        submit_info.commandBufferCount = 1; submit_info.pCommandBuffers = &command;
        check(vkQueueSubmit(queue, 1, &submit_info, fence), "queue submit");
        check(vkWaitForFences(device, 1, &fence, VK_TRUE, 5'000'000'000ull), "wait for workload");
    }

    template<size_t Size> VkShaderModule shader(const std::array<uint32_t, Size>& words) {
        auto info = structure<VkShaderModuleCreateInfo>(VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO);
        info.codeSize = words.size() * sizeof(uint32_t); info.pCode = words.data();
        VkShaderModule module{};
        check(vkCreateShaderModule(device, &info, nullptr, &module), "create shader module");
        shaders.push_back(module); return module;
    }
};

void buffer_barrier(VkCommandBuffer command, const Buffer& buffer, VkPipelineStageFlags src_stage,
                    VkAccessFlags src, VkPipelineStageFlags dst_stage, VkAccessFlags dst) {
    auto barrier = structure<VkBufferMemoryBarrier>(VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER);
    barrier.srcAccessMask = src; barrier.dstAccessMask = dst;
    barrier.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED; barrier.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    barrier.buffer = buffer.handle; barrier.size = VK_WHOLE_SIZE;
    vkCmdPipelineBarrier(command, src_stage, dst_stage, 0, 0, nullptr, 1, &barrier, 0, nullptr);
}

void image_barrier(VkCommandBuffer command, VkImage image, VkImageLayout old_layout, VkImageLayout new_layout,
                   VkPipelineStageFlags src_stage, VkAccessFlags src, VkPipelineStageFlags dst_stage, VkAccessFlags dst) {
    auto barrier = structure<VkImageMemoryBarrier>(VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER);
    barrier.oldLayout = old_layout; barrier.newLayout = new_layout; barrier.srcAccessMask = src; barrier.dstAccessMask = dst;
    barrier.srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED; barrier.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    barrier.image = image; barrier.subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
    vkCmdPipelineBarrier(command, src_stage, dst_stage, 0, 0, nullptr, 0, nullptr, 1, &barrier);
}

VkBufferImageCopy image_copy() {
    VkBufferImageCopy copy{}; copy.imageSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1}; copy.imageExtent = {SIDE, SIDE, 1}; return copy;
}

bool transfer_roundtrip(Context& context) {
    std::vector<uint8_t> expected(PIXEL_BYTES);
    for (size_t i = 0; i < expected.size(); ++i) expected[i] = static_cast<uint8_t>((i * 17 + i / 13) & 255);
    const auto source = context.buffer(PIXEL_BYTES, VK_BUFFER_USAGE_TRANSFER_SRC_BIT);
    const auto destination = context.buffer(PIXEL_BYTES, VK_BUFFER_USAGE_TRANSFER_DST_BIT);
    context.write(source, expected.data(), expected.size());
    const auto image = context.image(VK_FORMAT_R8G8B8A8_UNORM, VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT, VK_IMAGE_ASPECT_COLOR_BIT);
    const auto command = context.begin(); const auto copy = image_copy();
    buffer_barrier(command, source, VK_PIPELINE_STAGE_HOST_BIT, VK_ACCESS_HOST_WRITE_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_READ_BIT);
    image_barrier(command, image.handle, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                  VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, 0, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);
    vkCmdCopyBufferToImage(command, source.handle, image.handle, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, 1, &copy);
    image_barrier(command, image.handle, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                  VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_READ_BIT);
    vkCmdCopyImageToBuffer(command, image.handle, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, destination.handle, 1, &copy);
    buffer_barrier(command, destination, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, VK_PIPELINE_STAGE_HOST_BIT, VK_ACCESS_HOST_READ_BIT);
    context.submit(command);
    return context.read(destination) == expected;
}

void generate_vertices(Context& context, VkCommandBuffer command, const Buffer& vertices) {
    VkDescriptorSetLayoutBinding binding{}; binding.binding = 0; binding.descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
    binding.descriptorCount = 1; binding.stageFlags = VK_SHADER_STAGE_COMPUTE_BIT;
    auto layout_info = structure<VkDescriptorSetLayoutCreateInfo>(VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO);
    layout_info.bindingCount = 1; layout_info.pBindings = &binding;
    VkDescriptorSetLayout descriptor_layout{};
    check(vkCreateDescriptorSetLayout(context.device, &layout_info, nullptr, &descriptor_layout), "create compute descriptor layout");
    context.descriptor_layouts.push_back(descriptor_layout);
    VkDescriptorPoolSize size{VK_DESCRIPTOR_TYPE_STORAGE_BUFFER, 1};
    auto pool_info = structure<VkDescriptorPoolCreateInfo>(VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO);
    pool_info.maxSets = 1; pool_info.poolSizeCount = 1; pool_info.pPoolSizes = &size;
    VkDescriptorPool pool{};
    check(vkCreateDescriptorPool(context.device, &pool_info, nullptr, &pool), "create descriptor pool");
    context.descriptor_pools.push_back(pool);
    auto set_info = structure<VkDescriptorSetAllocateInfo>(VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO);
    set_info.descriptorPool = pool; set_info.descriptorSetCount = 1; set_info.pSetLayouts = &descriptor_layout;
    VkDescriptorSet set{};
    check(vkAllocateDescriptorSets(context.device, &set_info, &set), "allocate descriptor set");
    VkDescriptorBufferInfo buffer_info{vertices.handle, 0, vertices.size};
    auto write = structure<VkWriteDescriptorSet>(VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET);
    write.dstSet = set; write.descriptorCount = 1; write.descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER; write.pBufferInfo = &buffer_info;
    vkUpdateDescriptorSets(context.device, 1, &write, 0, nullptr);
    auto pipeline_layout_info = structure<VkPipelineLayoutCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO);
    pipeline_layout_info.setLayoutCount = 1; pipeline_layout_info.pSetLayouts = &descriptor_layout;
    VkPipelineLayout layout{};
    check(vkCreatePipelineLayout(context.device, &pipeline_layout_info, nullptr, &layout), "create compute pipeline layout");
    context.pipeline_layouts.push_back(layout);
    auto pipeline_info = structure<VkComputePipelineCreateInfo>(VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO);
    pipeline_info.layout = layout;
    pipeline_info.stage = structure<VkPipelineShaderStageCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO);
    pipeline_info.stage.stage = VK_SHADER_STAGE_COMPUTE_BIT; pipeline_info.stage.module = context.shader(shaders::vertices_comp); pipeline_info.stage.pName = "main";
    VkPipeline pipeline{};
    const auto result = vkCreateComputePipelines(context.device, VK_NULL_HANDLE, 1, &pipeline_info, nullptr, &pipeline);
    if (pipeline) context.pipelines.push_back(pipeline);
    check(result, "create compute pipeline");
    buffer_barrier(command, vertices, VK_PIPELINE_STAGE_HOST_BIT, VK_ACCESS_HOST_WRITE_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);
    vkCmdBindPipeline(command, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline);
    vkCmdBindDescriptorSets(command, VK_PIPELINE_BIND_POINT_COMPUTE, layout, 0, 1, &set, 0, nullptr);
    vkCmdDispatch(command, 1, 1, 1);
    buffer_barrier(command, vertices, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT,
                   VK_PIPELINE_STAGE_VERTEX_INPUT_BIT, VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT);
}

std::vector<uint8_t> draw_triangles(Context& context, bool compute) {
    const auto vertices = context.buffer(sizeof(VERTICES), VK_BUFFER_USAGE_VERTEX_BUFFER_BIT | VK_BUFFER_USAGE_STORAGE_BUFFER_BIT);
    const std::array<float, 36> empty{};
    context.write(vertices, compute ? empty.data() : VERTICES.data(), sizeof(VERTICES));
    const auto readback = context.buffer(PIXEL_BYTES, VK_BUFFER_USAGE_TRANSFER_DST_BIT);
    const auto color = context.image(VK_FORMAT_R8G8B8A8_UNORM, VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT, VK_IMAGE_ASPECT_COLOR_BIT);
    VkFormat depth_format = VK_FORMAT_UNDEFINED;
    for (const auto candidate : {VK_FORMAT_D32_SFLOAT, VK_FORMAT_D16_UNORM}) {
        VkFormatProperties properties{}; vkGetPhysicalDeviceFormatProperties(context.physical, candidate, &properties);
        if (properties.optimalTilingFeatures & VK_FORMAT_FEATURE_DEPTH_STENCIL_ATTACHMENT_BIT) { depth_format = candidate; break; }
    }
    if (depth_format == VK_FORMAT_UNDEFINED) throw std::runtime_error("No supported test depth attachment format");
    const auto depth = context.image(depth_format, VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT, VK_IMAGE_ASPECT_DEPTH_BIT);
    std::array<VkAttachmentDescription, 2> attachments{};
    attachments[0].format = VK_FORMAT_R8G8B8A8_UNORM; attachments[0].samples = VK_SAMPLE_COUNT_1_BIT;
    attachments[0].loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR; attachments[0].storeOp = VK_ATTACHMENT_STORE_OP_STORE;
    attachments[0].stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE; attachments[0].stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
    attachments[0].finalLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    attachments[1].format = depth_format; attachments[1].samples = VK_SAMPLE_COUNT_1_BIT;
    attachments[1].loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR; attachments[1].storeOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
    attachments[1].stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE; attachments[1].stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
    attachments[1].finalLayout = VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
    const VkAttachmentReference color_ref{0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
    const VkAttachmentReference depth_ref{1, VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL};
    VkSubpassDescription subpass{}; subpass.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
    subpass.colorAttachmentCount = 1; subpass.pColorAttachments = &color_ref; subpass.pDepthStencilAttachment = &depth_ref;
    std::array<VkSubpassDependency, 2> dependencies{};
    dependencies[0].srcSubpass = VK_SUBPASS_EXTERNAL; dependencies[0].dstSubpass = 0;
    dependencies[0].srcStageMask = VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT;
    dependencies[0].dstStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT | VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT;
    dependencies[0].dstAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT | VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT;
    dependencies[1].srcSubpass = 0; dependencies[1].dstSubpass = VK_SUBPASS_EXTERNAL;
    dependencies[1].srcStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT; dependencies[1].dstStageMask = VK_PIPELINE_STAGE_TRANSFER_BIT;
    dependencies[1].srcAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT; dependencies[1].dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT;
    auto render_info = structure<VkRenderPassCreateInfo>(VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO);
    render_info.attachmentCount = attachments.size(); render_info.pAttachments = attachments.data();
    render_info.subpassCount = 1; render_info.pSubpasses = &subpass;
    render_info.dependencyCount = dependencies.size(); render_info.pDependencies = dependencies.data();
    VkRenderPass render_pass{};
    check(vkCreateRenderPass(context.device, &render_info, nullptr, &render_pass), "create render pass");
    context.render_passes.push_back(render_pass);
    std::array<VkImageView, 2> views{color.view, depth.view};
    auto framebuffer_info = structure<VkFramebufferCreateInfo>(VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO);
    framebuffer_info.renderPass = render_pass; framebuffer_info.attachmentCount = views.size(); framebuffer_info.pAttachments = views.data();
    framebuffer_info.width = SIDE; framebuffer_info.height = SIDE; framebuffer_info.layers = 1;
    VkFramebuffer framebuffer{};
    check(vkCreateFramebuffer(context.device, &framebuffer_info, nullptr, &framebuffer), "create framebuffer");
    context.framebuffers.push_back(framebuffer);
    auto layout_info = structure<VkPipelineLayoutCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO);
    VkPipelineLayout layout{};
    check(vkCreatePipelineLayout(context.device, &layout_info, nullptr, &layout), "create graphics pipeline layout");
    context.pipeline_layouts.push_back(layout);
    std::array<VkPipelineShaderStageCreateInfo, 2> stages{};
    for (auto& stage : stages) stage = structure<VkPipelineShaderStageCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO);
    stages[0].stage = VK_SHADER_STAGE_VERTEX_BIT; stages[0].module = context.shader(shaders::triangle_vert); stages[0].pName = "main";
    stages[1].stage = VK_SHADER_STAGE_FRAGMENT_BIT; stages[1].module = context.shader(shaders::triangle_frag); stages[1].pName = "main";
    const VkVertexInputBindingDescription vertex_binding{0, 6 * sizeof(float), VK_VERTEX_INPUT_RATE_VERTEX};
    const std::array<VkVertexInputAttributeDescription, 2> attributes{{{0,0,VK_FORMAT_R32G32B32_SFLOAT,0}, {1,0,VK_FORMAT_R32G32B32_SFLOAT,3 * sizeof(float)}}};
    auto vertex_input = structure<VkPipelineVertexInputStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO);
    vertex_input.vertexBindingDescriptionCount = 1; vertex_input.pVertexBindingDescriptions = &vertex_binding;
    vertex_input.vertexAttributeDescriptionCount = attributes.size(); vertex_input.pVertexAttributeDescriptions = attributes.data();
    auto assembly = structure<VkPipelineInputAssemblyStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO);
    assembly.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
    const VkViewport viewport{0,0,SIDE,SIDE,0,1}; const VkRect2D scissor{{0,0},{SIDE,SIDE}};
    auto viewport_info = structure<VkPipelineViewportStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO);
    viewport_info.viewportCount = 1; viewport_info.pViewports = &viewport; viewport_info.scissorCount = 1; viewport_info.pScissors = &scissor;
    auto raster = structure<VkPipelineRasterizationStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO);
    raster.polygonMode = VK_POLYGON_MODE_FILL; raster.cullMode = VK_CULL_MODE_NONE; raster.lineWidth = 1;
    auto multisample = structure<VkPipelineMultisampleStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO);
    multisample.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
    auto depth_info = structure<VkPipelineDepthStencilStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_DEPTH_STENCIL_STATE_CREATE_INFO);
    depth_info.depthTestEnable = VK_TRUE; depth_info.depthWriteEnable = VK_TRUE; depth_info.depthCompareOp = VK_COMPARE_OP_LESS;
    VkPipelineColorBlendAttachmentState blend_attachment{}; blend_attachment.colorWriteMask = 15;
    auto blend = structure<VkPipelineColorBlendStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO);
    blend.attachmentCount = 1; blend.pAttachments = &blend_attachment;
    auto pipeline_info = structure<VkGraphicsPipelineCreateInfo>(VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO);
    pipeline_info.stageCount = stages.size(); pipeline_info.pStages = stages.data(); pipeline_info.pVertexInputState = &vertex_input;
    pipeline_info.pInputAssemblyState = &assembly; pipeline_info.pViewportState = &viewport_info; pipeline_info.pRasterizationState = &raster;
    pipeline_info.pMultisampleState = &multisample; pipeline_info.pDepthStencilState = &depth_info; pipeline_info.pColorBlendState = &blend;
    pipeline_info.layout = layout; pipeline_info.renderPass = render_pass;
    VkPipeline pipeline{};
    const auto result = vkCreateGraphicsPipelines(context.device, VK_NULL_HANDLE, 1, &pipeline_info, nullptr, &pipeline);
    if (pipeline) context.pipelines.push_back(pipeline);
    check(result, "create graphics pipeline");
    const auto command = context.begin();
    if (compute) generate_vertices(context, command, vertices);
    else buffer_barrier(command, vertices, VK_PIPELINE_STAGE_HOST_BIT, VK_ACCESS_HOST_WRITE_BIT, VK_PIPELINE_STAGE_VERTEX_INPUT_BIT, VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT);
    std::array<VkClearValue, 2> clears{}; clears[0].color.float32[3] = 1; clears[1].depthStencil.depth = 1;
    auto begin = structure<VkRenderPassBeginInfo>(VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO);
    begin.renderPass = render_pass; begin.framebuffer = framebuffer; begin.renderArea = {{0,0},{SIDE,SIDE}};
    begin.clearValueCount = clears.size(); begin.pClearValues = clears.data();
    vkCmdBeginRenderPass(command, &begin, VK_SUBPASS_CONTENTS_INLINE);
    vkCmdBindPipeline(command, VK_PIPELINE_BIND_POINT_GRAPHICS, pipeline);
    const VkDeviceSize offset = 0;
    vkCmdBindVertexBuffers(command, 0, 1, &vertices.handle, &offset);
    vkCmdDraw(command, 6, 1, 0, 0);
    vkCmdEndRenderPass(command);
    const auto copy = image_copy();
    vkCmdCopyImageToBuffer(command, color.handle, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, readback.handle, 1, &copy);
    buffer_barrier(command, readback, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, VK_PIPELINE_STAGE_HOST_BIT, VK_ACCESS_HOST_READ_BIT);
    context.submit(command);
    return context.read(readback);
}

bool expected_triangle(const std::vector<uint8_t>& bytes) {
    const size_t center = (SIDE / 2 * SIDE + SIDE / 2) * 4;
    return bytes.size() == PIXEL_BYTES && bytes[center] == 0 && bytes[center + 1] == 255 &&
           bytes[center + 2] == 0 && bytes[center + 3] == 255 &&
           bytes[0] == 0 && bytes[1] == 0 && bytes[2] == 0 && bytes[3] == 255;
}

void append_capabilities(std::ostream& json, const Context& context) {
    json << ",\"deviceAvailable\":" << (context.physical ? "true" : "false");
    const auto& limits = context.properties.limits;
    json << ",\"limits\":{\"maxImageDimension2D\":" << limits.maxImageDimension2D
         << ",\"maxStorageBufferRange\":" << limits.maxStorageBufferRange
         << ",\"maxComputeSharedMemorySize\":" << limits.maxComputeSharedMemorySize
         << ",\"maxComputeWorkGroupInvocations\":" << limits.maxComputeWorkGroupInvocations
         << ",\"maxMemoryAllocationCount\":" << limits.maxMemoryAllocationCount
         << ",\"nonCoherentAtomSize\":" << limits.nonCoherentAtomSize << '}';
    json << ",\"memoryHeaps\":[";
    for (uint32_t index = 0; index < context.memory_properties.memoryHeapCount; ++index) {
        if (index) json << ',';
        const auto& heap = context.memory_properties.memoryHeaps[index];
        json << "{\"sizeBytes\":" << heap.size << ",\"deviceLocal\":"
             << ((heap.flags & VK_MEMORY_HEAP_DEVICE_LOCAL_BIT) ? "true" : "false") << '}';
    }
    json << "],\"queueFamilies\":[";
    if (context.physical) {
        uint32_t count = 0;
        vkGetPhysicalDeviceQueueFamilyProperties(context.physical, &count, nullptr);
        std::vector<VkQueueFamilyProperties> queues(count);
        vkGetPhysicalDeviceQueueFamilyProperties(context.physical, &count, queues.data());
        for (uint32_t index = 0; index < count; ++index) {
            if (index) json << ',';
            json << "{\"index\":" << index << ",\"count\":" << queues[index].queueCount
                 << ",\"flagsRaw\":" << queues[index].queueFlags << '}';
        }
    }
    json << "],\"formats\":[";
    const std::array<std::pair<const char*, VkFormat>, 11> formats{{
        {"RGBA8_UNORM", VK_FORMAT_R8G8B8A8_UNORM}, {"RGBA8_SRGB", VK_FORMAT_R8G8B8A8_SRGB},
        {"BGRA8_UNORM", VK_FORMAT_B8G8R8A8_UNORM}, {"RGBA16_SFLOAT", VK_FORMAT_R16G16B16A16_SFLOAT},
        {"D16_UNORM", VK_FORMAT_D16_UNORM}, {"D32_SFLOAT", VK_FORMAT_D32_SFLOAT},
        {"D24_UNORM_S8_UINT", VK_FORMAT_D24_UNORM_S8_UINT},
        {"BC1_RGBA_UNORM", VK_FORMAT_BC1_RGBA_UNORM_BLOCK}, {"BC3_UNORM", VK_FORMAT_BC3_UNORM_BLOCK},
        {"BC7_UNORM", VK_FORMAT_BC7_UNORM_BLOCK}, {"ASTC_4x4_UNORM", VK_FORMAT_ASTC_4x4_UNORM_BLOCK}
    }};
    if (context.physical) for (size_t index = 0; index < formats.size(); ++index) {
        if (index) json << ',';
        const auto& [name, format] = formats[index];
        VkFormatProperties properties{};
        vkGetPhysicalDeviceFormatProperties(context.physical, format, &properties);
        VkImageFormatProperties image{};
        const auto result = vkGetPhysicalDeviceImageFormatProperties(context.physical, format,
            VK_IMAGE_TYPE_2D, VK_IMAGE_TILING_OPTIMAL,
            VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT, 0, &image);
        json << "{\"name\":" << quoted(name) << ",\"optimalFeaturesRaw\":" << properties.optimalTilingFeatures
             << ",\"sampledTransfer2D\":" << (result == VK_SUCCESS ? "true" : "false")
             << ",\"imageQueryResult\":" << result << '}';
    }
    json << ']';
}
} // namespace

GpuReport run_gpu_checks() {
    Context context;
    std::vector<Test> tests;
    std::string error;
    try {
        context.initialize();
        tests.push_back({"rgba8-transfer-roundtrip", transfer_roundtrip(context), "Exact buffer/image/buffer byte comparison"});
        const auto direct = draw_triangles(context, false);
        tests.push_back({"depth-tested-triangles", expected_triangle(direct), "Front green triangle must hide a later red triangle; corner remains clear"});
        const auto computed = draw_triangles(context, true);
        tests.push_back({"compute-to-vertex-visibility", expected_triangle(computed) && computed == direct, "GPU-generated vertices must match every byte of the CPU-uploaded reference draw"});
    } catch (const std::exception& failure) { error = failure.what(); }
    // Device queries must finish before the instance and physical-device handle are destroyed.
    std::ostringstream capabilities;
    append_capabilities(capabilities, context);
    context.shutdown();
    const bool passed = error.empty() && context.validation_errors == 0 && tests.size() == 3 && std::all_of(tests.begin(), tests.end(), [](const Test& test) { return test.passed; });
    std::ostringstream json;
    json << "{\"schemaVersion\":1,\"scope\":\"host-vulkan-only\",\"deviceName\":" << quoted(context.properties.deviceName)
         << ",\"validationEnabled\":" << (context.validation_enabled ? "true" : "false") << ",\"validationErrors\":" << context.validation_errors.load()
         << ",\"vendorId\":" << context.properties.vendorID << ",\"deviceId\":" << context.properties.deviceID
         << ",\"driverVersionRaw\":" << context.properties.driverVersion << ",\"apiVersionRaw\":" << context.properties.apiVersion
         << ",\"deviceType\":" << context.properties.deviceType
         << ",\"isSoftwareRenderer\":" << (context.properties.deviceType == VK_PHYSICAL_DEVICE_TYPE_CPU ? "true" : "false")
         << ",\"features\":{\"textureCompressionBC\":" << (context.features.textureCompressionBC ? "true" : "false")
         << ",\"textureCompressionASTC_LDR\":" << (context.features.textureCompressionASTC_LDR ? "true" : "false")
         << ",\"geometryShader\":" << (context.features.geometryShader ? "true" : "false")
         << ",\"tessellationShader\":" << (context.features.tessellationShader ? "true" : "false") << '}'
         << capabilities.str() << ",\"tests\":[";
    for (size_t index = 0; index < tests.size(); ++index) {
        if (index) json << ',';
        json << "{\"name\":" << quoted(tests[index].name) << ",\"passed\":" << (tests[index].passed ? "true" : "false") << ",\"detail\":" << quoted(tests[index].detail) << '}';
    }
    json << "],\"allPassed\":" << (passed ? "true" : "false") << ",\"error\":" << quoted(error) << '}';
    return {passed, json.str()};
}
