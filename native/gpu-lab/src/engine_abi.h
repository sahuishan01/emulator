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

extern "C" EngineReport emulator_run_shader_checks_v2();
struct ShaderDraw {
    std::uint32_t abi_version, status, vertex_words, fragment_words;
    std::uint32_t vertex[2048], fragment[2048], vertices[24];
    std::uint8_t pixels[512];
    char error[256];
};
static_assert(sizeof(ShaderDraw)==17264 && alignof(ShaderDraw)==4);
static_assert(offsetof(ShaderDraw,vertex)==16 && offsetof(ShaderDraw,fragment)==8208);
static_assert(offsetof(ShaderDraw,vertices)==16400 && offsetof(ShaderDraw,pixels)==16496);
static_assert(offsetof(ShaderDraw,error)==17008);
extern "C" ShaderDraw emulator_shader_draw_v1(std::uint32_t index);

struct GuestDrawOp {
    std::uint32_t address_upper,address_lower,width,height,pitch,format,rect[4],mask,vertex_count,vertex_words,fragment_words;
    std::uint32_t vertex[2048],fragment[2048],vertices[24];
};
struct GuestDrawReplay {
    std::uint32_t abi_version,status,draw_count,image_bytes;
    ClearOp background;
    GuestDrawOp draws[3];
    std::uint8_t pixels[512];
    char error[256];
};
static_assert(sizeof(GuestDrawOp)==16536 && alignof(GuestDrawOp)==4);
static_assert(offsetof(GuestDrawOp,vertex)==56 && offsetof(GuestDrawOp,fragment)==8248 && offsetof(GuestDrawOp,vertices)==16440);
static_assert(sizeof(GuestDrawReplay)==50452 && alignof(GuestDrawReplay)==4);
static_assert(offsetof(GuestDrawReplay,draws)==76 && offsetof(GuestDrawReplay,pixels)==49684 && offsetof(GuestDrawReplay,error)==50196);
extern "C" GuestDrawReplay emulator_guest_draw_replay_v1(std::uint32_t index);

struct LiveDraw {
    std::uint32_t abi_version,status,draw_count,image_bytes;
    ClearOp background;
    GuestDrawOp draws[3];
    std::uint8_t pixels[16384];
};
struct DemoStatus {
    std::uint32_t abi_version,status,frame,score,misses,ball_x,ball_y,paddle_x,instructions_low,instructions_high;
    char error[256];
};
static_assert(sizeof(LiveDraw)==66068 && alignof(LiveDraw)==4 && offsetof(LiveDraw,pixels)==49684);
static_assert(sizeof(DemoStatus)==296 && alignof(DemoStatus)==4 && offsetof(DemoStatus,error)==40);
extern "C" DemoStatus emulator_demo_start_v1(std::uint32_t (*callback)(LiveDraw));
extern "C" DemoStatus emulator_demo_step_v1(std::uint32_t x,std::uint32_t reset);
extern "C" DemoStatus emulator_demo_stop_v1();

struct ProgramChunk { std::uint32_t length; std::uint8_t bytes[4096]; };
static_assert(sizeof(ProgramChunk)==4100 && alignof(ProgramChunk)==4 && offsetof(ProgramChunk,bytes)==4);
extern "C" std::uint32_t emulator_program_begin_v1(std::uint32_t bytes);
extern "C" std::uint32_t emulator_program_append_v1(ProgramChunk chunk);
extern "C" EngineReport emulator_program_inspect_v1();
extern "C" EngineReport emulator_program_start_v1();
extern "C" EngineReport emulator_program_step_v1(std::uint32_t budget);
extern "C" std::uint32_t emulator_program_close_v1();
