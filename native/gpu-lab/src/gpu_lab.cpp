// Original Vulkan diagnostics and synchronous live guest graphics backend.
#include "gpu_lab.h"
#include "engine_abi.h"
#include "shaders.h"
#include "bc1_fixture.h"
#ifdef __ANDROID__
#define VK_USE_PLATFORM_ANDROID_KHR
#include <android/native_window.h>
#endif
#include <vulkan/vulkan.h>
#include <memory>
#include <algorithm>
#include <array>
#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <sstream>
#include <stdexcept>
#include <vector>
#include <type_traits>

namespace {
constexpr uint32_t SIDE = 64;
constexpr size_t PIXEL_BYTES = SIDE * SIDE * 4;
static_assert(bc1_fixture::rgba8.size() == SIDE * SIDE);
static_assert(bc1_fixture::updated_rgba8.size() == SIDE * SIDE);
static_assert(bc1_fixture::input.size() * sizeof(uint32_t) == (SIDE / 4) * (SIDE / 4) * 8);
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
    VkSurfaceKHR surface{};
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
    std::vector<VkSampler> samplers;
    uint64_t allocation_bytes = 0;
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
            clear_workloads();
            if (command_pool) vkDestroyCommandPool(device, command_pool, nullptr);
            vkDestroyDevice(device, nullptr);
            device = VK_NULL_HANDLE;
        }
        if (instance) {
            if (surface) { vkDestroySurfaceKHR(instance, surface, nullptr); surface = VK_NULL_HANDLE; }
            if (debug_messenger) {
                const auto destroy = reinterpret_cast<PFN_vkDestroyDebugUtilsMessengerEXT>(vkGetInstanceProcAddr(instance, "vkDestroyDebugUtilsMessengerEXT"));
                if (destroy) destroy(instance, debug_messenger, nullptr);
                debug_messenger = VK_NULL_HANDLE;
            }
            vkDestroyInstance(instance, nullptr);
            instance = VK_NULL_HANDLE;
        }
    }

    void clear_workloads() {
        for (auto handle : fences) vkDestroyFence(device, handle, nullptr);
        for (auto handle : pipelines) vkDestroyPipeline(device, handle, nullptr);
        for (auto handle : framebuffers) vkDestroyFramebuffer(device, handle, nullptr);
        for (auto handle : render_passes) vkDestroyRenderPass(device, handle, nullptr);
        for (auto handle : pipeline_layouts) vkDestroyPipelineLayout(device, handle, nullptr);
        for (auto handle : descriptor_pools) vkDestroyDescriptorPool(device, handle, nullptr);
        for (auto handle : descriptor_layouts) vkDestroyDescriptorSetLayout(device, handle, nullptr);
        for (auto handle : shaders) vkDestroyShaderModule(device, handle, nullptr);
        for (auto handle : samplers) vkDestroySampler(device, handle, nullptr);
        for (auto handle : views) vkDestroyImageView(device, handle, nullptr);
        for (auto handle : images) vkDestroyImage(device, handle, nullptr);
        for (auto handle : buffers) vkDestroyBuffer(device, handle, nullptr);
        for (auto handle : allocations) vkFreeMemory(device, handle, nullptr);

        fences.clear();
        pipelines.clear();
        framebuffers.clear();
        render_passes.clear();
        pipeline_layouts.clear();
        descriptor_pools.clear();
        descriptor_layouts.clear();
        shaders.clear();
        samplers.clear();
        views.clear();
        images.clear();
        buffers.clear();
        allocations.clear();
        allocation_bytes = 0;

    }

    static VKAPI_ATTR VkBool32 VKAPI_CALL validation_message(VkDebugUtilsMessageSeverityFlagBitsEXT severity,
            VkDebugUtilsMessageTypeFlagsEXT, const VkDebugUtilsMessengerCallbackDataEXT* message, void* user) {
        auto* context = static_cast<Context*>(user);
        if (severity & VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT) ++context->validation_errors;
        std::fprintf(stderr, "Vulkan validation: %s\n", message->pMessage);
        return VK_FALSE;
    }

    void initialize(void* native_window = nullptr) {
        auto app = structure<VkApplicationInfo>(VK_STRUCTURE_TYPE_APPLICATION_INFO);
        app.pApplicationName = "Original Emulator GPU Lab";
        app.apiVersion = VK_API_VERSION_1_1;
        auto instance_info = structure<VkInstanceCreateInfo>(VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO);
        instance_info.pApplicationInfo = &app;
        const char* layer = "VK_LAYER_KHRONOS_validation";
        std::vector<const char*> instance_extensions;
#ifdef __ANDROID__
        if (native_window) instance_extensions = {VK_KHR_SURFACE_EXTENSION_NAME, VK_KHR_ANDROID_SURFACE_EXTENSION_NAME};
#else
        if (native_window) throw std::runtime_error("Native window is Android-only");
#endif
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
            instance_extensions.push_back(VK_EXT_DEBUG_UTILS_EXTENSION_NAME);
            debug.messageSeverity = VK_DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT;
            debug.messageType = VK_DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT | VK_DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT;
            debug.pfnUserCallback = validation_message; debug.pUserData = this;
            instance_info.pNext = &debug;
        }
        instance_info.enabledExtensionCount = static_cast<uint32_t>(instance_extensions.size());
        instance_info.ppEnabledExtensionNames = instance_extensions.data();
        check(vkCreateInstance(&instance_info, nullptr, &instance), "create instance");
#ifdef __ANDROID__
        if (native_window) {
            auto info = structure<VkAndroidSurfaceCreateInfoKHR>(VK_STRUCTURE_TYPE_ANDROID_SURFACE_CREATE_INFO_KHR);
            info.window = static_cast<ANativeWindow*>(native_window);
            check(vkCreateAndroidSurfaceKHR(instance, &info, nullptr, &surface), "create Android surface");
        }
