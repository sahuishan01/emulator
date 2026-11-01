#pragma once
#include <cstddef>
#include <cstdint>

struct EngineReport {
    std::uint32_t abi_version;
    std::uint32_t length;
    std::uint8_t bytes[8192];
};
static_assert(sizeof(EngineReport) == 8200 && alignof(EngineReport) == 4);
static_assert(offsetof(EngineReport, bytes) == 8);
extern "C" EngineReport emulator_run_reference_guests_v2();
extern "C" EngineReport emulator_run_gpu_core_checks_v2();
extern "C" EngineReport emulator_run_nv_checks_v2();

struct ClearOp {
    std::uint32_t address_upper, address_lower, width, height, pitch, format;
    std::uint32_t rect[4], color[4], mask;
};
struct ClearReplay {
    std::uint32_t abi_version, status, operation_count, image_bytes;
    ClearOp operations[8];
    std::uint8_t pixels[512];
    char error[256];
};
static_assert(sizeof(ClearOp) == 60 && alignof(ClearOp) == 4);
static_assert(sizeof(ClearReplay) == 1264 && alignof(ClearReplay) == 4);
static_assert(offsetof(ClearReplay, operations) == 16);
static_assert(offsetof(ClearReplay, pixels) == 496);
static_assert(offsetof(ClearReplay, error) == 1008);
extern "C" ClearReplay emulator_guest_clear_replay_v1(std::uint32_t index);
