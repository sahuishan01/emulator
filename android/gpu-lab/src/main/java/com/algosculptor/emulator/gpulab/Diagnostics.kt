package com.algosculptor.emulator.gpulab

import android.content.Context
import android.os.Build
import android.view.Surface
import org.json.JSONObject
import java.util.concurrent.CancellationException

object NativeScenes {
    init { System.loadLibrary("emulator_gpu_lab") }
    val patterns = listOf("Texture", "BGRA texture", "Updated texture", "Indexed byte quad", "Indexed short quad", "Indexed triangle", "Tiled RGBA", "Multi-GOB RGBA", "BC1 texture", "Tiled BC1 alpha", "BC3 alpha", "Tiled BC3 alpha", "Depth never", "Near depth occlusion", "Equal depth", "Less or equal depth", "Reverse depth occlusion", "Not equal depth", "Greater or equal depth", "Always depth", "Read-only depth", "Sloped depth", "Reverse sloped depth", "Clear to far", "Clear to near", "Clear right region", "Clipped depth clear", "Empty depth clear", "Overlapping depth clears", "Clear between draws", "Near clear between draws", "Clear after draws", "Partial clear between draws", "Clear with read-only draws", "Clear with never comparison", "Reverse depth clear", "Equal depth clear", "Single pixel depth clear", "Full and partial clear", "Projected texture", "Small W texture", "Variable W texture", "Projected inset texture", "Reversed projected BGRA", "Projected right scissor", "Nonpower W texture", "Subpixel projected edges", "Projected byte indices", "Projected short indices", "Projected indexed triangle", "Projected indexed scissor", "Projected sloped depth", "Projected reverse depth", "Projected read-only depth", "Divided depth and position")
    external fun start(surface: Surface, variant: Int): ByteArray
    external fun stop(): ByteArray
}

/** Shared native checks and report shape for manual and automatic workflows. */
object Diagnostics {
    const val WORKLOAD_VERSION = 30
    init { System.loadLibrary("emulator_gpu_lab") }
    private external fun runEngineChecks(): ByteArray
    private external fun runGpuCoreChecks(): ByteArray
    private external fun runNvChecks(): ByteArray
    private external fun runShaderChecks(): ByteArray
    private external fun runGpuChecks(): ByteArray

    fun attachCapturedReports(context: Context, target: JSONObject) {
        for ((name, key) in listOf("last-demo-report.json" to "demo", "last-texture-report.json" to "textureScene")) {
            try { ReportStore.read(context, name)?.let {
                val value = JSONObject(it)
                if (value.optString("appVersion") == context.packageManager.getPackageInfo(context.packageName, 0).versionName) target.put(key, value)
            } } catch (_: Exception) { }
        }
    }

    fun run(context: Context, cancelled: () -> Boolean = { false }, progress: (String) -> Unit = {}): JSONObject {
        val groups = listOf(
            Triple("engine", "Guest programs", ::runEngineChecks),
            Triple("gpuCore", "GPU memory and commands", ::runGpuCoreChecks),
            Triple("nv", "NV services", ::runNvChecks),
            Triple("shader", "Shader translation", ::runShaderChecks),
            Triple("gpu", "Vulkan rendering", ::runGpuChecks),
        )
        val root = JSONObject()
        var all = true
        for ((key, label, run) in groups) {
            if (cancelled()) throw CancellationException()
            progress(label)
            val result = try { JSONObject(String(run(), Charsets.UTF_8)) }
                catch (error: Exception) { JSONObject().put("allPassed", false).put("error", error.message ?: error.javaClass.simpleName) }
            root.put(key, result)
            all = all && result.optBoolean("allPassed") && (result.optJSONArray("tests")?.length() ?: 0) > 0
        }
        if (cancelled()) throw CancellationException()
        attachCapturedReports(context, root)
        return root.apply {
            put("schemaVersion", 4); put("workloadVersion", WORKLOAD_VERSION)
            put("scope", "original-engine-gpu-core-and-host-vulkan"); put("allPassed", all)
            put("phoneModel", Build.MODEL); put("manufacturer", Build.MANUFACTURER); put("socModel", Build.SOC_MODEL)
            put("androidVersion", Build.VERSION.RELEASE); put("androidSdk", Build.VERSION.SDK_INT)
            put("appVersion", context.packageManager.getPackageInfo(context.packageName, 0).versionName)
            put("capturedAtEpochMs", System.currentTimeMillis())
        }
    }

    fun summary(text: String): String = try {
        val root = JSONObject(text)
        var total = 0; var passed = 0; var complete = true
        val details = listOf("engine" to "Guest", "nv" to "NV batch", "gpuCore" to "GPU core", "shader" to "Shaders", "gpu" to "Vulkan").map { (key, label) ->
            val tests = root.optJSONObject(key)?.optJSONArray("tests")
            val count = tests?.length() ?: 0
            if (count == 0) complete = false
            val good = (0 until count).count { tests!!.getJSONObject(it).optBoolean("passed") }
            total += count; passed += good; "$label: $good/$count"
        }
        val title = if (!complete) "Checks incomplete. See errors below" else if (root.optBoolean("allPassed") && total > 0) "All $total checks passed" else "$passed/$total checks passed; see failures below"
        val patterns = root.optJSONObject("patternBatch")?.let { "\nPatterns: ${it.optInt("passedPatterns")}/${it.optInt("totalPatterns")}" } ?: ""
        "$title\n${details.joinToString(" · ")}$patterns\n\n$text"
    } catch (_: Exception) { text }
}
