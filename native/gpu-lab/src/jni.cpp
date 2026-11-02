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

#include <fstream>
#include <stdexcept>
static void upload_program(const char* path) {
    (void)emulator_program_close_v1();
    std::ifstream input(path,std::ios::binary|std::ios::ate);
    if(!input)throw std::runtime_error("Could not open this homebrew file");
    const auto end=input.tellg();
    if(end<128 || end>32*1024*1024)throw std::runtime_error("Homebrew files must be between128 bytes and32MiB");
    input.seekg(0);
    if(emulator_program_begin_v1(static_cast<uint32_t>(end)))throw std::runtime_error("Could not create homebrew upload");
    ProgramChunk chunk{};
    while(input) {
        input.read(reinterpret_cast<char*>(chunk.bytes),sizeof(chunk.bytes));chunk.length=static_cast<uint32_t>(input.gcount());
        if(chunk.length && emulator_program_append_v1(chunk))throw std::runtime_error("Homebrew upload exceeds its declared size");
    }
    if(!input.eof())throw std::runtime_error("Could not read this homebrew file");
}
static jbyteArray open_program(JNIEnv* env,jstring path,bool inspect) {
    const char* text=env->GetStringUTFChars(path,nullptr);
    if(!text)return nullptr;
    try {
        upload_program(text);env->ReleaseStringUTFChars(path,text);text=nullptr;
        const auto result=inspect ? emulator_program_inspect_v1() : emulator_program_start_v1();
        if(inspect)(void)emulator_program_close_v1();
        return copy_engine_report(env,result);
    } catch(const std::exception& error) {
        if(text)env->ReleaseStringUTFChars(path,text);
        (void)emulator_program_close_v1();const auto type=env->FindClass("java/lang/RuntimeException");
        if(type)env->ThrowNew(type,error.what());return nullptr;
    }
}
extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_algosculptor_emulator_gpulab_NativePrograms_inspect(JNIEnv* env,jobject,jstring path) { return open_program(env,path,true); }
extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_algosculptor_emulator_gpulab_NativePrograms_start(JNIEnv* env,jobject,jstring path) { return open_program(env,path,false); }
extern "C" JNIEXPORT jbyteArray JNICALL
Java_com_algosculptor_emulator_gpulab_NativePrograms_step(JNIEnv* env,jobject,jint budget) { return copy_engine_report(env,emulator_program_step_v1(static_cast<uint32_t>(budget))); }
extern "C" JNIEXPORT void JNICALL
Java_com_algosculptor_emulator_gpulab_NativePrograms_stop(JNIEnv*,jobject) { (void)emulator_program_close_v1(); }