#endif
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
            VkBool32 present = VK_TRUE;
            if (surface) check(vkGetPhysicalDeviceSurfaceSupportKHR(physical, index, surface, &present), "query present queue");
            if (present && queues[index].queueCount && (queues[index].queueFlags & required) == required) { family = index; found = true; break; }
        }
        if (!found) throw std::runtime_error("No combined graphics/compute queue for this test backend");
        const float priority = 1.0f;
        auto queue_info = structure<VkDeviceQueueCreateInfo>(VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO);
        queue_info.queueFamilyIndex = family; queue_info.queueCount = 1; queue_info.pQueuePriorities = &priority;
        auto device_info = structure<VkDeviceCreateInfo>(VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO);
        device_info.queueCreateInfoCount = 1; device_info.pQueueCreateInfos = &queue_info;
        const char* swapchain_extension = VK_KHR_SWAPCHAIN_EXTENSION_NAME;
        if (surface) { device_info.enabledExtensionCount = 1; device_info.ppEnabledExtensionNames = &swapchain_extension; }
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
        allocation_bytes += requirements.size;
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

    Image image(VkFormat format, VkImageUsageFlags usage, VkImageAspectFlags aspect, uint32_t width = SIDE, uint32_t height = SIDE) {
        VkImageFormatProperties supported{};
        check(vkGetPhysicalDeviceImageFormatProperties(physical, format, VK_IMAGE_TYPE_2D,
              VK_IMAGE_TILING_OPTIMAL, usage, 0, &supported), "query image format/usage");
        if (supported.maxExtent.width < width || supported.maxExtent.height < height) throw std::runtime_error("Image extent unsupported");
        Image output{};
        auto info = structure<VkImageCreateInfo>(VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO);
        info.imageType = VK_IMAGE_TYPE_2D; info.format = format; info.extent = {width, height, 1};
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
        allocation_bytes += requirements.size;
        check(vkBindImageMemory(device, output.handle, memory, 0), "bind image memory");
        if (usage & (VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT | VK_IMAGE_USAGE_STORAGE_BIT | VK_IMAGE_USAGE_SAMPLED_BIT)) {
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

    template<size_t Size> VkShaderModule shader(const std::array<uint32_t, Size>& words) { return shader(words.data(),words.size()); }
    VkShaderModule shader(const uint32_t* words,size_t count) {
        auto info = structure<VkShaderModuleCreateInfo>(VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO);
        info.codeSize = count * sizeof(uint32_t); info.pCode = words;
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

struct ComputePipeline { VkPipeline pipeline; VkPipelineLayout layout; VkDescriptorSet set; };

template<size_t Size> ComputePipeline compute_pipeline(Context& context,
        const std::vector<VkDescriptorSetLayoutBinding>& bindings, const std::array<uint32_t, Size>& code) {
    auto layout_info = structure<VkDescriptorSetLayoutCreateInfo>(VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO);
    layout_info.bindingCount = static_cast<uint32_t>(bindings.size()); layout_info.pBindings = bindings.data();
    VkDescriptorSetLayout descriptor_layout{};
    check(vkCreateDescriptorSetLayout(context.device, &layout_info, nullptr, &descriptor_layout), "create compute descriptor layout");
    context.descriptor_layouts.push_back(descriptor_layout);
    std::vector<VkDescriptorPoolSize> sizes;
    for (const auto& binding : bindings) sizes.push_back({binding.descriptorType, binding.descriptorCount});
    auto pool_info = structure<VkDescriptorPoolCreateInfo>(VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO);
    pool_info.maxSets = 1; pool_info.poolSizeCount = static_cast<uint32_t>(sizes.size()); pool_info.pPoolSizes = sizes.data();
    VkDescriptorPool pool{};
    check(vkCreateDescriptorPool(context.device, &pool_info, nullptr, &pool), "create descriptor pool");
    context.descriptor_pools.push_back(pool);
    auto set_info = structure<VkDescriptorSetAllocateInfo>(VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO);
    set_info.descriptorPool = pool; set_info.descriptorSetCount = 1; set_info.pSetLayouts = &descriptor_layout;
    VkDescriptorSet set{};
    check(vkAllocateDescriptorSets(context.device, &set_info, &set), "allocate descriptor set");
    auto pipeline_layout_info = structure<VkPipelineLayoutCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO);
    pipeline_layout_info.setLayoutCount = 1; pipeline_layout_info.pSetLayouts = &descriptor_layout;
    VkPipelineLayout layout{};
    check(vkCreatePipelineLayout(context.device, &pipeline_layout_info, nullptr, &layout), "create compute pipeline layout");
    context.pipeline_layouts.push_back(layout);
    auto pipeline_info = structure<VkComputePipelineCreateInfo>(VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO);
    pipeline_info.layout = layout;
    pipeline_info.stage = structure<VkPipelineShaderStageCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO);
    pipeline_info.stage.stage = VK_SHADER_STAGE_COMPUTE_BIT; pipeline_info.stage.module = context.shader(code); pipeline_info.stage.pName = "main";
    VkPipeline pipeline{};
    const auto result = vkCreateComputePipelines(context.device, VK_NULL_HANDLE, 1, &pipeline_info, nullptr, &pipeline);
    if (pipeline) context.pipelines.push_back(pipeline);
    check(result, "create compute pipeline");
    return {pipeline, layout, set};
}

void generate_vertices(Context& context, VkCommandBuffer command, const Buffer& vertices) {
    VkDescriptorSetLayoutBinding binding{}; binding.binding = 0; binding.descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
    binding.descriptorCount = 1; binding.stageFlags = VK_SHADER_STAGE_COMPUTE_BIT;
    const auto compute = compute_pipeline(context, {binding}, shaders::vertices_comp);
    VkDescriptorBufferInfo buffer_info{vertices.handle, 0, vertices.size};
    auto write = structure<VkWriteDescriptorSet>(VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET);
    write.dstSet = compute.set; write.descriptorCount = 1; write.descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER; write.pBufferInfo = &buffer_info;
    vkUpdateDescriptorSets(context.device, 1, &write, 0, nullptr);
    buffer_barrier(command, vertices, VK_PIPELINE_STAGE_HOST_BIT, VK_ACCESS_HOST_WRITE_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);
    vkCmdBindPipeline(command, VK_PIPELINE_BIND_POINT_COMPUTE, compute.pipeline);
    vkCmdBindDescriptorSets(command, VK_PIPELINE_BIND_POINT_COMPUTE, compute.layout, 0, 1, &compute.set, 0, nullptr);
    vkCmdDispatch(command, 1, 1, 1);
    buffer_barrier(command, vertices, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT,
                   VK_PIPELINE_STAGE_VERTEX_INPUT_BIT, VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT);
}

struct Bc1Decoder { Buffer blocks; Image output; ComputePipeline compute; };

Bc1Decoder bc1_decoder(Context& context, const Image* shared_image = nullptr, bool sampled = false) {
    const auto blocks = context.buffer(sizeof(bc1_fixture::input), VK_BUFFER_USAGE_STORAGE_BUFFER_BIT);
    const auto output = shared_image ? *shared_image : context.image(VK_FORMAT_R8G8B8A8_UNORM,
        VK_IMAGE_USAGE_STORAGE_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT | (sampled ? VK_IMAGE_USAGE_SAMPLED_BIT : 0), VK_IMAGE_ASPECT_COLOR_BIT);
    std::vector<VkDescriptorSetLayoutBinding> bindings(2);
    bindings[0].binding = 0; bindings[0].descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
    bindings[1].binding = 1; bindings[1].descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_IMAGE;
    for (auto& binding : bindings) { binding.descriptorCount = 1; binding.stageFlags = VK_SHADER_STAGE_COMPUTE_BIT; }
    const auto compute = compute_pipeline(context, bindings, shaders::bc1_comp);
    VkDescriptorBufferInfo buffer_info{blocks.handle, 0, blocks.size};
    VkDescriptorImageInfo image_info{}; image_info.imageView = output.view; image_info.imageLayout = VK_IMAGE_LAYOUT_GENERAL;
    std::array<VkWriteDescriptorSet, 2> writes{};
    for (size_t index = 0; index < writes.size(); ++index) {
        writes[index] = structure<VkWriteDescriptorSet>(VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET);
        writes[index].dstSet = compute.set; writes[index].dstBinding = static_cast<uint32_t>(index);
        writes[index].descriptorCount = 1; writes[index].descriptorType = bindings[index].descriptorType;
    }
    writes[0].pBufferInfo = &buffer_info; writes[1].pImageInfo = &image_info;
    vkUpdateDescriptorSets(context.device, static_cast<uint32_t>(writes.size()), writes.data(), 0, nullptr);
    return {blocks, output, compute};
}

template<size_t Size> std::vector<uint8_t> word_bytes(const std::array<uint32_t, Size>& words) {
    std::vector<uint8_t> bytes(Size * 4);
    for (size_t index = 0; index < bytes.size(); ++index)
        bytes[index] = static_cast<uint8_t>(words[index / 4] >> (8 * (index % 4)));
    return bytes;
}

void decode_bc1(VkCommandBuffer command, const Bc1Decoder& decoder, bool sampled_before) {
    buffer_barrier(command, decoder.blocks, VK_PIPELINE_STAGE_HOST_BIT, VK_ACCESS_HOST_WRITE_BIT,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    image_barrier(command, decoder.output.handle,
        sampled_before ? VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL : VK_IMAGE_LAYOUT_UNDEFINED,
        VK_IMAGE_LAYOUT_GENERAL,
        sampled_before ? VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT : VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        sampled_before ? VK_ACCESS_SHADER_READ_BIT : 0,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);
    vkCmdBindPipeline(command, VK_PIPELINE_BIND_POINT_COMPUTE, decoder.compute.pipeline);
    vkCmdBindDescriptorSets(command, VK_PIPELINE_BIND_POINT_COMPUTE, decoder.compute.layout, 0, 1, &decoder.compute.set, 0, nullptr);
    vkCmdDispatch(command, (SIDE + 7) / 8, (SIDE + 7) / 8, 1);
}

bool bc1_conversion(Context& context) {
    const auto decoder = bc1_decoder(context);
    const auto input = word_bytes(bc1_fixture::input);
    context.write(decoder.blocks, input.data(), input.size());
    const auto readback = context.buffer(PIXEL_BYTES, VK_BUFFER_USAGE_TRANSFER_DST_BIT);
    const auto command = context.begin();
    decode_bc1(command, decoder, false);
    image_barrier(command, decoder.output.handle, VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_READ_BIT);
    const auto copy = image_copy();
    vkCmdCopyImageToBuffer(command, decoder.output.handle, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, readback.handle, 1, &copy);
    buffer_barrier(command, readback, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT,
        VK_PIPELINE_STAGE_HOST_BIT, VK_ACCESS_HOST_READ_BIT);
    context.submit(command);
    return context.read(readback) == word_bytes(bc1_fixture::rgba8) && context.read(decoder.blocks) == input;
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

std::array<bool, 2> sampled_bc1_updates(Context& context) {
    // Two descriptor sets keep the original and updated encoded data immutable
    // while both dispatch/draw pairs reuse one decoded image in one submission.
    const auto original = bc1_decoder(context, nullptr, true);
    const auto updated = bc1_decoder(context, &original.output, true);
    const auto first_input = word_bytes(bc1_fixture::input);
    const auto second_input = word_bytes(bc1_fixture::updated_input);
    context.write(original.blocks, first_input.data(), first_input.size());
    context.write(updated.blocks, second_input.data(), second_input.size());
    const auto color = context.image(VK_FORMAT_R8G8B8A8_UNORM,
        VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT, VK_IMAGE_ASPECT_COLOR_BIT);
    const auto readback = context.buffer(2 * PIXEL_BYTES, VK_BUFFER_USAGE_TRANSFER_DST_BIT);

    auto sampler_info = structure<VkSamplerCreateInfo>(VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO);
    sampler_info.magFilter = VK_FILTER_NEAREST; sampler_info.minFilter = VK_FILTER_NEAREST;
    sampler_info.mipmapMode = VK_SAMPLER_MIPMAP_MODE_NEAREST;
    sampler_info.addressModeU = sampler_info.addressModeV = sampler_info.addressModeW = VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE;
    VkSampler sampler{};
    check(vkCreateSampler(context.device, &sampler_info, nullptr, &sampler), "create nearest texture sampler");
    context.samplers.push_back(sampler);
    VkDescriptorSetLayoutBinding binding{}; binding.descriptorType = VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER;
    binding.descriptorCount = 1; binding.stageFlags = VK_SHADER_STAGE_FRAGMENT_BIT;
    auto descriptor_info = structure<VkDescriptorSetLayoutCreateInfo>(VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO);
    descriptor_info.bindingCount = 1; descriptor_info.pBindings = &binding;
    VkDescriptorSetLayout descriptor_layout{};
    check(vkCreateDescriptorSetLayout(context.device, &descriptor_info, nullptr, &descriptor_layout), "create texture descriptor layout");
    context.descriptor_layouts.push_back(descriptor_layout);
    VkDescriptorPoolSize pool_size{VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, 1};
    auto pool_info = structure<VkDescriptorPoolCreateInfo>(VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO);
    pool_info.maxSets = 1; pool_info.poolSizeCount = 1; pool_info.pPoolSizes = &pool_size;
    VkDescriptorPool pool{};
    check(vkCreateDescriptorPool(context.device, &pool_info, nullptr, &pool), "create texture descriptor pool");
    context.descriptor_pools.push_back(pool);
    auto set_info = structure<VkDescriptorSetAllocateInfo>(VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO);
    set_info.descriptorPool = pool; set_info.descriptorSetCount = 1; set_info.pSetLayouts = &descriptor_layout;
    VkDescriptorSet descriptor{};
    check(vkAllocateDescriptorSets(context.device, &set_info, &descriptor), "allocate texture descriptor");
    VkDescriptorImageInfo sampled_image{sampler, original.output.view, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL};
    auto write = structure<VkWriteDescriptorSet>(VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET);
    write.dstSet = descriptor; write.descriptorCount = 1;
    write.descriptorType = VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER; write.pImageInfo = &sampled_image;
    vkUpdateDescriptorSets(context.device, 1, &write, 0, nullptr);
    auto layout_info = structure<VkPipelineLayoutCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO);
    layout_info.setLayoutCount = 1; layout_info.pSetLayouts = &descriptor_layout;
    VkPipelineLayout layout{};
    check(vkCreatePipelineLayout(context.device, &layout_info, nullptr, &layout), "create texture pipeline layout");
    context.pipeline_layouts.push_back(layout);

    VkAttachmentDescription attachment{}; attachment.format = VK_FORMAT_R8G8B8A8_UNORM;
    attachment.samples = VK_SAMPLE_COUNT_1_BIT; attachment.loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR;
    attachment.storeOp = VK_ATTACHMENT_STORE_OP_STORE;
    attachment.stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE; attachment.stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
    attachment.initialLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
    attachment.finalLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    const VkAttachmentReference reference{0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
    VkSubpassDescription subpass{}; subpass.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
    subpass.colorAttachmentCount = 1; subpass.pColorAttachments = &reference;
    std::array<VkSubpassDependency, 2> dependencies{};
    dependencies[0].srcSubpass = VK_SUBPASS_EXTERNAL; dependencies[0].dstSubpass = 0;
    dependencies[0].srcStageMask = dependencies[0].dstStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
    dependencies[0].dstAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
    dependencies[1].srcSubpass = 0; dependencies[1].dstSubpass = VK_SUBPASS_EXTERNAL;
    dependencies[1].srcStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
    dependencies[1].srcAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
    dependencies[1].dstStageMask = VK_PIPELINE_STAGE_TRANSFER_BIT; dependencies[1].dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT;
    auto pass_info = structure<VkRenderPassCreateInfo>(VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO);
    pass_info.attachmentCount = 1; pass_info.pAttachments = &attachment;
    pass_info.subpassCount = 1; pass_info.pSubpasses = &subpass;
    pass_info.dependencyCount = static_cast<uint32_t>(dependencies.size()); pass_info.pDependencies = dependencies.data();
    VkRenderPass pass{};
    check(vkCreateRenderPass(context.device, &pass_info, nullptr, &pass), "create texture render pass");
    context.render_passes.push_back(pass);
    auto framebuffer_info = structure<VkFramebufferCreateInfo>(VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO);
    framebuffer_info.renderPass = pass; framebuffer_info.attachmentCount = 1; framebuffer_info.pAttachments = &color.view;
    framebuffer_info.width = SIDE; framebuffer_info.height = SIDE; framebuffer_info.layers = 1;
    VkFramebuffer framebuffer{};
    check(vkCreateFramebuffer(context.device, &framebuffer_info, nullptr, &framebuffer), "create texture framebuffer");
    context.framebuffers.push_back(framebuffer);
    std::array<VkPipelineShaderStageCreateInfo, 2> stages{};
    for (auto& stage : stages) { stage = structure<VkPipelineShaderStageCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO); stage.pName = "main"; }
    stages[0].stage = VK_SHADER_STAGE_VERTEX_BIT; stages[0].module = context.shader(shaders::texture_vert);
    stages[1].stage = VK_SHADER_STAGE_FRAGMENT_BIT; stages[1].module = context.shader(shaders::texture_frag);
    auto vertex_input = structure<VkPipelineVertexInputStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO);
    auto assembly = structure<VkPipelineInputAssemblyStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO);
    assembly.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
    const VkViewport viewport{0, 0, SIDE, SIDE, 0, 1}; const VkRect2D scissor{{0, 0}, {SIDE, SIDE}};
    auto viewport_info = structure<VkPipelineViewportStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO);
    viewport_info.viewportCount = 1; viewport_info.pViewports = &viewport; viewport_info.scissorCount = 1; viewport_info.pScissors = &scissor;
    auto raster = structure<VkPipelineRasterizationStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO);
    raster.polygonMode = VK_POLYGON_MODE_FILL; raster.cullMode = VK_CULL_MODE_NONE; raster.lineWidth = 1;
    auto multisample = structure<VkPipelineMultisampleStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO);
    multisample.rasterizationSamples = VK_SAMPLE_COUNT_1_BIT;
    VkPipelineColorBlendAttachmentState blend_attachment{}; blend_attachment.colorWriteMask = 15;
    auto blend = structure<VkPipelineColorBlendStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO);
    blend.attachmentCount = 1; blend.pAttachments = &blend_attachment;
    auto pipeline_info = structure<VkGraphicsPipelineCreateInfo>(VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO);
    pipeline_info.stageCount = static_cast<uint32_t>(stages.size()); pipeline_info.pStages = stages.data();
    pipeline_info.pVertexInputState = &vertex_input; pipeline_info.pInputAssemblyState = &assembly;
    pipeline_info.pViewportState = &viewport_info; pipeline_info.pRasterizationState = &raster;
    pipeline_info.pMultisampleState = &multisample; pipeline_info.pColorBlendState = &blend;
    pipeline_info.layout = layout; pipeline_info.renderPass = pass;
    VkPipeline pipeline{};
    const auto created = vkCreateGraphicsPipelines(context.device, VK_NULL_HANDLE, 1, &pipeline_info, nullptr, &pipeline);
    if (pipeline) context.pipelines.push_back(pipeline);
    check(created, "create texture sampling pipeline");

    const auto command = context.begin();
    for (int iteration = 0; iteration < 2; ++iteration) {
        decode_bc1(command, iteration == 0 ? original : updated, iteration != 0);
        image_barrier(command, original.output.handle, VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
        image_barrier(command, color.handle, iteration == 0 ? VK_IMAGE_LAYOUT_UNDEFINED : VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            iteration == 0 ? VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT : VK_PIPELINE_STAGE_TRANSFER_BIT,
            iteration == 0 ? 0 : VK_ACCESS_TRANSFER_READ_BIT,
            VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT);
        VkClearValue clear{}; clear.color.float32[0] = clear.color.float32[2] = clear.color.float32[3] = 1;
        auto begin = structure<VkRenderPassBeginInfo>(VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO);
        begin.renderPass = pass; begin.framebuffer = framebuffer; begin.renderArea = scissor;
        begin.clearValueCount = 1; begin.pClearValues = &clear;
        vkCmdBeginRenderPass(command, &begin, VK_SUBPASS_CONTENTS_INLINE);
        vkCmdBindPipeline(command, VK_PIPELINE_BIND_POINT_GRAPHICS, pipeline);
        vkCmdBindDescriptorSets(command, VK_PIPELINE_BIND_POINT_GRAPHICS, layout, 0, 1, &descriptor, 0, nullptr);
        vkCmdDraw(command, 3, 1, 0, 0);
        vkCmdEndRenderPass(command);
        auto copy = image_copy(); copy.bufferOffset = static_cast<VkDeviceSize>(iteration) * PIXEL_BYTES;
        vkCmdCopyImageToBuffer(command, color.handle, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, readback.handle, 1, &copy);
    }
    buffer_barrier(command, readback, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT,
        VK_PIPELINE_STAGE_HOST_BIT, VK_ACCESS_HOST_READ_BIT);
    context.submit(command);
    const auto actual = context.read(readback);
    std::array<bool, 2> results{true, true};
    for (size_t iteration = 0; iteration < 2; ++iteration) {
        const auto& expected = iteration == 0 ? bc1_fixture::rgba8 : bc1_fixture::updated_rgba8;
        for (int y = 0; y < static_cast<int>(SIDE); ++y) for (int x = 0; x < static_cast<int>(SIDE); ++x) {
            // Integer oracle for nearest sampling at pixel centers; positions
            // are a quarter texel away from boundaries, including both clamps.
            const int sx = std::clamp((5 * static_cast<int>(SIDE) - 6 * x - 3) / 4, 0, static_cast<int>(SIDE) - 1);
            const int sy = std::clamp((6 * y + 3 - static_cast<int>(SIDE)) / 4, 0, static_cast<int>(SIDE) - 1);
            const auto rgba = expected[sy * SIDE + sx];
            const size_t offset = iteration * PIXEL_BYTES + (y * SIDE + x) * 4;
            for (size_t channel = 0; channel < 4; ++channel)
                if (actual[offset + channel] != static_cast<uint8_t>(rgba >> (8 * channel))) results[iteration] = false;
        }
    }
    results[0] = results[0] && context.read(original.blocks) == first_input;
    results[1] = results[1] && context.read(updated.blocks) == second_input;
    return results;
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
// Replay validated original guest clears using a fixed host shader for masked
// attachment writes. This translates clear operations, not guest shader code.
bool guest_clear_replay(Context& context, uint32_t index) {
    const auto replay = emulator_guest_clear_replay_v1(index);
    if (replay.abi_version != 1 || replay.status != 0 || replay.operation_count != 3 || replay.image_bytes != 512)
        throw std::runtime_error(std::string("Owned clear replay failed: ") + std::string(replay.error, strnlen(replay.error, sizeof(replay.error))));
    const auto& first = replay.operations[0];
    if (first.width != 16 || first.height != 8 || first.pitch != 64 || !((first.format == 0xd5) || (first.format == 0xcf)))
        throw std::runtime_error("Clear replay surface ABI mismatch");
    const VkFormat format = first.format == 0xcf ? VK_FORMAT_B8G8R8A8_UNORM : VK_FORMAT_R8G8B8A8_UNORM;
    const auto color = context.image(format, VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
                                     VK_IMAGE_ASPECT_COLOR_BIT, first.width, first.height);
    const auto readback = context.buffer(512, VK_BUFFER_USAGE_TRANSFER_DST_BIT);
    VkAttachmentDescription attachment{};
    attachment.format = format; attachment.samples = VK_SAMPLE_COUNT_1_BIT;
    attachment.loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR; attachment.storeOp = VK_ATTACHMENT_STORE_OP_STORE;
    attachment.stencilLoadOp = VK_ATTACHMENT_LOAD_OP_DONT_CARE; attachment.stencilStoreOp = VK_ATTACHMENT_STORE_OP_DONT_CARE;
    attachment.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED; attachment.finalLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    const VkAttachmentReference ref{0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
    VkSubpassDescription subpass{}; subpass.pipelineBindPoint = VK_PIPELINE_BIND_POINT_GRAPHICS;
    subpass.colorAttachmentCount = 1; subpass.pColorAttachments = &ref;
    std::array<VkSubpassDependency,2> dependencies{};
    dependencies[0].srcSubpass = VK_SUBPASS_EXTERNAL; dependencies[0].dstSubpass = 0;
    dependencies[0].srcStageMask = VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT;
    dependencies[0].dstStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
    dependencies[0].dstAccessMask = VK_ACCESS_COLOR_ATTACHMENT_READ_BIT | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
    dependencies[1].srcSubpass = 0; dependencies[1].dstSubpass = VK_SUBPASS_EXTERNAL;
    dependencies[1].srcStageMask = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
    dependencies[1].dstStageMask = VK_PIPELINE_STAGE_TRANSFER_BIT;
    dependencies[1].srcAccessMask = VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT; dependencies[1].dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT;
    auto render_info = structure<VkRenderPassCreateInfo>(VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO);
    render_info.attachmentCount = 1; render_info.pAttachments = &attachment;
    render_info.subpassCount = 1; render_info.pSubpasses = &subpass;
    render_info.dependencyCount = dependencies.size(); render_info.pDependencies = dependencies.data();
    VkRenderPass pass{}; check(vkCreateRenderPass(context.device, &render_info, nullptr, &pass), "create clear replay pass");
    context.render_passes.push_back(pass);
    auto framebuffer_info = structure<VkFramebufferCreateInfo>(VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO);
    framebuffer_info.renderPass = pass; framebuffer_info.attachmentCount = 1; framebuffer_info.pAttachments = &color.view;
    framebuffer_info.width = first.width; framebuffer_info.height = first.height; framebuffer_info.layers = 1;
    VkFramebuffer framebuffer{};
    check(vkCreateFramebuffer(context.device, &framebuffer_info, nullptr, &framebuffer), "create clear replay framebuffer");
    context.framebuffers.push_back(framebuffer);
    const VkPushConstantRange constants{VK_SHADER_STAGE_FRAGMENT_BIT,0,16};
    auto layout_info = structure<VkPipelineLayoutCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO);
    layout_info.pushConstantRangeCount = 1; layout_info.pPushConstantRanges = &constants;
    VkPipelineLayout layout{}; check(vkCreatePipelineLayout(context.device,&layout_info,nullptr,&layout),"create clear replay layout");
    context.pipeline_layouts.push_back(layout);
    std::array<VkPipelineShaderStageCreateInfo,2> stages{};
    for (auto& stage : stages) stage = structure<VkPipelineShaderStageCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO);
    stages[0].stage = VK_SHADER_STAGE_VERTEX_BIT; stages[0].module = context.shader(shaders::clear_vert); stages[0].pName = "main";
    stages[1].stage = VK_SHADER_STAGE_FRAGMENT_BIT; stages[1].module = context.shader(shaders::clear_frag); stages[1].pName = "main";
    const auto command = context.begin();
    const VkClearValue initial{};
    auto begin = structure<VkRenderPassBeginInfo>(VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO);
    begin.renderPass = pass; begin.framebuffer = framebuffer; begin.renderArea = {{0,0},{first.width,first.height}};
    begin.clearValueCount = 1; begin.pClearValues = &initial;
    vkCmdBeginRenderPass(command,&begin,VK_SUBPASS_CONTENTS_INLINE);
    for (uint32_t i=0; i<replay.operation_count; ++i) {
        const auto& op = replay.operations[i];
        if (op.address_upper != 1 || op.address_lower != 0x800 || op.width != first.width || op.height != first.height
            || op.pitch != first.pitch || op.format != first.format || !op.mask || (op.mask & ~15u)
            || op.rect[0] > op.rect[2] || op.rect[1] > op.rect[3] || op.rect[2] > op.width || op.rect[3] > op.height)
            throw std::runtime_error("Clear replay operation ABI mismatch");
        if (op.rect[0] == op.rect[2] || op.rect[1] == op.rect[3]) continue;
        const auto vertex = structure<VkPipelineVertexInputStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO);
        auto assembly = structure<VkPipelineInputAssemblyStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO);
        assembly.topology = VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
        const VkViewport viewport{0,0,static_cast<float>(op.width),static_cast<float>(op.height),0,1};
        const VkRect2D scissor{{static_cast<int32_t>(op.rect[0]),static_cast<int32_t>(op.rect[1])},
                              {op.rect[2]-op.rect[0],op.rect[3]-op.rect[1]}};
        auto viewport_info = structure<VkPipelineViewportStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO);
        viewport_info.viewportCount=1; viewport_info.pViewports=&viewport; viewport_info.scissorCount=1; viewport_info.pScissors=&scissor;
        auto raster = structure<VkPipelineRasterizationStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO);
        raster.polygonMode=VK_POLYGON_MODE_FILL; raster.cullMode=VK_CULL_MODE_NONE; raster.lineWidth=1;
        auto multisample = structure<VkPipelineMultisampleStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO);
        multisample.rasterizationSamples=VK_SAMPLE_COUNT_1_BIT;
        VkPipelineColorBlendAttachmentState blend_attachment{}; blend_attachment.colorWriteMask=op.mask;
        auto blend = structure<VkPipelineColorBlendStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO);
        blend.attachmentCount=1; blend.pAttachments=&blend_attachment;
        auto pipeline_info = structure<VkGraphicsPipelineCreateInfo>(VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO);
        pipeline_info.stageCount=stages.size(); pipeline_info.pStages=stages.data(); pipeline_info.pVertexInputState=&vertex;
        pipeline_info.pInputAssemblyState=&assembly; pipeline_info.pViewportState=&viewport_info;
        pipeline_info.pRasterizationState=&raster; pipeline_info.pMultisampleState=&multisample;
        pipeline_info.pColorBlendState=&blend; pipeline_info.layout=layout; pipeline_info.renderPass=pass;
        VkPipeline pipeline{};
        const auto result=vkCreateGraphicsPipelines(context.device,VK_NULL_HANDLE,1,&pipeline_info,nullptr,&pipeline);
        if (pipeline) context.pipelines.push_back(pipeline);
        check(result,"create masked clear replay pipeline");
        vkCmdBindPipeline(command,VK_PIPELINE_BIND_POINT_GRAPHICS,pipeline);
        vkCmdPushConstants(command,layout,VK_SHADER_STAGE_FRAGMENT_BIT,0,16,op.color);
        vkCmdDraw(command,3,1,0,0);
    }
    vkCmdEndRenderPass(command);
    VkBufferImageCopy copy{}; copy.imageSubresource={VK_IMAGE_ASPECT_COLOR_BIT,0,0,1};
    copy.imageExtent={first.width,first.height,1};
    vkCmdCopyImageToBuffer(command,color.handle,VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,readback.handle,1,&copy);
    buffer_barrier(command,readback,VK_PIPELINE_STAGE_TRANSFER_BIT,VK_ACCESS_TRANSFER_WRITE_BIT,VK_PIPELINE_STAGE_HOST_BIT,VK_ACCESS_HOST_READ_BIT);
    context.submit(command);
    const auto actual=context.read(readback);
    return actual.size() == replay.image_bytes && std::equal(actual.begin(),actual.end(),replay.pixels);
}

