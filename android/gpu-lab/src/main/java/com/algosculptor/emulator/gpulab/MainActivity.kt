package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.content.Intent
import android.graphics.Typeface
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
import java.util.concurrent.CancellationException

class MainActivity : Activity() {
    companion object {
        private const val EXPORT_REQUEST = 10
    }

    private val worker = Executors.newSingleThreadExecutor()
    private lateinit var output: TextView
    private lateinit var runButton: Button
    private lateinit var exportButton: Button
    private var report: String? = null
    @Volatile private var active = true

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
        layout.addView(Button(this).apply { text = "Back to library"; setOnClickListener { finish() } })
        layout.addView(TextView(this).apply {
            text = "Diagnostics"
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
        layout.addView(Button(this).apply {
            text = "View & share logs"
            setOnClickListener { AppLog.showLogDialog(this@MainActivity) }
        })
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
            val saved = ReportStore.read(this, "last-report.json")
            if (saved != null) {
                if (JSONObject(saved).optInt("workloadVersion", 0) == Diagnostics.WORKLOAD_VERSION) {
                    report = saved
                    output.text = Diagnostics.summary(saved)
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
            report?.let { captured ->
                val value = JSONObject(captured)
                if (!value.has("automaticExport")) {
                    Diagnostics.attachCapturedReports(this, value)
                    report = value.toString(2); ReportStore.write(this, "last-report.json", report!!)
                } else Diagnostics.attachCapturedReports(this, JSONObject()) // Recover ancillary caches; keep the published report immutable.
                output.text = Diagnostics.summary(report!!)
            }
        } catch (_: Exception) { }
    }
    private fun runChecks() {
        runButton.isEnabled = false
        exportButton.isEnabled = false
        output.text = "Running the full test batch…"
        worker.execute {
            val result = try { Diagnostics.run(applicationContext, { !active }).toString(2) }
                catch (_: CancellationException) { return@execute }
                catch (error: Exception) { JSONObject().put("allPassed", false).put("error", error.message ?: error.javaClass.simpleName).toString(2) }
            if (!active) return@execute
            try { ReportStore.write(this, "last-report.json", result) } catch (_: Exception) { }
            runOnUiThread {
                if (!isFinishing && !isDestroyed) {
                    report = result
                    output.text = Diagnostics.summary(result)
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
        active = false
        worker.shutdown()
        super.onDestroy()
    }
}
