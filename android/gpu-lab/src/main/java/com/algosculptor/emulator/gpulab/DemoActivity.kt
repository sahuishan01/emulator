package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.graphics.Color
import android.graphics.SurfaceTexture
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.Surface
import android.view.TextureView
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.WindowManager
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.roundToInt

/** Owns one render worker. A destroyed TextureView stays alive until Vulkan releases it. */
class DemoActivity : Activity(), TextureView.SurfaceTextureListener {
    companion object { init { System.loadLibrary("emulator_gpu_lab") } }
    private external fun startDemo(surface: Surface): ByteArray
    private external fun stepDemo(x: Int, reset: Boolean): ByteArray
    private external fun stopDemo(): ByteArray
    private val thread = HandlerThread("HomebrewRenderer")
    private lateinit var worker: Handler
    private lateinit var view: TextureView
    private lateinit var stats: TextView
    @Volatile private var generation = 0
    @Volatile private var paddle = 24
    @Volatile private var paused = false
    private val reset = AtomicBoolean(false)
    private var resumed = false
    private val inputHandler = Handler(Looper.getMainLooper())
    private val heldDirections = mutableMapOf<Button, Int>()
    private val repeatDirection = object : Runnable {
        override fun run() {
            if (!resumed || paused || heldDirections.isEmpty()) return
            moveDirection(heldDirections.values.sum().coerceIn(-1, 1))
            inputHandler.postDelayed(this, 80)
        }
    }