// Owned raw SM53 code -> Rust decoder/typed IR/emitter -> Vulkan pipelines.
// Host draw setup is explicit; guest draw methods/header metadata are not parsed.
bool translated_shader_draw(Context& context,uint32_t index) {
    const auto assets=emulator_shader_draw_v1(index);
    const auto alternate=emulator_shader_draw_v1(2);
    for (const auto* a : {&assets,&alternate}) {
        if (a->abi_version!=1 || a->status || !a->vertex_words || a->vertex_words>2048 || !a->fragment_words || a->fragment_words>2048
            || a->vertex[0]!=0x07230203 || a->fragment[0]!=0x07230203)
            throw std::runtime_error(std::string("Owned shader ABI failed: ")+std::string(a->error,strnlen(a->error,sizeof(a->error))));
    }
    const bool bgra=index==1;
    const VkFormat format=bgra ? VK_FORMAT_B8G8R8A8_UNORM : VK_FORMAT_R8G8B8A8_UNORM;
    const auto color=context.image(format,VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT|VK_IMAGE_USAGE_TRANSFER_SRC_BIT,VK_IMAGE_ASPECT_COLOR_BIT,16,8);
    const auto readback=context.buffer(512,VK_BUFFER_USAGE_TRANSFER_DST_BIT);
    const auto vertices=context.buffer(sizeof(assets.vertices),VK_BUFFER_USAGE_VERTEX_BUFFER_BIT);
    context.write(vertices,assets.vertices,sizeof(assets.vertices));
    VkAttachmentDescription attachment{};
    attachment.format=format;attachment.samples=VK_SAMPLE_COUNT_1_BIT;attachment.loadOp=VK_ATTACHMENT_LOAD_OP_CLEAR;
    attachment.storeOp=VK_ATTACHMENT_STORE_OP_STORE;attachment.stencilLoadOp=VK_ATTACHMENT_LOAD_OP_DONT_CARE;
    attachment.stencilStoreOp=VK_ATTACHMENT_STORE_OP_DONT_CARE;attachment.finalLayout=VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    const VkAttachmentReference ref{0,VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
    VkSubpassDescription subpass{};subpass.pipelineBindPoint=VK_PIPELINE_BIND_POINT_GRAPHICS;
    subpass.colorAttachmentCount=1;subpass.pColorAttachments=&ref;
    std::array<VkSubpassDependency,2> dependencies{};
    dependencies[0].srcSubpass=VK_SUBPASS_EXTERNAL;dependencies[0].dstSubpass=0;
    dependencies[0].srcStageMask=VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT;dependencies[0].dstStageMask=VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
    dependencies[0].dstAccessMask=VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
    dependencies[1].srcSubpass=0;dependencies[1].dstSubpass=VK_SUBPASS_EXTERNAL;
    dependencies[1].srcStageMask=VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;dependencies[1].dstStageMask=VK_PIPELINE_STAGE_TRANSFER_BIT;
    dependencies[1].srcAccessMask=VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;dependencies[1].dstAccessMask=VK_ACCESS_TRANSFER_READ_BIT;
    auto pass_info=structure<VkRenderPassCreateInfo>(VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO);
    pass_info.attachmentCount=1;pass_info.pAttachments=&attachment;pass_info.subpassCount=1;pass_info.pSubpasses=&subpass;
    pass_info.dependencyCount=dependencies.size();pass_info.pDependencies=dependencies.data();
    VkRenderPass pass{};check(vkCreateRenderPass(context.device,&pass_info,nullptr,&pass),"create translated shader pass");context.render_passes.push_back(pass);
    auto framebuffer_info=structure<VkFramebufferCreateInfo>(VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO);
    framebuffer_info.renderPass=pass;framebuffer_info.attachmentCount=1;framebuffer_info.pAttachments=&color.view;
    framebuffer_info.width=16;framebuffer_info.height=8;framebuffer_info.layers=1;
    VkFramebuffer framebuffer{};check(vkCreateFramebuffer(context.device,&framebuffer_info,nullptr,&framebuffer),"create translated shader framebuffer");context.framebuffers.push_back(framebuffer);
    auto layout_info=structure<VkPipelineLayoutCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO);
    VkPipelineLayout layout{};check(vkCreatePipelineLayout(context.device,&layout_info,nullptr,&layout),"create translated shader layout");context.pipeline_layouts.push_back(layout);
    const auto vertex_module=context.shader(assets.vertex,assets.vertex_words);
    const auto green=emulator_shader_draw_v1(0);
    auto pipeline=[&](const ShaderDraw& source) {
        if (source.status || source.abi_version!=1 || !source.fragment_words || source.fragment_words>2048)
            throw std::runtime_error("Invalid translated fragment shader");
        std::array<VkPipelineShaderStageCreateInfo,2> stages{};
        for(auto& stage:stages)stage=structure<VkPipelineShaderStageCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO);
        stages[0].stage=VK_SHADER_STAGE_VERTEX_BIT;stages[0].module=vertex_module;stages[0].pName="main";
        stages[1].stage=VK_SHADER_STAGE_FRAGMENT_BIT;stages[1].module=context.shader(source.fragment,source.fragment_words);stages[1].pName="main";
        const VkVertexInputBindingDescription binding{0,4*sizeof(float),VK_VERTEX_INPUT_RATE_VERTEX};
        const VkVertexInputAttributeDescription attribute{0,0,VK_FORMAT_R32G32B32A32_SFLOAT,0};
        auto vertex=structure<VkPipelineVertexInputStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO);
        vertex.vertexBindingDescriptionCount=1;vertex.pVertexBindingDescriptions=&binding;vertex.vertexAttributeDescriptionCount=1;vertex.pVertexAttributeDescriptions=&attribute;
        auto assembly=structure<VkPipelineInputAssemblyStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO);assembly.topology=VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
        const VkViewport viewport{0,0,16,8,0,1};const VkRect2D scissor{{0,0},{16,8}};
        auto viewport_info=structure<VkPipelineViewportStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO);
        viewport_info.viewportCount=1;viewport_info.pViewports=&viewport;viewport_info.scissorCount=1;viewport_info.pScissors=&scissor;
        auto raster=structure<VkPipelineRasterizationStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO);
        raster.polygonMode=VK_POLYGON_MODE_FILL;raster.cullMode=VK_CULL_MODE_NONE;raster.lineWidth=1;
        auto multisample=structure<VkPipelineMultisampleStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO);multisample.rasterizationSamples=VK_SAMPLE_COUNT_1_BIT;
        VkPipelineColorBlendAttachmentState blend_attachment{};blend_attachment.colorWriteMask=15;
        auto blend=structure<VkPipelineColorBlendStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO);blend.attachmentCount=1;blend.pAttachments=&blend_attachment;
        const VkDynamicState scissor_dynamic=VK_DYNAMIC_STATE_SCISSOR;
        auto dynamic=structure<VkPipelineDynamicStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO);dynamic.dynamicStateCount=1;dynamic.pDynamicStates=&scissor_dynamic;
        auto info=structure<VkGraphicsPipelineCreateInfo>(VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO);
        info.stageCount=stages.size();info.pStages=stages.data();info.pVertexInputState=&vertex;info.pInputAssemblyState=&assembly;
        info.pViewportState=&viewport_info;info.pRasterizationState=&raster;info.pMultisampleState=&multisample;
        info.pColorBlendState=&blend;info.pDynamicState=&dynamic;info.layout=layout;info.renderPass=pass;
        VkPipeline handle{};const auto result=vkCreateGraphicsPipelines(context.device,VK_NULL_HANDLE,1,&info,nullptr,&handle);
        if(handle)context.pipelines.push_back(handle);check(result,"create translated shader pipeline");return handle;
    };
    const auto first_pipeline=pipeline(index==2 ? green : assets);
    const auto second_pipeline=index==2 ? pipeline(alternate) : VK_NULL_HANDLE;
    const auto command=context.begin();
    buffer_barrier(command,vertices,VK_PIPELINE_STAGE_HOST_BIT,VK_ACCESS_HOST_WRITE_BIT,VK_PIPELINE_STAGE_VERTEX_INPUT_BIT,VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT);
    VkClearValue initial{};initial.color.float32[3]=1;
    auto begin=structure<VkRenderPassBeginInfo>(VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO);
    begin.renderPass=pass;begin.framebuffer=framebuffer;begin.renderArea={{0,0},{16,8}};begin.clearValueCount=1;begin.pClearValues=&initial;
    vkCmdBeginRenderPass(command,&begin,VK_SUBPASS_CONTENTS_INLINE);
    const VkDeviceSize offset=0;vkCmdBindVertexBuffers(command,0,1,&vertices.handle,&offset);
    const VkRect2D whole{{0,0},{16,8}};vkCmdSetScissor(command,0,1,&whole);
    vkCmdBindPipeline(command,VK_PIPELINE_BIND_POINT_GRAPHICS,first_pipeline);vkCmdDraw(command,6,1,0,0);
    if(index==2) {
        const VkRect2D left{{0,0},{8,8}};vkCmdSetScissor(command,0,1,&left);
        vkCmdBindPipeline(command,VK_PIPELINE_BIND_POINT_GRAPHICS,second_pipeline);vkCmdDraw(command,6,1,0,0);
    }
    vkCmdEndRenderPass(command);
    VkBufferImageCopy copy{};copy.imageSubresource={VK_IMAGE_ASPECT_COLOR_BIT,0,0,1};copy.imageExtent={16,8,1};
    vkCmdCopyImageToBuffer(command,color.handle,VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,readback.handle,1,&copy);
    buffer_barrier(command,readback,VK_PIPELINE_STAGE_TRANSFER_BIT,VK_ACCESS_TRANSFER_WRITE_BIT,VK_PIPELINE_STAGE_HOST_BIT,VK_ACCESS_HOST_READ_BIT);
    context.submit(command);const auto actual=context.read(readback);
    if(actual.size()!=512)return false;
    for(size_t i=0;i<actual.size();++i) {
        const auto x=(i/4)%16;
        const auto component=i%4;
        auto expected=assets.pixels[i];
        if(index==2 && x>=8)expected=green.pixels[i];
        if(bgra && (component==0 || component==2))expected=assets.pixels[(i/4)*4+2-component];
        if(actual[i]!=expected)return false;
    }
    return true;
}

