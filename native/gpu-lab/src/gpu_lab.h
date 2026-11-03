#pragma once
#include <string>

struct GpuReport {
    bool passed;
    std::string json;
};

GpuReport run_gpu_checks();
#ifndef __ANDROID__
GpuReport run_replay_evidence_self_test();
#endif

std::string start_homebrew(void* window);
std::string step_homebrew(unsigned x,unsigned reset);
std::string stop_homebrew();
std::string start_texture_scene(void* window,unsigned variant);
std::string stop_texture_scene();
