package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.content.ClipData
import android.content.Intent
import android.graphics.SurfaceTexture
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.os.SystemClock
import android.view.Surface
import android.view.TextureView
import android.view.View
import android.view.WindowManager
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID
import java.util.concurrent.CancellationException

/** One tap owns the visible patterns, full diagnostics and automatic JSON export. */
class BatchActivity : Activity(), TextureView.SurfaceTextureListener {
    companion object { const val ACTION_LABEL = "Test all & export JSON" }
    private val thread = HandlerThread("FullDeviceTest")
    private lateinit var worker: Handler
    private val main = Handler(Looper.getMainLooper())
    private lateinit var view: TextureView
    private lateinit var status: TextView
    private lateinit var detail: TextView
    private lateinit var resultSummary: TextView
    private lateinit var resultPanel: ScrollView
    private lateinit var action: Button
    private lateinit var share: Button
    @Volatile private var generation = 0
    private var resumed = false
    private var running = false
    private var publishing = false
    private var completed = false
    private var userCancelled = false
    private var frameUpdates = 0L
    private var runId = ""
    private var startedAt = 0L
    private var results = JSONArray()
    private var readyReport: JSONObject? = null
    private var saved: ReportExporter.Saved? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        thread.start(); worker = Handler(thread.looper)
        runId = savedInstanceState?.getString("runId") ?: ""
        userCancelled = savedInstanceState?.getBoolean("userCancelled") ?: false
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val ui = AppUi(this); val root = ui.column()
        root.addView(ui.text("Full device test", 26f, bold = true)); ui.space(root, 8)
        status = ui.text("Preparing test…", 17f).apply { setLines(2) }; root.addView(status)
        view = TextureView(this).apply { surfaceTextureListener = this@BatchActivity; contentDescription = "Automatic test patterns" }
        root.addView(view, LinearLayout.LayoutParams(-1, 0, 1f))
        resultSummary = ui.text("", 16f)
        resultPanel = ScrollView(this).apply { addView(resultSummary); visibility = View.GONE }
        root.addView(resultPanel, LinearLayout.LayoutParams(-1, 0, 1f)); ui.space(root, 8)
        detail = ui.text("Runs all patterns and checks.\nSaves JSON to Downloads/Emulator.", 13f, ui.muted).apply { setLines(3) }; root.addView(detail)
        val buttons = LinearLayout(this)
        action = ui.button("Cancel test") {
            when { running -> interrupt(true); readyReport != null && saved == null -> beginExport(generation, readyReport!!); else -> {
                userCancelled = false; completed = false; readyReport = null; saved = null
                resultPanel.visibility = View.GONE; view.visibility = View.VISIBLE; maybeStart()
            } }
        }
        share = ui.button("Share JSON", true) { shareSaved() }.apply { isEnabled = false }
        buttons.addView(action, LinearLayout.LayoutParams(0, -2, 1f)); buttons.addView(share, LinearLayout.LayoutParams(0, -2, 1f)); root.addView(buttons)
        root.addView(ui.button("Back to library") { if (!publishing) finish() }); ui.install(root)
        restoreSaved()
        if (userCancelled && !completed) { status.text = "Test cancelled"; action.text = "Run again" }
    }
    private fun restoreSaved() {
        if (runId.isEmpty()) return
        try {
            val root = JSONObject(ReportStore.read(this, "last-report.json") ?: return)
            if (root.optString("appVersion") != packageManager.getPackageInfo(packageName, 0).versionName || root.optJSONObject("patternBatch")?.optString("runId") != runId) return
            val export = root.optJSONObject("automaticExport") ?: return
            val uri = Uri.parse(export.getString("uri"))
            readyReport = root
            if (ReportExporter.published(this, uri)) showSaved(root, ReportExporter.Saved(uri, export.getString("filename")))
            else { userCancelled = true; showSummary(root); status.text = "Report ready. Saving was interrupted."; action.text = "Retry export" }
        } catch (_: Exception) { }
    }
    private fun maybeStart() {
        if (!resumed || !view.isAvailable || running || publishing || completed || userCancelled) return
        val texture = view.surfaceTexture ?: return
        val current = ++generation
        runId = "${System.currentTimeMillis()}-${UUID.randomUUID().toString().take(8)}"
        startedAt = System.currentTimeMillis(); results = JSONArray(); readyReport = null; saved = null
        running = true; action.text = "Cancel test"; action.isEnabled = true; share.isEnabled = false
        detail.text = "Runs all patterns and checks.\nSaves JSON to Downloads/Emulator."
        pattern(current, texture, 0)
    }
    private fun current(token: Int) = token == generation
    private fun persistScene(value: JSONObject, id: String) {
        if (value.optLong("completedFrames") == 0L && value.optString("error").isEmpty()) return
        value.put("appVersion", packageManager.getPackageInfo(packageName, 0).versionName).put("batchRunId", id).put("capturedAtEpochMs", System.currentTimeMillis())
        try { ReportStore.write(applicationContext, "last-texture-report.json", value.toString(2)) } catch (_: Exception) { }
    }
    private fun closeNative(id: String) {
        try { persistScene(JSONObject(String(NativeScenes.stop(), Charsets.UTF_8)), id) } catch (_: Exception) { }
    }
    private fun pattern(token: Int, texture: SurfaceTexture, variant: Int) {
        if (!current(token) || !resumed) return
        status.text = "Pattern ${variant + 1} of ${NativeScenes.patterns.size}\n${NativeScenes.patterns[variant]}"
        val before = frameUpdates; val id = runId
        worker.post {
            if (!current(token)) return@post
            closeNative(id)
            if (!current(token)) return@post
            val surface = Surface(texture)
            val result = try { JSONObject(String(NativeScenes.start(surface, variant), Charsets.UTF_8)) }
                catch (error: Exception) { JSONObject().put("error", error.message ?: "Could not render the pattern") }
                finally { surface.release() }
            main.post { if (current(token) && resumed) observePattern(token, texture, variant, before, result, SystemClock.uptimeMillis() + 3000) }
        }
    }
    private fun observePattern(token: Int, texture: SurfaceTexture, variant: Int, before: Long, result: JSONObject, deadline: Long) {
        if (!current(token) || !resumed) return
        val nativePassed = result.optString("error").isEmpty() && result.optBoolean("pixelsVerified") && result.optLong("completedFrames") == 1L && result.optLong("presentedFrames") == 1L && result.optInt("validationErrors", -1) == 0
        val updated = frameUpdates > before
        if (nativePassed && !updated && SystemClock.uptimeMillis() < deadline) {
            main.postDelayed({ observePattern(token, texture, variant, before, result, deadline) }, 50); return
        }
        if (result.optString("error").isEmpty() && (!nativePassed || !updated)) result.put("error", "Pattern did not produce a verified frame and surface update")
        result.put("variant", variant).put("name", NativeScenes.patterns[variant]).put("surfaceUpdated", updated).put("passed", nativePassed && updated)
            .put("appVersion", packageManager.getPackageInfo(packageName, 0).versionName).put("capturedAtEpochMs", System.currentTimeMillis()).put("batchRunId", runId)
        results.put(result); persistScene(JSONObject(result.toString()), runId)
        // Keep each successfully presented pattern visible before advancing.
        main.postDelayed({
            if (current(token) && resumed) {
                if (variant + 1 < NativeScenes.patterns.size) pattern(token, texture, variant + 1) else checks(token)
            }
        }, if (nativePassed && updated) 1500 else 100)
    }
    private fun checks(token: Int) {
        val id = runId
        val batch = JSONObject().put("scope", "original-guest-visible-patterns").put("runId", id).put("status", "complete")
            .put("startedAtEpochMs", startedAt).put("capturedAtEpochMs", System.currentTimeMillis()).put("totalPatterns", NativeScenes.patterns.size)
            .put("patterns", JSONArray(results.toString()))
        val passed = (0 until results.length()).count { results.getJSONObject(it).optBoolean("passed") }
        batch.put("passedPatterns", passed).put("allPassed", passed == NativeScenes.patterns.size && results.length() == NativeScenes.patterns.size)
        worker.post {
            closeNative(id)
            if (!current(token)) return@post
            try {
                val root = Diagnostics.run(applicationContext, { !current(token) }) { label -> main.post { if (current(token)) status.text = "Running checks\n$label" } }
                root.put("patternBatch", batch).put("allPassed", root.optBoolean("allPassed") && batch.optBoolean("allPassed"))
                main.post { if (current(token) && resumed) { readyReport = root; beginExport(token, root) } }
            } catch (_: CancellationException) { }
            catch (error: Exception) {
                main.post { if (current(token)) { running = false; userCancelled = true; status.text = "Test stopped: ${error.message}"; action.text = "Run again" } }
            }
        }
    }
    private fun beginExport(token: Int, root: JSONObject) {
        if (!current(token) || publishing || !resumed) return
        publishing = true; running = false; status.text = "Saving JSON report…"; action.isEnabled = false
        showSummary(root)
        worker.post {
            try {
                val result = ReportExporter.save(applicationContext, root, root.getJSONObject("patternBatch").getString("runId"))
                main.post { if (!isDestroyed && current(token)) { publishing = false; showSaved(root, result) } }
            } catch (error: Exception) {
                main.post { if (!isDestroyed && current(token)) { publishing = false; userCancelled = true; status.text = "Could not save JSON\n${error.message}"; action.text = "Retry export"; action.isEnabled = true } }
            }
        }
    }
    private fun showSaved(root: JSONObject, result: ReportExporter.Saved) {
        saved = result; readyReport = root; completed = true; running = false
        val count = listOf("engine", "nv", "gpuCore", "shader", "gpu").sumOf { root.optJSONObject(it)?.optJSONArray("tests")?.length() ?: 0 }
        status.text = if (root.optBoolean("allPassed")) "All checks passed\n${NativeScenes.patterns.size} patterns · $count checks" else "Some checks failed\nReport saved with details"
        detail.text = "Saved to Downloads/Emulator\n${result.filename}"; action.text = "Run again"; action.isEnabled = true; share.isEnabled = true
        showSummary(root)
    }
    private fun showSummary(root: JSONObject) {
        val patterns = root.getJSONObject("patternBatch").getJSONArray("patterns")
        resultSummary.text = (0 until patterns.length()).joinToString("\n\n") {
            val pattern = patterns.getJSONObject(it)
            "${if (pattern.optBoolean("passed")) "Passed" else "Failed"} · ${pattern.optString("name") }"
        }
        resultPanel.visibility = View.VISIBLE; view.visibility = View.GONE
    }
    private fun shareSaved() {
        val result = saved ?: return
        startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).apply {
            type = "application/json"; putExtra(Intent.EXTRA_STREAM, result.uri)
            clipData = ClipData.newRawUri("Emulator report", result.uri); addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }, "Share JSON report"))
    }
    private fun interrupt(byUser: Boolean) {
        ++generation; running = false; userCancelled = byUser
        val id = runId; worker.post { closeNative(id) }
        status.text = if (byUser) "Test cancelled. No new report was saved." else "Test paused. Restarts when you return."
        action.text = "Run again"; action.isEnabled = true
    }
    override fun onResume() { super.onResume(); resumed = true; maybeStart() }
    override fun onPause() { resumed = false; if (running && !publishing) interrupt(false); super.onPause() }
    override fun onSaveInstanceState(outState: Bundle) { outState.putString("runId", runId); outState.putBoolean("userCancelled", userCancelled); super.onSaveInstanceState(outState) }
    override fun onSurfaceTextureAvailable(texture: SurfaceTexture, width: Int, height: Int) { maybeStart() }
    override fun onSurfaceTextureSizeChanged(texture: SurfaceTexture, width: Int, height: Int) { if (running && !publishing) { interrupt(false); maybeStart() } }
    override fun onSurfaceTextureUpdated(texture: SurfaceTexture) { ++frameUpdates }
    override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
        if (running && !publishing) interrupt(false)
        val id = runId
        if (!worker.post { closeNative(id); texture.release() }) texture.release()
        return false
    }
    @Deprecated("Activity back compatibility")
    override fun onBackPressed() { if (!publishing) super.onBackPressed() }
    override fun onDestroy() { if (!publishing) ++generation; resumed = false; val id = runId; worker.post { closeNative(id); thread.quitSafely() }; super.onDestroy() }
}