const GuestDrawOp& draw_source(const GuestDrawOp& draw) { return draw; }
const GuestDrawOp& draw_source(const TexturedDraw& draw) { return draw.draw; }
struct TextureBinding { Image image; Buffer upload; VkDescriptorSet set; };
VkDescriptorSetLayout texture_layout(Context& c) {
    VkDescriptorSetLayoutBinding binding{0,VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,1,VK_SHADER_STAGE_FRAGMENT_BIT,nullptr};
    auto info=structure<VkDescriptorSetLayoutCreateInfo>(VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO);
    info.bindingCount=1;info.pBindings=&binding;VkDescriptorSetLayout layout{};
    check(vkCreateDescriptorSetLayout(c.device,&info,nullptr,&layout),"create guest texture layout");c.descriptor_layouts.push_back(layout);return layout;
}
TextureBinding texture_binding(Context& c,const TexturedDraw& d,VkDescriptorSetLayout layout) {
    if(!d.texture_width || !d.texture_height || d.texture_width>64 || d.texture_height>64 || d.texture_bytes!=d.texture_width*d.texture_height*4 || d.texture_bytes>sizeof(d.texture))throw std::runtime_error("Guest texture ABI bounds");
    const auto image=c.image(VK_FORMAT_R8G8B8A8_UNORM,VK_IMAGE_USAGE_SAMPLED_BIT|VK_IMAGE_USAGE_TRANSFER_DST_BIT,VK_IMAGE_ASPECT_COLOR_BIT,d.texture_width,d.texture_height);
    const auto upload=c.buffer(d.texture_bytes,VK_BUFFER_USAGE_TRANSFER_SRC_BIT);c.write(upload,d.texture,d.texture_bytes);
    auto sampler_info=structure<VkSamplerCreateInfo>(VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO);
    sampler_info.magFilter=sampler_info.minFilter=VK_FILTER_NEAREST;sampler_info.mipmapMode=VK_SAMPLER_MIPMAP_MODE_NEAREST;
    sampler_info.addressModeU=sampler_info.addressModeV=sampler_info.addressModeW=VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE;
    VkSampler sampler{};check(vkCreateSampler(c.device,&sampler_info,nullptr,&sampler),"create guest sampler");c.samplers.push_back(sampler);
    VkDescriptorPoolSize size{VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER,1};
    auto pool_info=structure<VkDescriptorPoolCreateInfo>(VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO);pool_info.maxSets=1;pool_info.poolSizeCount=1;pool_info.pPoolSizes=&size;
    VkDescriptorPool pool{};check(vkCreateDescriptorPool(c.device,&pool_info,nullptr,&pool),"create guest sampler pool");c.descriptor_pools.push_back(pool);
    auto allocate=structure<VkDescriptorSetAllocateInfo>(VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO);allocate.descriptorPool=pool;allocate.descriptorSetCount=1;allocate.pSetLayouts=&layout;
    VkDescriptorSet set{};check(vkAllocateDescriptorSets(c.device,&allocate,&set),"allocate guest sampler set");
    VkDescriptorImageInfo image_info{sampler,image.view,VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL};
    auto write=structure<VkWriteDescriptorSet>(VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET);write.dstSet=set;write.descriptorCount=1;write.descriptorType=VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER;write.pImageInfo=&image_info;
    vkUpdateDescriptorSets(c.device,1,&write,0,nullptr);return {image,upload,set};
}
template<class Replay> bool render_guest_commands(Context& context,const Replay& replay,Image* rendered = nullptr) {
    constexpr bool textured=std::is_same_v<Replay,TextureReplay>;
    const auto w=replay.background.width, h=replay.background.height;
    const auto bytes=static_cast<size_t>(w)*h*4;
    if(replay.abi_version!=1 || replay.status || replay.draw_count!=2 || replay.image_bytes!=bytes)
        throw std::runtime_error("Guest draw ABI failure");
    const auto& background=replay.background;
    if(!w || !h || w>64 || h>64 || background.pitch!=w*4 || (background.format!=0xd5 && background.format!=0xcf)
        || background.rect[0]!=0 || background.rect[1]!=0 || background.rect[2]!=w || background.rect[3]!=h || background.mask!=15)
        throw std::runtime_error("Guest draw background ABI mismatch");
    const VkFormat format=background.format==0xcf ? VK_FORMAT_B8G8R8A8_UNORM : VK_FORMAT_R8G8B8A8_UNORM;
    const auto color=context.image(format,VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT|VK_IMAGE_USAGE_TRANSFER_SRC_BIT,VK_IMAGE_ASPECT_COLOR_BIT,w,h);
    const auto readback=context.buffer(bytes,VK_BUFFER_USAGE_TRANSFER_DST_BIT);
    VkAttachmentDescription attachment{};
    attachment.format=format;attachment.samples=VK_SAMPLE_COUNT_1_BIT;attachment.loadOp=VK_ATTACHMENT_LOAD_OP_CLEAR;
    attachment.storeOp=VK_ATTACHMENT_STORE_OP_STORE;attachment.stencilLoadOp=VK_ATTACHMENT_LOAD_OP_DONT_CARE;
    attachment.stencilStoreOp=VK_ATTACHMENT_STORE_OP_DONT_CARE;attachment.finalLayout=VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    const VkAttachmentReference ref{0,VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL};
    VkSubpassDescription subpass{};subpass.pipelineBindPoint=VK_PIPELINE_BIND_POINT_GRAPHICS;
    subpass.colorAttachmentCount=1;subpass.pColorAttachments=&ref;
    std::array<VkSubpassDependency,2> dependencies{};
    dependencies[0].srcSubpass=VK_SUBPASS_EXTERNAL;dependencies[0].dstSubpass=0;
    dependencies[0].srcStageMask=VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT;dependencies[0].dstStageMask=VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
    dependencies[0].dstAccessMask=VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;
    dependencies[1].srcSubpass=0;dependencies[1].dstSubpass=VK_SUBPASS_EXTERNAL;
    dependencies[1].srcStageMask=VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;dependencies[1].dstStageMask=VK_PIPELINE_STAGE_TRANSFER_BIT;
    dependencies[1].srcAccessMask=VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT;dependencies[1].dstAccessMask=VK_ACCESS_TRANSFER_READ_BIT;
    auto pass_info=structure<VkRenderPassCreateInfo>(VK_STRUCTURE_TYPE_RENDER_PASS_CREATE_INFO);
    pass_info.attachmentCount=1;pass_info.pAttachments=&attachment;pass_info.subpassCount=1;pass_info.pSubpasses=&subpass;
    pass_info.dependencyCount=dependencies.size();pass_info.pDependencies=dependencies.data();
    VkRenderPass pass{};check(vkCreateRenderPass(context.device,&pass_info,nullptr,&pass),"create translated shader pass");context.render_passes.push_back(pass);
    auto framebuffer_info=structure<VkFramebufferCreateInfo>(VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO);
    framebuffer_info.renderPass=pass;framebuffer_info.attachmentCount=1;framebuffer_info.pAttachments=&color.view;
    framebuffer_info.width=w;framebuffer_info.height=h;framebuffer_info.layers=1;
    VkFramebuffer framebuffer{};check(vkCreateFramebuffer(context.device,&framebuffer_info,nullptr,&framebuffer),"create translated shader framebuffer");context.framebuffers.push_back(framebuffer);
    auto layout_info=structure<VkPipelineLayoutCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO);
    VkDescriptorSetLayout sampler_layout{};
    if constexpr(textured){sampler_layout=texture_layout(context);layout_info.setLayoutCount=1;layout_info.pSetLayouts=&sampler_layout;}
    VkPipelineLayout layout{};check(vkCreatePipelineLayout(context.device,&layout_info,nullptr,&layout),"create translated shader layout");context.pipeline_layouts.push_back(layout);
    auto pipeline=[&](const GuestDrawOp& source) {
        if (!source.vertex_words || source.vertex_words>2048 || !source.fragment_words || source.fragment_words>2048 || source.vertex[0]!=0x07230203 || source.fragment[0]!=0x07230203)
            throw std::runtime_error("Invalid translated fragment shader");
        std::array<VkPipelineShaderStageCreateInfo,2> stages{};
        for(auto& stage:stages)stage=structure<VkPipelineShaderStageCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO);
        stages[0].stage=VK_SHADER_STAGE_VERTEX_BIT;stages[0].module=context.shader(source.vertex,source.vertex_words);stages[0].pName="main";
        stages[1].stage=VK_SHADER_STAGE_FRAGMENT_BIT;stages[1].module=context.shader(source.fragment,source.fragment_words);stages[1].pName="main";
        const VkVertexInputBindingDescription binding{0,(textured?8u:4u)*sizeof(float),VK_VERTEX_INPUT_RATE_VERTEX};
        const std::array<VkVertexInputAttributeDescription,2> attributes{{{0,0,VK_FORMAT_R32G32B32A32_SFLOAT,0},{1,0,VK_FORMAT_R32G32B32A32_SFLOAT,16}}};
        auto vertex=structure<VkPipelineVertexInputStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO);
        vertex.vertexBindingDescriptionCount=1;vertex.pVertexBindingDescriptions=&binding;vertex.vertexAttributeDescriptionCount=textured?2:1;vertex.pVertexAttributeDescriptions=attributes.data();
        auto assembly=structure<VkPipelineInputAssemblyStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO);assembly.topology=VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST;
        const VkViewport viewport{0,0,static_cast<float>(w),static_cast<float>(h),0,1};const VkRect2D scissor{{0,0},{w,h}};
        auto viewport_info=structure<VkPipelineViewportStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_VIEWPORT_STATE_CREATE_INFO);
        viewport_info.viewportCount=1;viewport_info.pViewports=&viewport;viewport_info.scissorCount=1;viewport_info.pScissors=&scissor;
        auto raster=structure<VkPipelineRasterizationStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_RASTERIZATION_STATE_CREATE_INFO);
        raster.polygonMode=VK_POLYGON_MODE_FILL;raster.cullMode=VK_CULL_MODE_NONE;raster.lineWidth=1;
        auto multisample=structure<VkPipelineMultisampleStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO);multisample.rasterizationSamples=VK_SAMPLE_COUNT_1_BIT;
        VkPipelineColorBlendAttachmentState blend_attachment{};blend_attachment.colorWriteMask=15;
        auto blend=structure<VkPipelineColorBlendStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO);blend.attachmentCount=1;blend.pAttachments=&blend_attachment;
        const VkDynamicState scissor_dynamic=VK_DYNAMIC_STATE_SCISSOR;
        auto dynamic=structure<VkPipelineDynamicStateCreateInfo>(VK_STRUCTURE_TYPE_PIPELINE_DYNAMIC_STATE_CREATE_INFO);dynamic.dynamicStateCount=1;dynamic.pDynamicStates=&scissor_dynamic;
        auto info=structure<VkGraphicsPipelineCreateInfo>(VK_STRUCTURE_TYPE_GRAPHICS_PIPELINE_CREATE_INFO);
        info.stageCount=stages.size();info.pStages=stages.data();info.pVertexInputState=&vertex;info.pInputAssemblyState=&assembly;
        info.pViewportState=&viewport_info;info.pRasterizationState=&raster;info.pMultisampleState=&multisample;
        info.pColorBlendState=&blend;info.pDynamicState=&dynamic;info.layout=layout;info.renderPass=pass;
        VkPipeline handle{};const auto result=vkCreateGraphicsPipelines(context.device,VK_NULL_HANDLE,1,&info,nullptr,&handle);
        if(handle)context.pipelines.push_back(handle);check(result,"create translated shader pipeline");return handle;
    };
    std::array<Buffer,3> vertex_buffers{};
    std::array<TextureBinding,3> textures{};
    std::array<VkPipeline,3> draw_pipelines{};
    for(uint32_t i=0;i<replay.draw_count;++i) {
        const auto& d=draw_source(replay.draws[i]);
        if(d.address_upper!=background.address_upper || d.address_lower!=background.address_lower || d.width!=w || d.height!=h
            || d.pitch!=w*4 || d.format!=background.format || d.mask!=15 || d.vertex_count!=6
            || d.rect[0]>d.rect[2] || d.rect[1]>d.rect[3] || d.rect[2]>w || d.rect[3]>h)
            throw std::runtime_error("Guest draw operation ABI mismatch");
        if constexpr(textured) {
            const auto& source=replay.draws[i];std::array<uint32_t,48> inputs{};
            for(size_t v=0;v<6;++v) {std::copy_n(d.vertices+v*4,4,inputs.data()+v*8);std::copy_n(source.varyings+v*4,4,inputs.data()+v*8+4);}
            vertex_buffers[i]=context.buffer(sizeof(inputs),VK_BUFFER_USAGE_VERTEX_BUFFER_BIT);context.write(vertex_buffers[i],inputs.data(),sizeof(inputs));
            textures[i]=texture_binding(context,source,sampler_layout);
        } else {
            vertex_buffers[i]=context.buffer(sizeof(d.vertices),VK_BUFFER_USAGE_VERTEX_BUFFER_BIT);
            context.write(vertex_buffers[i],d.vertices,sizeof(d.vertices));
        }
        draw_pipelines[i]=pipeline(d);
    }
    const auto command=context.begin();
    for(uint32_t i=0;i<replay.draw_count;++i)
        buffer_barrier(command,vertex_buffers[i],VK_PIPELINE_STAGE_HOST_BIT,VK_ACCESS_HOST_WRITE_BIT,VK_PIPELINE_STAGE_VERTEX_INPUT_BIT,VK_ACCESS_VERTEX_ATTRIBUTE_READ_BIT);
    if constexpr(textured) for(uint32_t i=0;i<replay.draw_count;++i){
        const auto& t=textures[i];const auto& d=replay.draws[i];
        buffer_barrier(command,t.upload,VK_PIPELINE_STAGE_HOST_BIT,VK_ACCESS_HOST_WRITE_BIT,VK_PIPELINE_STAGE_TRANSFER_BIT,VK_ACCESS_TRANSFER_READ_BIT);
        image_barrier(command,t.image.handle,VK_IMAGE_LAYOUT_UNDEFINED,VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,0,VK_PIPELINE_STAGE_TRANSFER_BIT,VK_ACCESS_TRANSFER_WRITE_BIT);
        VkBufferImageCopy copy{};copy.imageSubresource={VK_IMAGE_ASPECT_COLOR_BIT,0,0,1};copy.imageExtent={d.texture_width,d.texture_height,1};
        vkCmdCopyBufferToImage(command,t.upload.handle,t.image.handle,VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,1,&copy);
        image_barrier(command,t.image.handle,VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,VK_PIPELINE_STAGE_TRANSFER_BIT,VK_ACCESS_TRANSFER_WRITE_BIT,VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,VK_ACCESS_SHADER_READ_BIT);
    }
    VkClearValue initial{};std::memcpy(initial.color.float32,background.color,sizeof(background.color));
    auto begin=structure<VkRenderPassBeginInfo>(VK_STRUCTURE_TYPE_RENDER_PASS_BEGIN_INFO);
    begin.renderPass=pass;begin.framebuffer=framebuffer;begin.renderArea={{0,0},{w,h}};begin.clearValueCount=1;begin.pClearValues=&initial;
    vkCmdBeginRenderPass(command,&begin,VK_SUBPASS_CONTENTS_INLINE);
    for(uint32_t i=0;i<replay.draw_count;++i) {
        const auto& d=draw_source(replay.draws[i]);
        if(d.rect[0]==d.rect[2] || d.rect[1]==d.rect[3])continue;
        const VkDeviceSize offset=0;vkCmdBindVertexBuffers(command,0,1,&vertex_buffers[i].handle,&offset);
        const VkRect2D scissor{{static_cast<int32_t>(d.rect[0]),static_cast<int32_t>(d.rect[1])},{d.rect[2]-d.rect[0],d.rect[3]-d.rect[1]}};
        vkCmdSetScissor(command,0,1,&scissor);vkCmdBindPipeline(command,VK_PIPELINE_BIND_POINT_GRAPHICS,draw_pipelines[i]);
        if constexpr(textured)vkCmdBindDescriptorSets(command,VK_PIPELINE_BIND_POINT_GRAPHICS,layout,0,1,&textures[i].set,0,nullptr);
        vkCmdDraw(command,d.vertex_count,1,0,0);
    }
    vkCmdEndRenderPass(command);
    VkBufferImageCopy copy{};copy.imageSubresource={VK_IMAGE_ASPECT_COLOR_BIT,0,0,1};copy.imageExtent={w,h,1};
    vkCmdCopyImageToBuffer(command,color.handle,VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,readback.handle,1,&copy);
    buffer_barrier(command,readback,VK_PIPELINE_STAGE_TRANSFER_BIT,VK_ACCESS_TRANSFER_WRITE_BIT,VK_PIPELINE_STAGE_HOST_BIT,VK_ACCESS_HOST_READ_BIT);
    context.submit(command);const auto actual=context.read(readback);
    if(rendered)*rendered=color;
    return actual.size()==bytes && std::equal(actual.begin(),actual.end(),replay.pixels);
}

