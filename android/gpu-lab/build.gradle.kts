plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.algosculptor.emulator.gpulab"
    compileSdk = 35
    buildToolsVersion = "34.0.0"
    ndkVersion = "28.2.13676358"

    defaultConfig {
        applicationId = "com.algosculptor.emulator.gpulab"
        minSdk = 31
        targetSdk = 35
        versionCode = 13
        versionName = "0.13.0-dev"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
        externalNativeBuild {
            cmake {
                arguments += "-DANDROID_STL=c++_static"
                cppFlags += "-std=c++17"
            }
        }
    }
    externalNativeBuild {
        cmake {
            path = file("../../native/gpu-lab/CMakeLists.txt")
            version = "3.22.1"
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
}

val engineRoot = rootProject.projectDir.parentFile
val buildOriginalEngine by tasks.registering(Exec::class) {
    workingDir(engineRoot)
    commandLine("python3", "tools/build_android_engine.py")
    inputs.files(fileTree(engineRoot.resolve("crates")) { include("**/*.rs", "**/Cargo.toml") })
    inputs.files(fileTree(engineRoot.resolve("examples/homebrew")) { include("*.nro", "*.S") })
    inputs.files(engineRoot.resolve("Cargo.toml"), engineRoot.resolve("Cargo.lock"), engineRoot.resolve("tools/build_android_engine.py"))
    inputs.property("rustToolchain", "1.85.1")
    inputs.property("engineNdk", "28.2.13676358")
    outputs.files(engineRoot.resolve("target/android-engine/arm64-v8a/libemulator_android_ffi.a"),
        engineRoot.resolve("target/android-engine/x86_64/libemulator_android_ffi.a"))
}
tasks.configureEach {
    if (name.startsWith("configureCMake") || name.startsWith("buildCMake")) dependsOn(buildOriginalEngine)
}
