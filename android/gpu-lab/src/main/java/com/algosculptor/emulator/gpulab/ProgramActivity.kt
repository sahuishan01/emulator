package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.graphics.Typeface
import android.os.Bundle
import android.widget.Button
import android.widget.TextView
import android.widget.ScrollView
import org.json.JSONObject
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** Text-mode static NRO execution. Guest faults are explicit, bounded results. */
class ProgramActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private val stop = AtomicBoolean(false)
    private lateinit var state: TextView
    private lateinit var output: TextView
    private lateinit var stopButton: Button
    private val store get() = (application as EmulatorApplication).library
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val ui = AppUi(this); val root = ui.column()
        root.addView(ui.button("Back to library") { finish() }); ui.space(root, 24)
        root.addView(ui.label("HOMEBREW SESSION")); ui.space(root, 10)
        val title = ui.text("Opening homebrew…", 26f, bold = true); root.addView(title); ui.space(root, 16)
        state = ui.text("Loading…", 16f, ui.muted); root.addView(state); ui.space(root, 18)
        output = ui.text("", 14f).apply { typeface = Typeface.MONOSPACE; setTextIsSelectable(true) }; root.addView(output)
        ui.space(root, 20)
        stopButton = ui.button("Stop session") { stop.set(true) }; root.addView(stopButton)
        root.addView(ui.button("Copy output") { getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Homebrew output", output.text)) })
        ui.space(root, 16)
        root.addView(ui.text("Program output only. General homebrew graphics and commercial game loading are still in development.", 13f, ui.muted))
        ui.install(ScrollView(this).apply { addView(root) })
        val id = intent.getStringExtra("entryId") ?: ""
        store.find(id) { entry ->
            if (isDestroyed || isFinishing || stop.get()) return@find
            title.text = entry?.title ?: "Homebrew unavailable"
            val file = entry?.let { store.file(it) }
            if (entry == null || file == null || entry.mode != "static") { state.text = "The imported file is unavailable."; stopButton.isEnabled = false; return@find }
            worker.execute {
                if (stop.get()) return@execute
                var result = JSONObject()
                try {
                    result = JSONObject(String(NativePrograms.start(file.absolutePath), Charsets.UTF_8))
                    if (result.optBoolean("ok")) {
                        store.recordLaunch(id)
                        var lastShow = 0L
                        while (!stop.get() && result.optString("state") == "running") {
                            result = JSONObject(String(NativePrograms.step(1024), Charsets.UTF_8))
                            if (System.currentTimeMillis() - lastShow > 1600 || result.optString("state") != "running") {
                                show(result); lastShow = System.currentTimeMillis()
                            }
                        }
                    }
                    if (stop.get()) result.put("state", "stopped")
                    store.recordResult(id, when (result.optString("state")) {
                        "exited" -> "Exited normally"
                        "stopped" -> "Session stopped"
                        "waiting" -> "Stopped while waiting for guest events"
                        else -> "Needs more runtime support"
                    })
                } catch (e: Exception) { result = JSONObject().put("state", "error").put("error", e.message ?: "Could not run this homebrew.") }
                finally { NativePrograms.stop() }
                show(result, finished = true)
            }
        }
    }
    private fun show(value: JSONObject, finished: Boolean = false) {
        val text = value.optString("output")
        val label = when (value.optString("state")) {
            "exited" -> "Exited normally"
            "stopped" -> "Session stopped"
            "waiting" -> "Stopped while waiting for guest events"
            "running" -> "Running · ${value.optLong("instructions")} instructions"
            "limit" -> "Execution limit reached"
            else -> "This homebrew could not run"
        }
        val error = value.optString("error")
        runOnUiThread { if (!isDestroyed && !isFinishing) {
            state.text = if (error.isEmpty()) label else "$label\n$error"
            output.text = if (text.isEmpty() && finished) "No program output." else if (value.optBoolean("outputTruncated")) "$text\n… Output truncated." else text
            stopButton.isEnabled = !finished
        } }
    }
    override fun onPause() { stop.set(true); super.onPause() }
    override fun onDestroy() { stop.set(true); worker.shutdown(); super.onDestroy() }
}
