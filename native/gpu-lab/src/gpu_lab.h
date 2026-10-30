#pragma once
#include <string>

struct GpuReport {
    bool passed;
    std::string json;
};

GpuReport run_gpu_checks();
