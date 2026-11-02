package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.graphics.SurfaceTexture
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import android.view.TextureView
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONObject

/** One worker owns native Vulkan and retains a destroyed texture until teardown. */
class TextureActivity : Activity(), TextureView.SurfaceTextureListener {
    companion object { init { System.loadLibrary("emulator_gpu_lab") } }
    private external fun startScene(surface: Surface, variant: Int): ByteArray
    private external fun stopScene(): ByteArray
    private val thread = HandlerThread("TextureScene")
    private lateinit var worker: Handler
    private lateinit var view: TextureView
    private lateinit var status: TextView
    @Volatile private var generation = 0
    private var resumed = false
    private var variant = 0
    private val patterns = arrayOf("Texture", "BGRA texture", "Updated texture", "Indexed byte quad", "Indexed short quad", "Indexed triangle")
    private var recorded = false
    private val store get() = (application as EmulatorApplication).library
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState); thread.start(); worker = Handler(thread.looper)
        variant = savedInstanceState?.getInt("variant", 0) ?: 0
        if (AppSettings.keepAwake(this)) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val ui = AppUi(this); val root = ui.column()
        root.addView(ui.text("Texture scene", 28f, bold = true)); ui.space(root, 8)
        root.addView(ui.text("Original homebrew graphics. Choose a pattern.", 15f, ui.muted)); ui.space(root, 16)
        view = TextureView(this).apply { surfaceTextureListener = this@TextureActivity; contentDescription = "Textured guest scene" }
        root.addView(view, LinearLayout.LayoutParams(-1, 0, 1f)); ui.space(root, 12)
        status = ui.text("Opening scene…", 15f, ui.muted); root.addView(status)
        root.addView(ui.button("Next pattern") { variant = (variant + 1) % patterns.size; if (resumed && view.isAvailable) view.surfaceTexture?.let { start(it) } })
        root.addView(ui.button("Back to library") { finish() }); ui.install(root)
    }
    private fun persist(value: JSONObject) {
        if (value.optLong("completedFrames") == 0L && value.optString("error").isEmpty()) return
        value.put("appVersion", packageManager.getPackageInfo(packageName, 0).versionName)
        value.put("capturedAtEpochMs", System.currentTimeMillis())
        try { ReportStore.write(this, "last-texture-report.json", value.toString(2)) } catch (_: Exception) { }
    }
    private fun closeNative() { persist(JSONObject(String(stopScene(), Charsets.UTF_8))) }
    private fun start(texture: SurfaceTexture) {
        val current = ++generation; val pattern = variant; status.text = "Rendering pattern ${pattern + 1}…"
        worker.post {
            if (current != generation) return@post
            closeNative()
            if (current != generation) return@post
            val surface = Surface(texture)
            val result = try { JSONObject(String(startScene(surface, pattern), Charsets.UTF_8)) }
                catch (error: Exception) { JSONObject().put("error", error.message ?: "Could not render this scene.") }
                finally { surface.release() }
            persist(result)
            runOnUiThread { if (current == generation && resumed && !isDestroyed) {
                val error = result.optString("error")
                status.text = if (error.isNotEmpty()) "Rendering stopped: $error" else "${patterns[pattern]} · ${pattern + 1} of ${patterns.size} · Ready" + if (AppSettings.showStats(this)) "\n${result.optString("deviceName")} · Presented ${result.optLong("presentedFrames")}" else ""
                if (error.isEmpty() && result.optLong("presentedFrames") > 0 && !recorded) { recorded = true; store.recordLaunch("builtin:textures"); store.recordResult("builtin:textures", "Texture scene verified") }
            } }
        }
    }
    override fun onResume() { super.onResume(); resumed = true; if (view.isAvailable) view.surfaceTexture?.let { start(it) } }
    override fun onPause() { resumed = false; ++generation; worker.post { closeNative() }; super.onPause() }
    override fun onSaveInstanceState(outState: Bundle) { outState.putInt("variant", variant); super.onSaveInstanceState(outState) }
    override fun onSurfaceTextureAvailable(texture: SurfaceTexture, width: Int, height: Int) { if (resumed) start(texture) }
    override fun onSurfaceTextureSizeChanged(texture: SurfaceTexture, width: Int, height: Int) { if (resumed) start(texture) }
    override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean { ++generation; if (!worker.post { closeNative(); texture.release() }) texture.release(); return false }
    override fun onSurfaceTextureUpdated(texture: SurfaceTexture) { }
    override fun onDestroy() { ++generation; worker.post { closeNative(); thread.quitSafely() }; super.onDestroy() }
}
