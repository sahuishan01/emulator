package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.res.ColorStateList
import android.content.res.Configuration
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Bundle
import android.view.HapticFeedbackConstants
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import org.json.JSONObject
import java.io.File
import java.text.NumberFormat
import java.util.Locale
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.sin

/**
 * Full emulator GUI for running Switch homebrew NROs and retail NSP packages.
 * Features a 16:9 Switch display viewport, live telemetry HUD, on-screen controls,
 * pause/resume lifecycle handling, and collapsible console diagnostics.
 */
class ProgramActivity : Activity() {
    companion object {
        private const val REQ_PROD_KEYS = 201
    }

    private val worker = Executors.newSingleThreadExecutor()
    private val isPaused = AtomicBoolean(false)
    private val isStopped = AtomicBoolean(false)
    private val pauseLock = Object()

    private lateinit var ui: AppUi
    private lateinit var statusBadge: TextView
    private lateinit var titleText: TextView
    private lateinit var subtitleText: TextView
    private lateinit var viewportView: GameViewportView
    private lateinit var pauseButton: Button
    private lateinit var stopButton: Button
    private lateinit var restartButton: Button
    private lateinit var controlsButton: Button
    private lateinit var logsButton: Button
    private lateinit var controllerLayout: LinearLayout
    private lateinit var logSectionLayout: LinearLayout
    private lateinit var output: TextView
    private lateinit var installKeysButton: Button

    private val store get() = (application as EmulatorApplication).library
    private var activeFile: File? = null
    private var activeId: String = ""
    private var activeMode: String = "static"
    private var activeTitle: String = ""

    private var currentInstructions: Long = 0
    private var currentPc: Long = 0
    private var currentCore: Int = 0
    private var currentMips: Double = 0.0
    private var currentStateStr: String = "loading"
    private var lastErrorStr: String = ""
    private var sessionFinished: Boolean = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        ui = AppUi(this)

