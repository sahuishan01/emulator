plugins { id("com.android.application") }
android {
    namespace = "com.algosculptor.emulator.qadocuments"
    compileSdk = 35
    buildToolsVersion = "34.0.0"
    defaultConfig { applicationId = "com.algosculptor.emulator.qadocuments"; minSdk = 31; targetSdk = 35; versionCode = 1; versionName = "1" }
    sourceSets["main"].assets.srcDir(layout.buildDirectory.dir("generated/qa-assets"))
    compileOptions { sourceCompatibility = JavaVersion.VERSION_17; targetCompatibility = JavaVersion.VERSION_17 }
}

val generateDocuments by tasks.registering(Exec::class) {
    workingDir(rootProject.projectDir.parentFile)
    commandLine("python3", "tools/generate_qa_documents.py")
    inputs.files(rootProject.projectDir.parentFile.resolve("tools/generate_qa_documents.py"), rootProject.projectDir.parentFile.resolve("examples/homebrew/hello.nro"), rootProject.projectDir.parentFile.resolve("examples/homebrew/ipc-clone.nro"))
    outputs.dir(layout.buildDirectory.dir("generated/qa-assets"))
}
tasks.named("preBuild") { dependsOn(generateDocuments) }
