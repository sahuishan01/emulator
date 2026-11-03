#include "gpu_lab.h"
#include <iostream>
int main() {
    const auto report=run_replay_evidence_self_test();
    std::cout<<report.json<<'\n';
    return report.passed?0:1;
}
