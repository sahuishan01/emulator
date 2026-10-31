#include "gpu_lab.h"
#include <exception>
#include <cstddef>
#include <cstdint>
#include <jni.h>

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

static jbyteArray copy_engine_report(JNIEnv* env, const EngineReport& report) {
    if (report.abi_version != 2 || report.length > sizeof(report.bytes)) {
        const auto type = env->FindClass("java/lang/RuntimeException");
        if (type) env->ThrowNew(type, "Invalid original engine report ABI");
        return nullptr;
    }
    const auto bytes = env->NewByteArray(static_cast<jsize>(report.length));
    if (bytes) env->SetByteArrayRegion(bytes, 0, static_cast<jsize>(report.length), reinterpret_cast<const jbyte*>(report.bytes));
    return bytes;
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_algosculptor_emulator_gpulab_MainActivity_runEngineChecks(JNIEnv* env, jobject) {
    return copy_engine_report(env, emulator_run_reference_guests_v2());
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_algosculptor_emulator_gpulab_MainActivity_runGpuCoreChecks(JNIEnv* env, jobject) {
    return copy_engine_report(env, emulator_run_gpu_core_checks_v2());
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_algosculptor_emulator_gpulab_MainActivity_runGpuChecks(JNIEnv* env, jobject) {
    try {
        const auto report = run_gpu_checks();
        const auto bytes = env->NewByteArray(static_cast<jsize>(report.json.size()));
        if (bytes) env->SetByteArrayRegion(bytes, 0, static_cast<jsize>(report.json.size()), reinterpret_cast<const jbyte*>(report.json.data()));
        return bytes;
    } catch (const std::exception& error) {
        const auto type = env->FindClass("java/lang/RuntimeException");
        if (type) env->ThrowNew(type, error.what());
        return nullptr;
    }
}

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_algosculptor_emulator_gpulab_MainActivity_runNvChecks(JNIEnv* env, jobject) {
    return copy_engine_report(env, emulator_run_nv_checks_v2());
}