bool guest_draw_command_replay(Context& context,uint32_t index) {
    return render_guest_commands(context,emulator_guest_draw_replay_v1(index));
}

// This backend stays on the single Java render worker. Every frame is bounded;
// no guest or Rust pointers escape into Vulkan, and callbacks cannot reenter.
class LiveRenderer {
public:
    Context context;
    VkSwapchainKHR swapchain{};
    VkExtent2D extent{};
    VkFormat swap_format{};
    std::vector<VkImage> swap_images;
    VkSemaphore acquired{};
    std::vector<VkSemaphore> ready_images;
    uint64_t completed=0, presented=0;
    void* native_window=nullptr;
    std::string error;
    ~LiveRenderer() {
        if(context.device) {
            vkDeviceWaitIdle(context.device);
            if(acquired)vkDestroySemaphore(context.device,acquired,nullptr);
            for(auto ready:ready_images)vkDestroySemaphore(context.device,ready,nullptr);
            if(swapchain)vkDestroySwapchainKHR(context.device,swapchain,nullptr);
        }
        context.shutdown();
#ifdef __ANDROID__
        if(native_window)ANativeWindow_release(static_cast<ANativeWindow*>(native_window));
#endif
    }
    void initialize(void* window) {
        native_window=window;
#ifdef __ANDROID__
        if(window)ANativeWindow_acquire(static_cast<ANativeWindow*>(window));
#endif
        context.initialize(window);
        if(context.surface) create_swapchain();
    }
    void create_swapchain() {
        VkSurfaceCapabilitiesKHR caps{};
        check(vkGetPhysicalDeviceSurfaceCapabilitiesKHR(context.physical,context.surface,&caps),"surface capabilities");
        if(!(caps.supportedUsageFlags & VK_IMAGE_USAGE_TRANSFER_DST_BIT))throw std::runtime_error("Surface cannot receive transfer images");
        uint32_t n=0;check(vkGetPhysicalDeviceSurfaceFormatsKHR(context.physical,context.surface,&n,nullptr),"surface format count");
        std::vector<VkSurfaceFormatKHR> formats(n);
        check(vkGetPhysicalDeviceSurfaceFormatsKHR(context.physical,context.surface,&n,formats.data()),"surface formats");
        const auto chosen=std::find_if(formats.begin(),formats.end(),[](auto f){return (f.format==VK_FORMAT_R8G8B8A8_UNORM || f.format==VK_FORMAT_B8G8R8A8_UNORM) && f.colorSpace==VK_COLOR_SPACE_SRGB_NONLINEAR_KHR;});
        if(chosen==formats.end())throw std::runtime_error("No supported UNORM Android surface format");
        swap_format=chosen->format;
        VkFormatProperties src{},dst{};
        vkGetPhysicalDeviceFormatProperties(context.physical,VK_FORMAT_R8G8B8A8_UNORM,&src);
        vkGetPhysicalDeviceFormatProperties(context.physical,swap_format,&dst);
        if(!(src.optimalTilingFeatures & VK_FORMAT_FEATURE_BLIT_SRC_BIT) || !(dst.optimalTilingFeatures & VK_FORMAT_FEATURE_BLIT_DST_BIT))
            throw std::runtime_error("Surface blit unsupported");
        extent=caps.currentExtent;
        if(extent.width==UINT32_MAX || !extent.width || !extent.height)throw std::runtime_error("Surface extent unavailable");
        auto info=structure<VkSwapchainCreateInfoKHR>(VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR);
        info.surface=context.surface;info.minImageCount=std::max(2u,caps.minImageCount);
        if(caps.maxImageCount)info.minImageCount=std::min(info.minImageCount,caps.maxImageCount);
        info.imageFormat=chosen->format;info.imageColorSpace=chosen->colorSpace;info.imageExtent=extent;
        info.imageArrayLayers=1;info.imageUsage=VK_IMAGE_USAGE_TRANSFER_DST_BIT;info.imageSharingMode=VK_SHARING_MODE_EXCLUSIVE;
        info.preTransform=caps.currentTransform;
        for(auto flag:{VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR,VK_COMPOSITE_ALPHA_INHERIT_BIT_KHR,VK_COMPOSITE_ALPHA_PRE_MULTIPLIED_BIT_KHR,VK_COMPOSITE_ALPHA_POST_MULTIPLIED_BIT_KHR}) {
            if(caps.supportedCompositeAlpha & flag){info.compositeAlpha=flag;break;}
        }
        info.presentMode=VK_PRESENT_MODE_FIFO_KHR;info.clipped=VK_TRUE;
        check(vkCreateSwapchainKHR(context.device,&info,nullptr,&swapchain),"create Android swapchain");
        check(vkGetSwapchainImagesKHR(context.device,swapchain,&n,nullptr),"swapchain image count");swap_images.resize(n);
        check(vkGetSwapchainImagesKHR(context.device,swapchain,&n,swap_images.data()),"swapchain images");
        auto sem=structure<VkSemaphoreCreateInfo>(VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO);
        check(vkCreateSemaphore(context.device,&sem,nullptr,&acquired),"acquire semaphore");
        ready_images.resize(swap_images.size());
        for(auto& ready:ready_images)check(vkCreateSemaphore(context.device,&sem,nullptr,&ready),"per-image present semaphore");
    }
    void present(const Image& source,uint32_t width,uint32_t height,VkFormat format) {
        VkFormatProperties properties{};vkGetPhysicalDeviceFormatProperties(context.physical,format,&properties);
        if(!(properties.optimalTilingFeatures & VK_FORMAT_FEATURE_BLIT_SRC_BIT))throw std::runtime_error("Source format cannot be presented by blit");
        uint32_t index=0;
        auto result=vkAcquireNextImageKHR(context.device,swapchain,1'000'000'000ull,acquired,VK_NULL_HANDLE,&index);
        if(result!=VK_SUBOPTIMAL_KHR)check(result,"acquire Android image");
        const auto ready=ready_images.at(index);
        const auto command=context.begin();
        auto barrier=structure<VkImageMemoryBarrier>(VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER);
        barrier.oldLayout=VK_IMAGE_LAYOUT_UNDEFINED;barrier.newLayout=VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
        barrier.srcQueueFamilyIndex=VK_QUEUE_FAMILY_IGNORED;barrier.dstQueueFamilyIndex=VK_QUEUE_FAMILY_IGNORED;
        barrier.dstAccessMask=VK_ACCESS_TRANSFER_WRITE_BIT;barrier.image=swap_images[index];
        barrier.subresourceRange={VK_IMAGE_ASPECT_COLOR_BIT,0,1,0,1};
        vkCmdPipelineBarrier(command,VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,VK_PIPELINE_STAGE_TRANSFER_BIT,0,0,nullptr,0,nullptr,1,&barrier);
        VkClearColorValue black{};black.float32[3]=1;
        vkCmdClearColorImage(command,swap_images[index],VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,&black,1,&barrier.subresourceRange);
        // Serialize the letterbox clear and the overlapping destination blit.
        barrier.oldLayout=VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;barrier.srcAccessMask=VK_ACCESS_TRANSFER_WRITE_BIT;
        vkCmdPipelineBarrier(command,VK_PIPELINE_STAGE_TRANSFER_BIT,VK_PIPELINE_STAGE_TRANSFER_BIT,0,0,nullptr,0,nullptr,1,&barrier);
        uint32_t dw=extent.width,dh=std::max(1u,static_cast<uint32_t>(static_cast<uint64_t>(dw)*height/width));
        if(dh>extent.height){dh=extent.height;dw=std::max(1u,static_cast<uint32_t>(static_cast<uint64_t>(dh)*width/height));}
        const auto x=(extent.width-dw)/2,y=(extent.height-dh)/2;
        VkImageBlit blit{};blit.srcSubresource={VK_IMAGE_ASPECT_COLOR_BIT,0,0,1};blit.srcOffsets[1]={static_cast<int32_t>(width),static_cast<int32_t>(height),1};
        blit.dstSubresource={VK_IMAGE_ASPECT_COLOR_BIT,0,0,1};
        blit.dstOffsets[0]={static_cast<int32_t>(x),static_cast<int32_t>(y),0};
        blit.dstOffsets[1]={static_cast<int32_t>(x+dw),static_cast<int32_t>(y+dh),1};
        vkCmdBlitImage(command,source.handle,VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,swap_images[index],VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,1,&blit,VK_FILTER_NEAREST);
        barrier.oldLayout=VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;barrier.newLayout=VK_IMAGE_LAYOUT_PRESENT_SRC_KHR;
        barrier.srcAccessMask=VK_ACCESS_TRANSFER_WRITE_BIT;barrier.dstAccessMask=0;
        vkCmdPipelineBarrier(command,VK_PIPELINE_STAGE_TRANSFER_BIT,VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,0,0,nullptr,0,nullptr,1,&barrier);
        check(vkEndCommandBuffer(command),"end present command");
        auto fence_info=structure<VkFenceCreateInfo>(VK_STRUCTURE_TYPE_FENCE_CREATE_INFO);VkFence fence{};
        check(vkCreateFence(context.device,&fence_info,nullptr,&fence),"present fence");context.fences.push_back(fence);
        const VkPipelineStageFlags wait_stage=VK_PIPELINE_STAGE_TRANSFER_BIT;
        auto submit=structure<VkSubmitInfo>(VK_STRUCTURE_TYPE_SUBMIT_INFO);
        submit.waitSemaphoreCount=1;submit.pWaitSemaphores=&acquired;submit.pWaitDstStageMask=&wait_stage;
        submit.commandBufferCount=1;submit.pCommandBuffers=&command;submit.signalSemaphoreCount=1;submit.pSignalSemaphores=&ready;
        check(vkQueueSubmit(context.queue,1,&submit,fence),"submit present transfer");
        auto present_info=structure<VkPresentInfoKHR>(VK_STRUCTURE_TYPE_PRESENT_INFO_KHR);
        present_info.waitSemaphoreCount=1;present_info.pWaitSemaphores=&ready;present_info.swapchainCount=1;
        present_info.pSwapchains=&swapchain;present_info.pImageIndices=&index;
        result=vkQueuePresentKHR(context.queue,&present_info);
        if(result!=VK_SUBOPTIMAL_KHR)check(result,"present Android image");
        check(vkWaitForFences(context.device,1,&fence,VK_TRUE,5'000'000'000ull),"present transfer completion");
        // Deliberately single-frame reference backend. Drain the presentation wait
        // before reusing binary semaphores, frame resources or the native window.
        check(vkQueueWaitIdle(context.queue),"presentation queue idle");
        ++presented;
    }
    template<class Replay> uint32_t render(const Replay& draw) noexcept {
        try {
            context.clear_workloads();
            check(vkResetCommandPool(context.device,context.command_pool,0),"reset live frame commands");
            Image image{};
            if(!render_guest_commands(context,draw,&image))throw std::runtime_error("Live guest Vulkan pixels differ from serial raster oracle");
            if(context.validation_errors)throw std::runtime_error("Vulkan validation error");
            ++completed;
            if(swapchain)present(image,draw.background.width,draw.background.height,draw.background.format==0xcf?VK_FORMAT_B8G8R8A8_UNORM:VK_FORMAT_R8G8B8A8_UNORM);
            return 0;
        } catch(const std::exception& e) {error=e.what();return 1;}
    }
};
thread_local std::unique_ptr<LiveRenderer> live;
extern "C" uint32_t complete_live_draw(LiveDraw draw) { return live ? live->render(draw) : 2; }
void status_check(const DemoStatus& s) {
    if(s.abi_version!=1 || s.status)throw std::runtime_error(live && !live->error.empty() ? live->error : std::string(s.error,strnlen(s.error,sizeof(s.error))));
}
bool live_homebrew_check() {
    live=std::make_unique<LiveRenderer>();
    try {
        live->initialize(nullptr);status_check(emulator_demo_start_v1(complete_live_draw));
        DemoStatus s{};
        for(uint32_t i=0;i<110;++i) {s=emulator_demo_step_v1(i<54 ? 20 : 0,0);status_check(s);}
        const bool ok=s.frame==110 && s.score==1 && s.misses==1 && live->completed==110;
        s=emulator_demo_step_v1(48,1);status_check(s);
        const bool reset=s.frame==1 && s.score==0 && s.misses==0 && s.paddle_x==48;
        status_check(emulator_demo_stop_v1());
        const bool valid=live->context.validation_errors==0;live.reset();return ok && reset && valid;
    } catch(...) { (void)emulator_demo_stop_v1();live.reset();throw; }
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
        tests.push_back({"bc1-to-rgba8-fallback", bc1_conversion(context), "GPU BC1 RGBA decode must match every byte of the Rust reference; no native BC feature required"});
        const auto sampled = sampled_bc1_updates(context);
        tests.push_back({"bc1-converted-texture-sampling", sampled[0], "Nearest sampled draw must match the CPU texel-coordinate oracle, including edges and transparent pixels"});
        tests.push_back({"bc1-texture-update-resample", sampled[1], "The same image is redecoded and resampled in one submission; updated pixels and encoded source bytes must match"});
        tests.push_back({"guest-rgba-clear-command-replay", guest_clear_replay(context,0), "Actual NV NRO command bytes -> original decoder -> Vulkan clear lowering; all 512 bytes match the independent oracle and guest image"});
        tests.push_back({"owned-bgra-clear-command-replay", guest_clear_replay(context,1), "BGRA target storage and ordered clear rectangles match every reference byte"});
        tests.push_back({"owned-masked-clear-command-replay", guest_clear_replay(context,2), "Masked green writes preserve previous red/blue/alpha in overlapping rectangles"});
        tests.push_back({"sm53-translated-rgba-draw",translated_shader_draw(context,0),"Original SM53 vertex/fragment code -> typed IR -> emitted SPIR-V; every RGBA pixel matches"});
        tests.push_back({"sm53-transformed-bgra-draw",translated_shader_draw(context,1),"Translated FMUL transforms position; geometry coverage, clear border and BGRA bytes match"});
        tests.push_back({"sm53-shader-change-ordered-draws",translated_shader_draw(context,2),"Two translated fragment shaders draw green then scoped red in one submission; all pixels match"});
        tests.push_back({"guest-nv-draw-command-replay",guest_draw_command_replay(context,0),"Interpreted NRO -> NV commands -> SPH/shader/vertex/draw decode -> Vulkan; all512 bytes match guest and independent oracle"});
        tests.push_back({"owned-nv-bgra-draw-command-replay",guest_draw_command_replay(context,1),"Decoded guest BGRA draw state, shader headers, transformed geometry and byte storage match"});
        tests.push_back({"owned-nv-scissor-draw-command-replay",guest_draw_command_replay(context,2),"Decoded guest scissor and vertex transforms preserve surrounding pixels across ordered draws"});
        tests.push_back({"live-homebrew-frames-input-and-completion",live_homebrew_check(),"111 interpreted homebrew frames; live Vulkan completion before guest publication; catches, misses, reset and resource release"});
        for(uint32_t i=0;i<3;++i)tests.push_back({std::array<const char*,3>{"guest-sampled-texture-rgba","guest-sampled-texture-bgra","guest-sampled-texture-updated"}[i],render_guest_commands(context,emulator_texture_replay_v1(i)),"Guest SPH/IPA/TEX/TIC/TSC -> original SPIR-V and Vulkan; every byte matches independent sampling and raster oracles"});
    } catch (const std::exception& failure) { error = failure.what(); }
    // Device queries must finish before the instance and physical-device handle are destroyed.
    std::ostringstream capabilities;
    append_capabilities(capabilities, context);
    context.shutdown();
    const bool passed = error.empty() && context.validation_errors == 0 && tests.size() == 19 && std::all_of(tests.begin(), tests.end(), [](const Test& test) { return test.passed; });
    std::ostringstream json;
    json << "{\"schemaVersion\":1,\"workloadVersion\":9,\"scope\":\"host-vulkan-and-owned-guest-draw-replay\",\"deviceName\":" << quoted(context.properties.deviceName)
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
    json << "],\"diagnosticAllocations\":{\"scope\":\"all-lab-workloads\",\"bytesRequested\":" << context.allocation_bytes
         << ",\"count\":" << context.allocations.size() << ",\"includesDriverOverhead\":false},\"allPassed\":" << (passed ? "true" : "false") << ",\"error\":" << quoted(error) << '}';
    return {passed, json.str()};
}

namespace {
std::string demo_json(const DemoStatus& s,bool running,const std::string& error="") {
    std::ostringstream j;
    j << "{\"scope\":\"original-live-homebrew\",\"running\":" << (running ? "true" : "false")
      << ",\"frame\":" << s.frame << ",\"score\":" << s.score << ",\"misses\":" << s.misses
      << ",\"ballX\":" << s.ball_x << ",\"ballY\":" << s.ball_y << ",\"paddleX\":" << s.paddle_x
      << ",\"completedFrames\":" << (live ? live->completed : 0) << ",\"presentedFrames\":" << (live ? live->presented : 0)
      << ",\"renderWidth\":64,\"renderHeight\":64,\"deviceName\":" << quoted(live ? live->context.properties.deviceName : "")
      << ",\"isSoftwareRenderer\":" << (live && live->context.properties.deviceType==VK_PHYSICAL_DEVICE_TYPE_CPU ? "true" : "false")
      << ",\"validationErrors\":" << (live ? live->context.validation_errors.load() : 0)
      << ",\"frameAllocationCount\":" << (live ? live->context.allocations.size() : 0)
      << ",\"error\":" << quoted(error) << "}";
    return j.str();
}
}
std::string start_homebrew(void* window) {
    try {
        (void)emulator_demo_stop_v1();live.reset();live=std::make_unique<LiveRenderer>();
        live->initialize(window);status_check(emulator_demo_start_v1(complete_live_draw));
        return demo_json(DemoStatus{},true);
    } catch(const std::exception& e) {const auto result=demo_json(DemoStatus{},false,e.what());(void)emulator_demo_stop_v1();live.reset();return result;}
}
std::string step_homebrew(uint32_t x,uint32_t reset) {
    try {if(!live)throw std::runtime_error("No native homebrew session");const auto s=emulator_demo_step_v1(x,reset);status_check(s);return demo_json(s,true);}
    catch(const std::exception& e) {return demo_json(DemoStatus{},false,e.what());}
}
std::string stop_homebrew() {
    const auto s=emulator_demo_stop_v1();const auto result=demo_json(s,false,s.status ? std::string(s.error,strnlen(s.error,sizeof(s.error))) : "");live.reset();return result;
}

namespace {
thread_local std::unique_ptr<LiveRenderer> texture_scene;
thread_local unsigned texture_variant=0;
std::string texture_scene_json(bool running,const std::string& error="") {
    const auto* scene=texture_scene.get();std::ostringstream j;
    j << "{\"scope\":\"original-guest-texture-scene\",\"running\":" << (running?"true":"false")
      << ",\"variant\":" << texture_variant << ",\"renderWidth\":16,\"renderHeight\":8"
      << ",\"completedFrames\":" << (scene?scene->completed:0) << ",\"presentedFrames\":" << (scene?scene->presented:0)
      << ",\"pixelsVerified\":" << (scene && scene->completed ? "true":"false")
      << ",\"deviceName\":" << quoted(scene?scene->context.properties.deviceName:"")
      << ",\"isSoftwareRenderer\":" << (scene && scene->context.properties.deviceType==VK_PHYSICAL_DEVICE_TYPE_CPU?"true":"false")
      << ",\"validationErrors\":" << (scene?scene->context.validation_errors.load():0)
      << ",\"frameAllocationCount\":" << (scene?scene->context.allocations.size():0)
      << ",\"error\":" << quoted(error) << "}";return j.str();
}
}
std::string start_texture_scene(void* window,unsigned variant) {
    try {
        texture_scene.reset();texture_variant=variant;
        if(variant>2)throw std::runtime_error("Unknown texture scene variant");
        texture_scene=std::make_unique<LiveRenderer>();texture_scene->initialize(window);
        const auto replay=emulator_texture_replay_v1(variant);
        if(replay.status)throw std::runtime_error(std::string(replay.error,strnlen(replay.error,sizeof(replay.error))));
        if(texture_scene->render(replay))throw std::runtime_error(texture_scene->error);
        return texture_scene_json(true);
    }catch(const std::exception& e){const auto result=texture_scene_json(false,e.what());texture_scene.reset();return result;}
}
std::string stop_texture_scene() {const auto result=texture_scene_json(false,texture_scene?texture_scene->error:"");texture_scene.reset();return result;}