        val landscape = resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE
        val rootLayout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(ui.background)
        }

        // --- Top Bar ---
        val topBar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(ui.dp(16), ui.dp(12), ui.dp(16), ui.dp(8))
        }

        val backButton = Button(this).apply {
            text = "❮ Library"
            textSize = 14f
            isAllCaps = false
            setTextColor(ui.accent)
            backgroundTintList = ColorStateList.valueOf(Color.rgb(228, 235, 227))
            setOnClickListener { finish() }
        }
        topBar.addView(backButton)

        val titleContainer = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(ui.dp(12), 0, ui.dp(12), 0)
        }
        titleText = TextView(this).apply {
            text = "Loading program…"
            textSize = 18f
            typeface = Typeface.create("sans-serif-medium", Typeface.BOLD)
            setTextColor(ui.ink)
            isSingleLine = true
        }
        subtitleText = TextView(this).apply {
            text = "Initializing runtime…"
            textSize = 12f
            setTextColor(ui.muted)
            isSingleLine = true
        }
        titleContainer.addView(titleText)
        titleContainer.addView(subtitleText)
        topBar.addView(titleContainer, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))

        statusBadge = TextView(this).apply {
            text = "READY"
            textSize = 12f
            typeface = Typeface.MONOSPACE
            setPadding(ui.dp(10), ui.dp(4), ui.dp(10), ui.dp(4))
            setTextColor(Color.WHITE)
            background = GradientDrawable().apply {
                setColor(Color.rgb(44, 92, 66))
                cornerRadius = ui.dp(12).toFloat()
            }
        }
        topBar.addView(statusBadge)
        rootLayout.addView(topBar)

        // --- Viewport / Game Screen ---
        val viewportContainer = FrameLayout(this).apply {
            setPadding(ui.dp(16), ui.dp(4), ui.dp(16), ui.dp(4))
        }
        viewportView = GameViewportView(this).apply {
            onResumeRequested = { resumeSession() }
        }
        viewportContainer.addView(viewportView, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
        rootLayout.addView(viewportContainer)

        // --- In-Game Action Bar ---
        val actionBar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(ui.dp(16), ui.dp(8), ui.dp(16), ui.dp(8))
        }

        pauseButton = Button(this).apply {
            text = "⏸ Pause"
            textSize = 13f
            isAllCaps = false
            setTextColor(Color.WHITE)
            backgroundTintList = ColorStateList.valueOf(ui.accent)
            setOnClickListener { togglePause() }
        }
        actionBar.addView(pauseButton, LinearLayout.LayoutParams(0, ui.dp(44), 1.2f))

        ui.space(actionBar, 8)

        stopButton = Button(this).apply {
            text = "⏹ Stop"
            textSize = 13f
            isAllCaps = false
            setTextColor(Color.rgb(180, 40, 40))
            backgroundTintList = ColorStateList.valueOf(Color.rgb(248, 224, 224))
            setOnClickListener { stopSession() }
        }
        actionBar.addView(stopButton, LinearLayout.LayoutParams(0, ui.dp(44), 1f))

        ui.space(actionBar, 8)

        restartButton = Button(this).apply {
            text = "🔄 Restart"
            textSize = 13f
            isAllCaps = false
            setTextColor(ui.ink)
            backgroundTintList = ColorStateList.valueOf(Color.rgb(228, 235, 227))
            setOnClickListener { restartSession() }
        }
        actionBar.addView(restartButton, LinearLayout.LayoutParams(0, ui.dp(44), 1f))

        ui.space(actionBar, 8)

        controlsButton = Button(this).apply {
            text = "🎮 Controls"
            textSize = 13f
            isAllCaps = false
            setTextColor(ui.ink)
            backgroundTintList = ColorStateList.valueOf(Color.rgb(228, 235, 227))
            setOnClickListener {
                val isVisible = controllerLayout.visibility == View.VISIBLE
                controllerLayout.visibility = if (isVisible) View.GONE else View.VISIBLE
                controlsButton.backgroundTintList = ColorStateList.valueOf(
                    if (!isVisible) Color.rgb(208, 225, 210) else Color.rgb(228, 235, 227)
                )
            }
        }
        actionBar.addView(controlsButton, LinearLayout.LayoutParams(0, ui.dp(44), 1.1f))

        ui.space(actionBar, 8)

        logsButton = Button(this).apply {
            text = "📋 Logs"
            textSize = 13f
            isAllCaps = false
            setTextColor(ui.muted)
            backgroundTintList = ColorStateList.valueOf(Color.rgb(228, 235, 227))
            setOnClickListener {
                val isVisible = logSectionLayout.visibility == View.VISIBLE
                logSectionLayout.visibility = if (isVisible) View.GONE else View.VISIBLE
                logsButton.setTextColor(if (!isVisible) ui.accent else ui.muted)
            }
        }
        actionBar.addView(logsButton, LinearLayout.LayoutParams(0, ui.dp(44), 0.9f))
        rootLayout.addView(actionBar)

        // --- Scrollable Lower Area: Virtual Controller + Logs ---
        val scrollContent = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(ui.dp(16), 0, ui.dp(16), ui.dp(16))
        }

        // --- Virtual Controller Overlay ---
        controllerLayout = buildControllerLayout()
        scrollContent.addView(controllerLayout)

        // --- Collapsible Logs Section ---
        logSectionLayout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
            setPadding(0, ui.dp(12), 0, 0)
        }

        installKeysButton = ui.button("Install prod.keys", true) {
            startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE); type = "*/*"
            }, REQ_PROD_KEYS)
        }.apply { visibility = View.GONE }
        logSectionLayout.addView(installKeysButton)
        ui.space(logSectionLayout, 8)

        output = TextView(this).apply {
            text = "Waiting for guest output…"
            textSize = 12f
            typeface = Typeface.MONOSPACE
            setTextIsSelectable(true)
            setTextColor(ui.ink)
            setBackgroundColor(Color.rgb(238, 240, 235))
            setPadding(ui.dp(12), ui.dp(12), ui.dp(12), ui.dp(12))
        }
        logSectionLayout.addView(output)
        ui.space(logSectionLayout, 8)

        val logActions = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
        }
        val copyButton = Button(this).apply {
            text = "Copy output"
            textSize = 13f
            isAllCaps = false
            setTextColor(ui.accent)
            backgroundTintList = ColorStateList.valueOf(Color.rgb(228, 235, 227))
            setOnClickListener {
                getSystemService(ClipboardManager::class.java).setPrimaryClip(
                    ClipData.newPlainText("Homebrew output", output.text)
                )
            }
        }
        logActions.addView(copyButton, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        ui.space(logActions, 8)

        val shareButton = Button(this).apply {
            text = "Share logs"
            textSize = 13f
            isAllCaps = false
            setTextColor(ui.accent)
            backgroundTintList = ColorStateList.valueOf(Color.rgb(228, 235, 227))
            setOnClickListener { AppLog.showLogDialog(this@ProgramActivity) }
        }
        logActions.addView(shareButton, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
        logSectionLayout.addView(logActions)

        scrollContent.addView(logSectionLayout)

        val scrollView = ScrollView(this).apply {
            isFillViewport = true
            addView(scrollContent)
        }
        rootLayout.addView(scrollView, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))

        ui.install(rootLayout)

        // Resolve entry and start execution
        val id = intent.getStringExtra("entryId") ?: ""
        activeId = id
        store.find(id) { entry ->
            if (isDestroyed || isFinishing || isStopped.get()) return@find
            val title = entry?.title ?: "Program unavailable"
            activeTitle = title
            titleText.text = title
            val file = entry?.let { store.file(it) }
            if (entry == null || file == null || (entry.mode != "static" && entry.mode != "package")) {
                subtitleText.text = "The imported file is unavailable."
                pauseButton.isEnabled = false
                stopButton.isEnabled = false
                restartButton.isEnabled = false
                updateStatusBadge("UNAVAILABLE", Color.rgb(180, 50, 50))
                AppLog.w("ProgramActivity", "File unavailable or unsupported mode: entry=$entry")
                return@find
            }
            activeFile = file
            activeMode = entry.mode
            launchSession(file, id, entry.title, entry.mode)
        }
    }

    private fun buildControllerLayout(): LinearLayout {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(0, ui.dp(8), 0, ui.dp(8))
        }

        // Shoulder row: ZL, L, -, +, R, ZR
        val shoulderRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(0, 0, 0, ui.dp(8))
        }
        shoulderRow.addView(createControllerButton("ZL", ui.dp(44), 1f))
        ui.space(shoulderRow, 4)
        shoulderRow.addView(createControllerButton("L", ui.dp(44), 1f))
        ui.space(shoulderRow, 12)
        shoulderRow.addView(createControllerButton("－", ui.dp(36), 0.8f))
        ui.space(shoulderRow, 8)
        shoulderRow.addView(createControllerButton("＋", ui.dp(36), 0.8f))
        ui.space(shoulderRow, 12)
        shoulderRow.addView(createControllerButton("R", ui.dp(44), 1f))
        ui.space(shoulderRow, 4)
        shoulderRow.addView(createControllerButton("ZR", ui.dp(44), 1f))
        root.addView(shoulderRow)

        // Main Pad: D-Pad on left, Action diamond on right
        val mainRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
        }

        // Left D-Pad container
        val dpadCol = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = android.view.Gravity.CENTER
        }
        dpadCol.addView(createControllerButton("▲", ui.dp(44), 0f, widthDp = 52))
        val dpadMid = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = android.view.Gravity.CENTER
        }
        dpadMid.addView(createControllerButton("◀", ui.dp(44), 0f, widthDp = 52))
        ui.space(dpadMid, 8)
        dpadMid.addView(createControllerButton("▶", ui.dp(44), 0f, widthDp = 52))
        dpadCol.addView(dpadMid)
        dpadCol.addView(createControllerButton("▼", ui.dp(44), 0f, widthDp = 52))
        mainRow.addView(dpadCol, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))

        // Center space / info
        val centerCol = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = android.view.Gravity.CENTER
        }
        centerCol.addView(TextView(this).apply {
            text = "SWITCH\nINPUT"
            textSize = 10f
            typeface = Typeface.MONOSPACE
            setTextColor(ui.muted)
            gravity = android.view.Gravity.CENTER
        })
        mainRow.addView(centerCol, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 0.6f))

        // Right ABXY diamond container (Nintendo Switch layout: X top, Y left, A right, B bottom)
        val abxyCol = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = android.view.Gravity.CENTER
        }
        abxyCol.addView(createControllerButton("X", ui.dp(44), 0f, widthDp = 52, accentColor = Color.rgb(52, 152, 219)))
        val abxyMid = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = android.view.Gravity.CENTER
        }
        abxyMid.addView(createControllerButton("Y", ui.dp(44), 0f, widthDp = 52, accentColor = Color.rgb(46, 204, 113)))
        ui.space(abxyMid, 8)
        abxyMid.addView(createControllerButton("A", ui.dp(44), 0f, widthDp = 52, accentColor = Color.rgb(231, 76, 60)))
        abxyCol.addView(abxyMid)
        abxyCol.addView(createControllerButton("B", ui.dp(44), 0f, widthDp = 52, accentColor = Color.rgb(241, 196, 15)))
        mainRow.addView(abxyCol, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))

        root.addView(mainRow)
        return root
    }

    private fun createControllerButton(
        label: String,
        heightDp: Int,
        weight: Float = 0f,
        widthDp: Int = 0,
        accentColor: Int? = null
    ): Button {
        return Button(this).apply {
            text = label
            textSize = 14f
            typeface = Typeface.DEFAULT_BOLD
            isAllCaps = false
            setTextColor(accentColor ?: ui.ink)
            background = GradientDrawable().apply {
                setColor(Color.rgb(226, 232, 224))
                cornerRadius = ui.dp(8).toFloat()
                setStroke(ui.dp(1), Color.rgb(205, 215, 203))
            }
            layoutParams = if (weight > 0) {
                LinearLayout.LayoutParams(0, heightDp, weight)
            } else {
                LinearLayout.LayoutParams(if (widthDp > 0) ui.dp(widthDp) else ViewGroup.LayoutParams.WRAP_CONTENT, heightDp)
            }
            setOnTouchListener { v, event ->
                when (event.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        v.isPressed = true
                        v.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY)
                        true
                    }
                    MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                        v.isPressed = false
                        true
                    }
                    else -> false
                }
            }
        }
    }

    private fun launchSession(file: File, id: String, title: String, mode: String) {
        isStopped.set(false)
        isPaused.set(false)
        sessionFinished = false
        installKeysButton.visibility = View.GONE
        pauseButton.isEnabled = true
        pauseButton.text = "⏸ Pause"
        stopButton.isEnabled = true
        restartButton.isEnabled = true
        updateStatusBadge("RUNNING", Color.rgb(44, 92, 66))
        subtitleText.text = "Executing $mode program…"
        output.text = ""

        viewportView.programTitle = title
        viewportView.isPaused = false
        viewportView.statusText = "RUNNING"
        viewportView.instructions = 0
        viewportView.pc = 0
        viewportView.core = 0
        viewportView.mips = 0.0
        viewportView.postInvalidate()

        AppLog.i("ProgramActivity", "Starting program session: title='$title', mode=$mode, file=${file.absolutePath}")

        worker.execute {
            if (isStopped.get()) return@execute
            var result = JSONObject()
            try {
                result = JSONObject(String(NativePrograms.start(file.absolutePath), Charsets.UTF_8))
                AppLog.i("ProgramActivity", "NativePrograms.start returned: ok=${result.optBoolean("ok")}, state=${result.optString("state")}")
                if (result.optBoolean("ok")) {
                    store.recordLaunch(id)
                    var lastShow = 0L
                    var lastInstructionCount = 0L
                    var lastRateTime = System.currentTimeMillis()

                    while (!isStopped.get()) {
                        // Background / User Pause wait loop
                        while (isPaused.get() && !isStopped.get()) {
                            synchronized(pauseLock) {
                                try {
                                    pauseLock.wait(200)
                                } catch (_: InterruptedException) {
                                }
                            }
                        }
                        if (isStopped.get()) break

                        val stepBytes = NativePrograms.step(4096)
                        result = JSONObject(String(stepBytes, Charsets.UTF_8))
                        val state = result.optString("state")

                        val now = System.currentTimeMillis()
                        val instructions = result.optLong("instructions")
                        if (now - lastRateTime >= 400) {
                            val deltaInst = instructions - lastInstructionCount
                            val deltaTime = (now - lastRateTime).coerceAtLeast(1)
                            val mips = (deltaInst.toDouble() / deltaTime) / 1000.0
                            lastInstructionCount = instructions
                            lastRateTime = now
                            result.put("mips", mips)
                        }

                        if (now - lastShow > 150 || state != "running") {
                            show(result)
                            lastShow = now
                        }

                        if (state != "running" && state != "waiting") {
                            // Completed, faulted, or hit limit
                            break
                        }

                        if (state == "waiting") {
                            // Yield briefly to let guest scheduler time advance
                            try { Thread.sleep(16) } catch (_: InterruptedException) {}
                        }
                    }
                }
                if (isStopped.get()) result.put("state", "stopped")
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
        val state = value.optString("state")
        currentStateStr = state
        val error = value.optString("error")
        lastErrorStr = error
        val text = value.optString("output")
        currentInstructions = value.optLong("instructions")
        currentPc = value.optLong("pc", 0L)
        currentCore = value.optInt("core", 0)
        currentMips = value.optDouble("mips", currentMips)
        sessionFinished = finished

        val needsKeys = error.contains("prod.keys") || error.contains("decryption keys")

        runOnUiThread {
            if (!isDestroyed && !isFinishing) {
                val label = when (state) {
                    "exited" -> "Exited normally"
                    "stopped" -> "Session stopped"
                    "waiting" -> "Waiting for guest events"
                    "running" -> "Running · ${NumberFormat.getNumberInstance(Locale.US).format(currentInstructions)} inst"
                    "limit" -> "Execution limit reached"
                    else -> if (error.isNotEmpty()) "Execution halted" else "Homebrew ready"
                }

                subtitleText.text = if (error.isEmpty()) label else "$label: $error"

                if (finished) {
                    pauseButton.isEnabled = false
                    stopButton.isEnabled = false
                    val (badgeText, badgeColor) = when (state) {
                        "exited" -> "EXITED" to Color.rgb(44, 92, 66)
                        "stopped" -> "STOPPED" to Color.rgb(100, 110, 105)
                        "waiting" -> "WAITING" to Color.rgb(41, 128, 185)
                        else -> "ERROR" to Color.rgb(192, 57, 43)
                    }
                    updateStatusBadge(badgeText, badgeColor)
                    if (needsKeys) {
                        installKeysButton.visibility = View.VISIBLE
                        logSectionLayout.visibility = View.VISIBLE
                    }
                } else if (isPaused.get()) {
                    updateStatusBadge("PAUSED", Color.rgb(211, 84, 0))
                } else if (state == "waiting") {
                    updateStatusBadge("WAITING", Color.rgb(41, 128, 185))
                } else {
                    updateStatusBadge("RUNNING", Color.rgb(44, 92, 66))
                }

                output.text = if (text.isEmpty() && finished) {
                    "No program output."
                } else if (value.optBoolean("outputTruncated")) {
                    "$text\n… Output truncated."
                } else {
                    text
                }

                viewportView.instructions = currentInstructions
                viewportView.pc = currentPc
                viewportView.core = currentCore
                viewportView.mips = currentMips
                viewportView.isPaused = isPaused.get()
                viewportView.statusText = if (finished) state.uppercase() else if (isPaused.get()) "PAUSED" else state.uppercase()
                viewportView.postInvalidate()
            }
        }
    }

    private fun updateStatusBadge(text: String, color: Int) {
        statusBadge.text = text
        statusBadge.background = GradientDrawable().apply {
            setColor(color)
            cornerRadius = ui.dp(12).toFloat()
        }
    }

    private fun pauseSession() {
        if (!isStopped.get() && !sessionFinished && !isPaused.getAndSet(true)) {
            AppLog.i("ProgramActivity", "Session paused")
            runOnUiThread {
                pauseButton.text = "▶ Resume"
                pauseButton.backgroundTintList = ColorStateList.valueOf(Color.rgb(39, 174, 96))
                updateStatusBadge("PAUSED", Color.rgb(211, 84, 0))
                viewportView.isPaused = true
                viewportView.statusText = "PAUSED"
                viewportView.postInvalidate()
            }
        }
    }

    private fun resumeSession() {
        if (!isStopped.get() && !sessionFinished && isPaused.getAndSet(false)) {
            AppLog.i("ProgramActivity", "Session resumed")
            synchronized(pauseLock) {
                pauseLock.notifyAll()
            }
            runOnUiThread {
                pauseButton.text = "⏸ Pause"
                pauseButton.backgroundTintList = ColorStateList.valueOf(ui.accent)
                updateStatusBadge("RUNNING", Color.rgb(44, 92, 66))
                viewportView.isPaused = false
                viewportView.statusText = "RUNNING"
                viewportView.postInvalidate()
            }
        }
    }

    private fun togglePause() {
        if (isPaused.get()) {
            resumeSession()
        } else {
            pauseSession()
        }
    }

    private fun stopSession() {
        isStopped.set(true)
        isPaused.set(false)
        synchronized(pauseLock) {
            pauseLock.notifyAll()
        }
        pauseButton.isEnabled = false
        stopButton.isEnabled = false
        updateStatusBadge("STOPPED", Color.rgb(100, 110, 105))
        viewportView.isPaused = false
        viewportView.statusText = "STOPPED"
        viewportView.postInvalidate()
    }

    private fun restartSession() {
        stopSession()
        val file = activeFile
        if (file != null) {
            launchSession(file, activeId, activeTitle, activeMode)
        }
    }

    override fun onPause() {
        super.onPause()
        // Gracefully pause session when app is backgrounded; do NOT terminate or lose state!
        pauseSession()
    }

    override fun onResume() {
        super.onResume()
        // Keep paused status until the user explicitly resumes, updating UI to show Resume option
        if (isPaused.get()) {
            pauseButton.text = "▶ Resume"
            pauseButton.backgroundTintList = ColorStateList.valueOf(Color.rgb(39, 174, 96))
            updateStatusBadge("PAUSED", Color.rgb(211, 84, 0))
            viewportView.isPaused = true
            viewportView.statusText = "PAUSED"
            viewportView.postInvalidate()
        }
    }

    override fun onDestroy() {
        stopSession()
        worker.shutdown()
        super.onDestroy()
    }

    @Deprecated("Deprecated in Java")
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

    /**
     * 16:9 Aspect Ratio Display Viewport Canvas.
     * Draws Switch native 720p presentation, live CPU telemetry HUD,
     * instruction counter, activity waveforms, and interactive Pause overlay.
     */
    class GameViewportView(context: Context) : View(context) {
        var onResumeRequested: (() -> Unit)? = null

        var instructions: Long = 0
        var pc: Long = 0
        var core: Int = 0
        var mips: Double = 0.0
        var statusText: String = "READY"
        var isPaused: Boolean = false
        var programTitle: String = ""

        private val backgroundPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.rgb(10, 14, 20)
        }
        private val bezelPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeWidth = 3f
            color = Color.rgb(31, 41, 55)
        }
        private val scanlinePaint = Paint().apply {
            color = Color.argb(18, 0, 0, 0)
            strokeWidth = 2f
        }
        private val gridPaint = Paint().apply {
            color = Color.argb(12, 52, 152, 219)
            strokeWidth = 1f
        }
        private val textPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.rgb(230, 235, 240)
            typeface = Typeface.MONOSPACE
        }
        private val accentPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.rgb(46, 204, 113)
            typeface = Typeface.MONOSPACE
        }
        private val waveformPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.argb(160, 46, 204, 113)
            strokeWidth = 3f
            style = Paint.Style.STROKE
        }
        private val overlayPaint = Paint().apply {
            color = Color.argb(210, 5, 8, 14)
        }
        private val resumeBtnPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.rgb(39, 174, 96)
        }

        private var animPhase: Float = 0f
        private val resumeRect = RectF()

        init {
            contentDescription = "Switch Emulator Display Viewport"
        }

        override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
            val width = MeasureSpec.getSize(widthMeasureSpec)
            val heightFromWidth = (width * 9) / 16
            val heightMode = MeasureSpec.getMode(heightMeasureSpec)
            val heightSize = MeasureSpec.getSize(heightMeasureSpec)
            val finalHeight = if (heightMode == MeasureSpec.EXACTLY) {
                heightSize
            } else if (heightMode == MeasureSpec.AT_MOST) {
                minOf(heightFromWidth, heightSize)
            } else {
                heightFromWidth
            }
            setMeasuredDimension(width, finalHeight)
        }

        override fun onDraw(canvas: Canvas) {
            super.onDraw(canvas)
            val w = width.toFloat()
            val h = height.toFloat()
            val cornerRadius = 24f

            // 1. Clip rounded viewport and draw background
            val rect = RectF(0f, 0f, w, h)
            val path = Path().apply { addRoundRect(rect, cornerRadius, cornerRadius, Path.Direction.CW) }
            canvas.clipPath(path)
            canvas.drawRect(rect, backgroundPaint)

            // 2. Subtle grid pattern
            val step = 40f
            var x = step
            while (x < w) {
                canvas.drawLine(x, 0f, x, h, gridPaint)
                x += step
            }
            var y = step
            while (y < h) {
                canvas.drawLine(0f, y, w, y, gridPaint)
                y += step
            }

            // 3. Scanline effect
            var sY = 0f
            while (sY < h) {
                canvas.drawLine(0f, sY, w, sY, scanlinePaint)
                sY += 4f
            }

            // 4. Top Telemetry Header
            accentPaint.textSize = h * 0.055f
            canvas.drawText("SWITCH 720P · 1280×720 · 60HZ", w * 0.04f, h * 0.12f, accentPaint)

            textPaint.textSize = h * 0.055f
            val coreStr = "CORE $core"
            canvas.drawText(coreStr, w * 0.96f - textPaint.measureText(coreStr), h * 0.12f, textPaint)

            // 5. Active Oscilloscope / CPU Waveform
            if (!isPaused && instructions > 0) {
                animPhase += 0.2f
                val wavePath = Path()
                val waveY = h * 0.48f
                val waveAmp = h * 0.08f
                wavePath.moveTo(w * 0.1f, waveY)
                var wx = w * 0.1f
                while (wx <= w * 0.9f) {
                    val progress = (wx - w * 0.1f) / (w * 0.8f)
                    val wy = waveY + (sin(progress * 16.0 + animPhase) * waveAmp * (1.0 - progress * 0.5)).toFloat()
                    wavePath.lineTo(wx, wy)
                    wx += 12f
                }
                canvas.drawPath(wavePath, waveformPaint)
            }

            // 6. Central Telemetry Readout
            textPaint.textSize = h * 0.11f
            textPaint.typeface = Typeface.DEFAULT_BOLD
            val formattedInst = NumberFormat.getNumberInstance(Locale.US).format(instructions)
            canvas.drawText(formattedInst, w * 0.08f, h * 0.72f, textPaint)

            textPaint.textSize = h * 0.045f
            textPaint.typeface = Typeface.MONOSPACE
            canvas.drawText("INSTRUCTIONS EXECUTED", w * 0.08f, h * 0.80f, accentPaint)

            // Right side stats: PC and MIPS
            val pcStr = String.format("PC: 0x%010X", pc)
            canvas.drawText(pcStr, w * 0.92f - textPaint.measureText(pcStr), h * 0.68f, textPaint)

            val mipsStr = String.format(Locale.US, "SPEED: %.2f MIPS", mips)
            canvas.drawText(mipsStr, w * 0.92f - textPaint.measureText(mipsStr), h * 0.78f, textPaint)

            // 7. Outer Bezel Stroke
            canvas.drawRoundRect(rect, cornerRadius, cornerRadius, bezelPaint)

            // 8. Frosted Pause Overlay with Centered "▶ RESUME" Button
            if (isPaused) {
                canvas.drawRect(rect, overlayPaint)

                textPaint.textSize = h * 0.12f
                textPaint.typeface = Typeface.DEFAULT_BOLD
                val pauseTitle = "⏸  PAUSED"
                val pTitleW = textPaint.measureText(pauseTitle)
                canvas.drawText(pauseTitle, (w - pTitleW) / 2f, h * 0.40f, textPaint)

                // Large Glowing Resume Button
                val btnW = w * 0.44f
                val btnH = h * 0.22f
                val btnL = (w - btnW) / 2f
                val btnT = h * 0.52f
                resumeRect.set(btnL, btnT, btnL + btnW, btnT + btnH)

                canvas.drawRoundRect(resumeRect, 16f, 16f, resumeBtnPaint)

                textPaint.textSize = h * 0.085f
                val resumeText = "▶  RESUME"
                val rTextW = textPaint.measureText(resumeText)
                canvas.drawText(resumeText, btnL + (btnW - rTextW) / 2f, btnT + btnH * 0.64f, textPaint)
            }
        }

        override fun onTouchEvent(event: MotionEvent): Boolean {
            if (isPaused && event.action == MotionEvent.ACTION_UP) {
                // Tapping anywhere on the paused screen or the resume button resumes!
                performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY)
                onResumeRequested?.invoke()
                return true
            }
            return super.onTouchEvent(event)
        }
    }
}
