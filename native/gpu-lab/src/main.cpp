#include "gpu_lab.h"
#include <iostream>

int main() {
    const auto report = run_gpu_checks();
    std::cout << report.json << '\n';
    return report.passed ? 0 : 1;
}
