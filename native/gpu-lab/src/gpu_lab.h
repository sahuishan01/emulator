#pragma once
#include <string>

struct GpuReport {
    bool passed;
    std::string json;
};

GpuReport run_gpu_checks();

std::string start_homebrew(void* window);
std::string step_homebrew(unsigned x,unsigned reset);
std::string stop_homebrew();
