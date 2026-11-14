package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.graphics.Typeface
import android.os.Bundle
import android.view.View
import android.widget.Button
import android.widget.TextView
import android.widget.ScrollView
import org.json.JSONObject
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/** Text-mode static NRO and package NSP execution. Guest faults are explicit, bounded results. */
class ProgramActivity : Activity() {
    companion object {
        private const val REQ_PROD_KEYS = 201
    }

    private val worker = Executors.newSingleThreadExecutor()
    private val stop = AtomicBoolean(false)
    private lateinit var state: TextView
    private lateinit var output: TextView
    private lateinit var stopButton: Button
    private lateinit var installKeysButton: Button
    private val store get() = (application as EmulatorApplication).library
    private var activeFile: File? = null
    private var activeId: String = ""

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val ui = AppUi(this); val root = ui.column()
        root.addView(ui.button("Back to library") { finish() }); ui.space(root, 24)
        root.addView(ui.label("HOMEBREW SESSION")); ui.space(root, 10)
        val title = ui.text("Opening homebrew…", 26f, bold = true); root.addView(title); ui.space(root, 16)
        state = ui.text("Loading…", 16f, ui.muted); root.addView(state); ui.space(root, 18)

        installKeysButton = ui.button("Install prod.keys", true) {
            startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE); type = "*/*"
            }, REQ_PROD_KEYS)
        }.apply { visibility = View.GONE }
        root.addView(installKeysButton); ui.space(root, 12)

        output = ui.text("", 14f).apply { typeface = Typeface.MONOSPACE; setTextIsSelectable(true) }; root.addView(output)
        ui.space(root, 20)
        stopButton = ui.button("Stop session") { stop.set(true) }; root.addView(stopButton)
        root.addView(ui.button("Copy output") { getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Homebrew output", output.text)) })
        ui.space(root, 10)
        root.addView(ui.button("Share logs", true) { AppLog.showLogDialog(this) })
        ui.space(root, 16)
        root.addView(ui.text("Program output only. General homebrew graphics and commercial game loading are still in development.", 13f, ui.muted))
        ui.install(ScrollView(this).apply { addView(root) })

        val id = intent.getStringExtra("entryId") ?: ""
        activeId = id
        store.find(id) { entry ->
            if (isDestroyed || isFinishing || stop.get()) return@find
            title.text = entry?.title ?: "Program unavailable"
            val file = entry?.let { store.file(it) }
            if (entry == null || file == null || (entry.mode != "static" && entry.mode != "package")) {
                state.text = "The imported file is unavailable."
                stopButton.isEnabled = false
                AppLog.w("ProgramActivity", "File unavailable or unsupported mode: entry=$entry")
                return@find
            }
            activeFile = file
            launchSession(file, id, entry.title, entry.mode)
        }
    }

    private fun launchSession(file: File, id: String, title: String, mode: String) {
        stop.set(false)
        installKeysButton.visibility = View.GONE
        state.text = "Starting session…"
        output.text = ""
        stopButton.isEnabled = true
        AppLog.i("ProgramActivity", "Starting program session: title='$title', mode=$mode, file=${file.absolutePath}")
        worker.execute {
            if (stop.get()) return@execute
            var result = JSONObject()
            try {
                result = JSONObject(String(NativePrograms.start(file.absolutePath), Charsets.UTF_8))
                AppLog.i("ProgramActivity", "NativePrograms.start returned: ok=${result.optBoolean("ok")}, state=${result.optString("state")}")
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
                val finalState = result.optString("state")
                AppLog.i("ProgramActivity", "Session finished: state=$finalState, instructions=${result.optLong("instructions")}, error='${result.optString("error")}'")
                store.recordResult(id, when (finalState) {
                    "exited" -> "Exited normally"
                    "stopped" -> "Session stopped"
                    "waiting" -> "Stopped while waiting for guest events"
                    else -> "Needs more runtime support"
                })
            } catch (e: Exception) {
                AppLog.e("ProgramActivity", "Error executing program: ${e.message}", e)
                result = JSONObject().put("state", "error").put("error", e.message ?: "Could not run this program.")
            } finally {
                NativePrograms.stop()
            }
            show(result, finished = true)
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
        val needsKeys = error.contains("prod.keys") || error.contains("decryption keys")
        runOnUiThread {
            if (!isDestroyed && !isFinishing) {
                state.text = if (error.isEmpty()) label else "$label\n$error"
                output.text = if (text.isEmpty() && finished) "No program output." else if (value.optBoolean("outputTruncated")) "$text\n… Output truncated." else text
                stopButton.isEnabled = !finished
                if (finished && needsKeys) {
                    installKeysButton.visibility = View.VISIBLE
                } else if (!needsKeys) {
                    installKeysButton.visibility = View.GONE
                }
            }
        }
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQ_PROD_KEYS && resultCode == RESULT_OK && data?.data != null) {
            val (ok, msg) = KeyManager.installFromUri(this, data.data!!, false)
            if (ok) {
                val f = activeFile
                if (f != null) {
                    launchSession(f, activeId, f.name, "package")
                }
            } else {
                AlertDialog.Builder(this)
                    .setTitle("Key Installation Failed")
                    .setMessage(msg)
                    .setPositiveButton("OK", null)
                    .show()
            }
        }
    }

    override fun onPause() { stop.set(true); super.onPause() }
    override fun onDestroy() { stop.set(true); worker.shutdown(); super.onDestroy() }
}
