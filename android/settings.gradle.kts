pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "OriginalEmulator"
include(":gpu-lab")

// Separate QA-only provider; never packaged in the emulator application.
include(":qa-documents")
project(":qa-documents").projectDir = file("../tools/qa-documents")
