package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.content.Intent
import android.graphics.Typeface
import android.os.Build
import android.os.Bundle
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import org.json.JSONObject
import java.util.concurrent.Executors

class MainActivity : Activity() {
    companion object {
        init { System.loadLibrary("emulator_gpu_lab") }
        private const val EXPORT_REQUEST = 10
        private const val WORKLOAD_VERSION = 20
    }

    private external fun runNvChecks(): ByteArray
    private external fun runShaderChecks(): ByteArray
    private external fun runGpuChecks(): ByteArray
    private external fun runEngineChecks(): ByteArray
    private external fun runGpuCoreChecks(): ByteArray
    private val worker = Executors.newSingleThreadExecutor()
    private lateinit var output: TextView
    private lateinit var runButton: Button
    private lateinit var exportButton: Button
    private var report: String? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val padding = (20 * resources.displayMetrics.density).toInt()
        val layout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(padding, padding, padding, padding)
            setOnApplyWindowInsetsListener { view, insets ->
                val system = insets.getInsets(WindowInsets.Type.systemBars())
                view.setPadding(padding + system.left, padding + system.top, padding + system.right, padding + system.bottom)
                insets
            }
        }
        layout.addView(TextView(this).apply {
            text = "Emulator GPU Lab"
            textSize = 24f
            setTypeface(null, Typeface.BOLD)
        })
        layout.addView(TextView(this).apply {
            text = "Run the full CPU, NV, memory, shader translation and Vulkan test batch."
            textSize = 15f
            setPadding(0, padding / 2, 0, padding / 2)
        })
        runButton = Button(this).apply {
            text = "Run all checks"
            setOnClickListener { runChecks() }
        }
        exportButton = Button(this).apply {
            text = "Export report"
            isEnabled = false
            setOnClickListener {
                startActivityForResult(Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                    addCategory(Intent.CATEGORY_OPENABLE)
                    type = "application/json"
                    putExtra(Intent.EXTRA_TITLE, "emulator-gpu-report.json")
                }, EXPORT_REQUEST)
            }
        }
        layout.addView(Button(this).apply {
            text = "Play homebrew demo"
            setOnClickListener { startActivity(Intent(this@MainActivity, DemoActivity::class.java)) }
        })
        layout.addView(runButton)
        layout.addView(exportButton)
        output = TextView(this).apply {
            text = "Ready. Run all checks once, then export one report."
            textSize = 13f
            typeface = Typeface.MONOSPACE
            setTextIsSelectable(true)
        }
        layout.addView(ScrollView(this).apply { addView(output) }, LinearLayout.LayoutParams(-1, 0, 1f))
        setContentView(layout)
        window.insetsController?.setSystemBarsAppearance(
            WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
            WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
        )
        try {
            val previous = filesDir.resolve("last-report.json")
            if (previous.exists()) {
                val saved = previous.readText()
                if (JSONObject(saved).optInt("workloadVersion", 0) == WORKLOAD_VERSION) {
                    report = saved
                    output.text = summarize(saved)
                    exportButton.isEnabled = true
                } else {
                    output.text = "Ready. Run checks again for this version's updated tests."
                }
            }
        } catch (_: Exception) { }
    }

    override fun onResume() {
        super.onResume()
        try {
            val file = filesDir.resolve("last-demo-report.json")
            if (file.exists() && report != null) {
                val demo = JSONObject(file.readText())
                if (demo.optString("appVersion") == packageManager.getPackageInfo(packageName, 0).versionName) {
                    report = JSONObject(report!!).put("demo", demo).toString(2)
                    filesDir.resolve("last-report.json").writeText(report!!)
                    output.text = summarize(report!!)
                }
            }
        } catch (_: Exception) { }
    }

    private fun summarize(text: String): String = try {
        val root = JSONObject(text)
        var total = 0
        var passed = 0
        var complete = true
        val groups = listOf("engine" to "Guest", "nv" to "NV batch", "gpuCore" to "GPU core", "shader" to "Shaders", "gpu" to "Vulkan")
        val details = groups.map { (key, label) ->
            val tests = root.optJSONObject(key)?.optJSONArray("tests")
            val count = tests?.length() ?: 0
            if (count == 0) complete = false
            val good = (0 until count).count { tests!!.getJSONObject(it).optBoolean("passed", false) }
            total += count
            passed += good
            "$label: $good/$count"
        }
        val title = if (!complete) "Checks incomplete. See errors below" else if (root.optBoolean("allPassed", false) && total > 0) "All $total checks passed" else "$passed/$total checks passed; see failures below"
        "$title\n${details.joinToString(" · ")}\n\n$text"
    } catch (_: Exception) { text }

    private fun runChecks() {
        runButton.isEnabled = false
        exportButton.isEnabled = false
        output.text = "Running the full test batch…"
        worker.execute {
            val result = try {
                val engine = try {
                    JSONObject(String(runEngineChecks(), Charsets.UTF_8))
                } catch (error: Exception) {
                    JSONObject().put("scope", "original-guest-reference-programs").put("allPassed", false)
                        .put("error", error.message ?: error.javaClass.simpleName)
                }
                val gpuCore = try {
                    JSONObject(String(runGpuCoreChecks(), Charsets.UTF_8))
                } catch (error: Exception) {
                    JSONObject().put("scope", "original-gpu-memory-and-command-reference").put("allPassed", false)
                        .put("error", error.message ?: error.javaClass.simpleName)
                }
                val nv = try {
                    JSONObject(String(runNvChecks(), Charsets.UTF_8))
                } catch (error: Exception) {
                    JSONObject().put("scope", "original-nv-protocol-reference").put("allPassed", false)
                        .put("error", error.message ?: error.javaClass.simpleName)
                }
                val shader = try {
                    JSONObject(String(runShaderChecks(), Charsets.UTF_8))
                } catch (error: Exception) {
                    JSONObject().put("scope", "original-sm53-shader-reference").put("allPassed", false)
                        .put("error", error.message ?: error.javaClass.simpleName)
                }
                val gpu = try {
                    JSONObject(String(runGpuChecks(), Charsets.UTF_8))
                } catch (error: Exception) {
                    JSONObject().put("scope", "host-vulkan-only").put("allPassed", false)
                        .put("error", error.message ?: error.javaClass.simpleName)
                }
                JSONObject().apply {
                    val demoFile = filesDir.resolve("last-demo-report.json")
                    try { if (demoFile.exists()) {
                        val demo = JSONObject(demoFile.readText())
                        if (demo.optString("appVersion") == packageManager.getPackageInfo(packageName, 0).versionName) put("demo", demo)
                    } } catch (_: Exception) { }
                    put("schemaVersion", 4)
                    put("workloadVersion", WORKLOAD_VERSION)
                    put("scope", "original-engine-gpu-core-and-host-vulkan")
                    put("engine", engine)
                    put("gpuCore", gpuCore)
                    put("nv", nv)
                    put("gpu", gpu)
                    put("shader", shader)
                    put("allPassed", engine.optBoolean("allPassed", false) && gpuCore.optBoolean("allPassed", false) && nv.optBoolean("allPassed", false) && shader.optBoolean("allPassed", false) && gpu.optBoolean("allPassed", false))
                    put("phoneModel", Build.MODEL)
                    put("manufacturer", Build.MANUFACTURER)
                    put("socModel", Build.SOC_MODEL)
                    put("androidVersion", Build.VERSION.RELEASE)
                    put("androidSdk", Build.VERSION.SDK_INT)
                    put("appVersion", packageManager.getPackageInfo(packageName, 0).versionName)
                    put("capturedAtEpochMs", System.currentTimeMillis())
                }.toString(2)
            } catch (error: Exception) {
                JSONObject().put("scope", "original-engine-gpu-core-and-host-vulkan").put("allPassed", false)
                    .put("error", error.message ?: error.javaClass.simpleName).toString(2)
            }
            try { filesDir.resolve("last-report.json").writeText(result) } catch (_: Exception) { }
            runOnUiThread {
                if (!isFinishing && !isDestroyed) {
                    report = result
                    output.text = summarize(result)
                    runButton.isEnabled = true
                    exportButton.isEnabled = true
                }
            }
        }
    }

    @Deprecated("Activity result compatibility for this minimal lab")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == EXPORT_REQUEST && resultCode == RESULT_OK) {
            val text = report ?: return
            val uri = data?.data ?: return
            try {
                contentResolver.openOutputStream(uri, "wt")?.use { it.write(text.toByteArray(Charsets.UTF_8)) }
                    ?: error("Could not open report destination")
                Toast.makeText(this, "Report exported", Toast.LENGTH_SHORT).show()
            } catch (error: Exception) {
                Toast.makeText(this, "Export failed: ${error.message}", Toast.LENGTH_LONG).show()
            }
        }
    }

    override fun onDestroy() {
        worker.shutdown()
        super.onDestroy()
    }
}