    private fun moveDirection(direction: Int) {
        paddle = (paddle + direction * 4).coerceIn(0, 48)
    }
    private fun clearDirections() {
        inputHandler.removeCallbacks(repeatDirection)
        heldDirections.keys.forEach { it.isPressed = false }
        heldDirections.clear()
    }
    private fun releaseDirection(button: Button) {
        heldDirections.remove(button)
        button.isPressed = false
        if (heldDirections.isEmpty()) inputHandler.removeCallbacks(repeatDirection)
    }
    private fun directionButton(label: String, direction: Int): Button {
        var pointer = -1
        var touchClick = false
        return Button(this).apply {
            text = label
            // Keyboard/accessibility clicks retain the same single-step action.
            setOnClickListener { if (!touchClick) moveDirection(direction) }
            setOnTouchListener { button, event ->
                val control = button as Button
                when (event.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        pointer = event.getPointerId(0)
                        heldDirections[control] = direction
                        control.isPressed = true
                        control.parent.requestDisallowInterceptTouchEvent(true)
                        moveDirection(heldDirections.values.sum().coerceIn(-1, 1))
                        inputHandler.removeCallbacks(repeatDirection)
                        if (resumed && !paused) inputHandler.postDelayed(repeatDirection, 240)
                    }
                    MotionEvent.ACTION_MOVE -> {
                        val index = event.findPointerIndex(pointer)
                        if (index < 0 || event.getX(index) < 0 || event.getX(index) >= control.width ||
                            event.getY(index) < 0 || event.getY(index) >= control.height) {
                            releaseDirection(control)
                            pointer = -1
                        }
                    }
                    MotionEvent.ACTION_POINTER_UP -> {
                        if (event.getPointerId(event.actionIndex) == pointer) {
                            releaseDirection(control)
                            pointer = -1
                        }
                    }
                    MotionEvent.ACTION_UP -> {
                        val clicked = heldDirections.containsKey(control)
                        releaseDirection(control)
                        pointer = -1
                        control.parent.requestDisallowInterceptTouchEvent(false)
                        if (clicked) {
                            touchClick = true
                            try { control.performClick() } finally { touchClick = false }
                        }
                    }
                    MotionEvent.ACTION_CANCEL -> {
                        releaseDirection(control)
                        pointer = -1
                        control.parent.requestDisallowInterceptTouchEvent(false)
                    }
                }
                true
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        thread.start(); worker = Handler(thread.looper)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val pad = (16 * resources.displayMetrics.density).toInt()
        val layout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad, pad, pad)
            setOnApplyWindowInsetsListener { v, insets ->
                val system = insets.getInsets(WindowInsets.Type.systemBars())
                v.setPadding(pad + system.left, pad + system.top, pad + system.right, pad + system.bottom)
                insets
            }
        }
        layout.addView(TextView(this).apply { text = "Catch · Original homebrew"; textSize = 23f })
        layout.addView(TextView(this).apply {
            text = "Drag to move the red paddle. Catch the falling green square."
            textSize = 15f; setPadding(0, pad / 2, 0, pad / 2)
        })
        view = object : TextureView(this) {
            override fun onMeasure(widthSpec: Int, heightSpec: Int) {
                val side = minOf(MeasureSpec.getSize(widthSpec), MeasureSpec.getSize(heightSpec))
                setMeasuredDimension(side, side)
            }
            override fun performClick(): Boolean { super.performClick(); return true }
        }.apply {
            surfaceTextureListener = this@DemoActivity
            contentDescription = "Homebrew game screen"
            isFocusableInTouchMode = true
            isFocusable = true
            setOnTouchListener { v, event ->
                when (event.actionMasked) {
                    MotionEvent.ACTION_DOWN, MotionEvent.ACTION_MOVE -> {
                        if (event.actionMasked == MotionEvent.ACTION_DOWN) clearDirections()
                        v.requestFocus()
                        paddle = (event.x / v.width * 64 - 8).roundToInt().coerceIn(0, 48)
                        true
                    }
                    MotionEvent.ACTION_UP -> { v.performClick(); true }
                    MotionEvent.ACTION_CANCEL -> true
                    else -> false
                }
            }
        }
        layout.addView(view, LinearLayout.LayoutParams(-1, 0, 1f))
        stats = TextView(this).apply { text = "Starting guest…"; textSize = 15f; setPadding(0, pad, 0, pad / 2) }
        layout.addView(stats)
        val controls = LinearLayout(this)
        fun control(label: String, action: () -> Unit) {
            controls.addView(Button(this).apply { text = label; setOnClickListener { action() } }, LinearLayout.LayoutParams(0, -2, 1f))
        }
        controls.addView(directionButton("Left", -1), LinearLayout.LayoutParams(0, -2, 1f))
        controls.addView(directionButton("Right", 1), LinearLayout.LayoutParams(0, -2, 1f))
        val pause = Button(this).apply {
            text = "Pause"
            setOnClickListener { clearDirections(); paused = !paused; text = if (paused) "Resume" else "Pause" }
        }
        controls.addView(pause, LinearLayout.LayoutParams(0, -2, 1f))
        control("Reset") { reset.set(true) }
        layout.addView(controls)
        layout.addView(TextView(this).apply { text = "Runs our own emulated program. Commercial games are not supported."; textSize = 12f; setTextColor(Color.DKGRAY) })
        layout.addView(Button(this).apply { text = "Back to checks"; setOnClickListener { finish() } })
        setContentView(layout)
        view.requestFocus()
        window.insetsController?.setSystemBarsAppearance(
            WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
            WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS,
        )
    }

    private fun start(texture: SurfaceTexture) {
        clearDirections()
        val current = ++generation
        val surface = Surface(texture)
        worker.post {
            if (current != generation) { surface.release(); return@post }
            val initial = try { JSONObject(String(startDemo(surface), Charsets.UTF_8)) }
                catch (e: Exception) { JSONObject().put("running", false).put("error", e.message) }
            finally { surface.release() }
            if (!initial.optBoolean("running")) { show(current, initial); return@post }
            var lastSave = 0L
            var lastShow = 0L
            var lastResult = initial
            var wasPaused = false
            val frames = object : Runnable {
                override fun run() {
                    if (current != generation) return
                    if (paused && !wasPaused) {
                        lastResult.put("paused", true)
                        persist(lastResult)
                        show(current, lastResult)
                        wasPaused = true
                    }
                    if (!paused) {
                        wasPaused = false
                        val result = try { JSONObject(String(stepDemo(paddle, reset.getAndSet(false)), Charsets.UTF_8)) }
                            catch (e: Exception) { JSONObject().put("running", false).put("error", e.message) }
                        result.put("paused", false)
                        lastResult = result
                        if (System.currentTimeMillis() - lastSave > 500 || !result.optBoolean("running")) {
                            persist(result)
                            lastSave = System.currentTimeMillis()
                            if (System.currentTimeMillis() - lastShow > 1600 || !result.optBoolean("running")) {
                                show(current, result)
                                lastShow = System.currentTimeMillis()
                            }
                        }
                        if (!result.optBoolean("running")) { stopDemo(); return }
                    }
                    worker.postDelayed(this, 33)
                }
            }
            worker.post(frames)
        }
    }
    private fun show(current: Int, result: JSONObject) {
        runOnUiThread {
            if (current == generation && !isDestroyed) {
                stats.text = if (result.optString("error").isNotEmpty()) "Rendering stopped: ${result.optString("error")}"
                    else "Score ${result.optInt("score")}   Misses ${result.optInt("misses")}\nFrame ${result.optInt("frame")} · Presented ${result.optLong("presentedFrames")}\n${result.optString("deviceName")}"
            }
        }
    }
    private fun persist(result: JSONObject) {
        result.put("appVersion", packageManager.getPackageInfo(packageName, 0).versionName)
        result.put("capturedAtEpochMs", System.currentTimeMillis())
        try {
            val temporary = filesDir.resolve("last-demo-report.tmp")
            temporary.writeText(result.toString(2))
            check(temporary.renameTo(filesDir.resolve("last-demo-report.json")))
        } catch (_: Exception) { }
    }
    private fun stopNative() {
        val result = JSONObject(String(stopDemo(), Charsets.UTF_8))
        if (result.optLong("completedFrames") > 0 || result.optString("error").isNotEmpty()) persist(result)
    }
    private fun stop() { clearDirections(); ++generation; worker.post { stopNative() } }
    override fun onResume() {
        super.onResume(); resumed = true
        if (view.isAvailable) view.surfaceTexture?.let { start(it) }
    }
    override fun onPause() { resumed = false; stop(); super.onPause() }
    override fun onSurfaceTextureAvailable(texture: SurfaceTexture, width: Int, height: Int) { if (resumed) start(texture) }
    override fun onSurfaceTextureSizeChanged(texture: SurfaceTexture, width: Int, height: Int) { if (resumed) start(texture) }
    override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
        clearDirections()
        ++generation
        // If Activity teardown already stopped and quit the worker, Vulkan is
        // finished and the late TextureView callback can release directly.
        if (!worker.post { stopNative(); texture.release() }) texture.release()
        return false // Renderer retains the old texture until queued work and Vulkan surface end.
    }
    override fun onSurfaceTextureUpdated(texture: SurfaceTexture) { }
    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (event.keyCode == KeyEvent.KEYCODE_DPAD_LEFT || event.keyCode == KeyEvent.KEYCODE_DPAD_RIGHT) {
            if (event.action == KeyEvent.ACTION_DOWN) {
                paddle = (paddle + if (event.keyCode == KeyEvent.KEYCODE_DPAD_LEFT) -4 else 4).coerceIn(0, 48)
            }
            return true
        }
        return super.dispatchKeyEvent(event)
    }
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus) clearDirections()
    }
    override fun onDestroy() {
        clearDirections()
        ++generation
        worker.post { stopNative(); thread.quitSafely() }
        super.onDestroy()
    }
}
