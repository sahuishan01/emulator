#include "gpu_lab.h"
#include <exception>
#include <cstddef>
#include <cstdint>
#include <jni.h>

#include "engine_abi.h"

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

extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_algosculptor_emulator_gpulab_MainActivity_runShaderChecks(JNIEnv* env, jobject) {
    return copy_engine_report(env, emulator_run_shader_checks_v2());
}

#include <android/native_window_jni.h>
static jbyteArray demo_bytes(JNIEnv* env,const std::string& text) {
    const auto bytes=env->NewByteArray(static_cast<jsize>(text.size()));
    if(bytes)env->SetByteArrayRegion(bytes,0,static_cast<jsize>(text.size()),reinterpret_cast<const jbyte*>(text.data()));
    return bytes;
}
extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_algosculptor_emulator_gpulab_DemoActivity_startDemo(JNIEnv* env,jobject,jobject surface) {
    auto* window=ANativeWindow_fromSurface(env,surface);
    if(!window)return demo_bytes(env,"{\"running\":false,\"error\":\"Android surface unavailable\"}");
    const auto result=start_homebrew(window);ANativeWindow_release(window);return demo_bytes(env,result);
}
extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_algosculptor_emulator_gpulab_DemoActivity_stepDemo(JNIEnv* env,jobject,jint x,jboolean reset) {
    return demo_bytes(env,step_homebrew(static_cast<uint32_t>(x),reset ? 1 : 0));
}
extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_algosculptor_emulator_gpulab_DemoActivity_stopDemo(JNIEnv* env,jobject) {
    return demo_bytes(env,stop_homebrew());
}
