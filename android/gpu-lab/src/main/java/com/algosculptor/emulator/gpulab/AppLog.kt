package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.graphics.Typeface
import android.os.Build
import android.util.Log
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import java.io.BufferedReader
import java.io.File
import java.io.FileWriter
import java.io.InputStreamReader
import java.io.PrintWriter
import java.io.StringWriter
import java.text.SimpleDateFormat
import java.util.ArrayDeque
import java.util.Date
import java.util.Locale
import java.util.concurrent.Executors

/**
 * Thread-safe logging, crash-capture, and diagnostic export utility.
 * Captures in-memory and persistent logs so users can inspect and share logs when something goes wrong.
 */
object AppLog {
    private const val TAG = "Emulator"
    private const val MAX_MEMORY_LINES = 1000
    private const val MAX_LOG_FILE_BYTES = 512 * 1024L

    private val ringBuffer = ArrayDeque<String>()
    private val diskExecutor = Executors.newSingleThreadExecutor()
    private val timeFormat = SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US)

    private var logDir: File? = null
    private var logFile: File? = null
    private var crashFile: File? = null
    private var initialized = false

    fun init(app: Context) {
        if (initialized) return
        initialized = true

        val dir = File(app.filesDir, "logs").apply { mkdirs() }
        logDir = dir
        logFile = File(dir, "emulator.log")
        crashFile = File(dir, "last-crash.txt")

        // Rotate log file if it exceeds size limit
        try {
            if (logFile?.exists() == true && (logFile?.length() ?: 0L) > MAX_LOG_FILE_BYTES) {
                val old = File(dir, "emulator.log.old")
                old.delete()
                logFile?.renameTo(old)
                logFile = File(dir, "emulator.log")
            }
        } catch (_: Exception) { }

        // Setup global uncaught crash handler
        val previousHandler = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, throwable ->
            try {
                recordCrash(thread, throwable)
            } catch (_: Exception) { }
            previousHandler?.uncaughtException(thread, throwable)
        }

        i("System", "=== Emulator Started ===")
        try {
            val pInfo = app.packageManager.getPackageInfo(app.packageName, 0)
            val vName = pInfo.versionName
            val vCode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) pInfo.longVersionCode else @Suppress("DEPRECATION") pInfo.versionCode.toLong()
            i("System", "App Version: $vName (code $vCode)")
        } catch (_: Exception) { }
        i("System", "Device: ${Build.MANUFACTURER} ${Build.MODEL} (${Build.DEVICE})")
        i("System", "Hardware: ${Build.HARDWARE}, SoC: ${if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) Build.SOC_MODEL else "unknown"}")
        i("System", "Android: ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})")
        i("System", "Supported ABIs: ${Build.SUPPORTED_ABIS.joinToString(", ")}")
    }

    private fun append(level: String, tag: String, message: String, tr: Throwable? = null) {
        val timestamp = timeFormat.format(Date())
        val trace = tr?.let {
            val sw = StringWriter()
            it.printStackTrace(PrintWriter(sw))
            "\n" + sw.toString().trimEnd()
        } ?: ""
        val entry = "[$timestamp] [$level] [$tag] $message$trace"

        synchronized(ringBuffer) {
            if (ringBuffer.size >= MAX_MEMORY_LINES) {
                ringBuffer.pollFirst()
            }
            ringBuffer.addLast(entry)
        }

        diskExecutor.execute {
            try {
                logFile?.let { file ->
                    FileWriter(file, true).use { writer ->
                        writer.append(entry).append('\n')
                    }
                }
            } catch (_: Exception) { }
        }
    }

    fun d(tag: String, message: String) {
        Log.d(tag, message)
        append("DEBUG", tag, message)
    }

    fun i(tag: String, message: String) {
        Log.i(tag, message)
        append("INFO", tag, message)
    }

    fun w(tag: String, message: String, tr: Throwable? = null) {
        if (tr != null) Log.w(tag, message, tr) else Log.w(tag, message)
        append("WARN", tag, message, tr)
    }

    fun e(tag: String, message: String, tr: Throwable? = null) {
        if (tr != null) Log.e(tag, message, tr) else Log.e(tag, message)
        append("ERROR", tag, message, tr)
    }

    private fun recordCrash(thread: Thread, throwable: Throwable) {
        val sw = StringWriter()
        val pw = PrintWriter(sw)
        throwable.printStackTrace(pw)
        val stackTrace = sw.toString()

        val text = buildString {
            append("=== FATAL UNCAUGHT EXCEPTION ===\n")
            append("Timestamp: ${timeFormat.format(Date())}\n")
            append("Thread: ${thread.name} (id: ${thread.id})\n")
            append("Exception: ${throwable.javaClass.name}: ${throwable.message}\n\n")
            append(stackTrace)
        }

        e("CrashHandler", "Uncaught crash on thread ${thread.name}: ${throwable.message}", throwable)

        try {
            crashFile?.let { file ->
                file.writeText(text, Charsets.UTF_8)
            }
        } catch (_: Exception) { }
    }

    private fun collectLogcat(): String = try {
        val process = Runtime.getRuntime().exec(arrayOf("logcat", "-d", "-v", "time", "-t", "200", "-s", "Emulator:V", "AndroidRuntime:E", "*:S"))
        BufferedReader(InputStreamReader(process.inputStream)).use { reader ->
            val sb = java.lang.StringBuilder()
            var line = reader.readLine()
            while (line != null) {
                sb.append(line).append('\n')
                line = reader.readLine()
            }
            sb.toString()
        }
    } catch (_: Exception) {
        "Logcat capture unavailable."
    }

    fun buildDiagnosticLog(context: Context): String = buildString {
        append("=== EMULATOR DIAGNOSTIC & LOG REPORT ===\n")
        append("Captured at: ${timeFormat.format(Date())}\n")
        try {
            val pInfo = context.packageManager.getPackageInfo(context.packageName, 0)
            val vName = pInfo.versionName
            val vCode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) pInfo.longVersionCode else @Suppress("DEPRECATION") pInfo.versionCode.toLong()
            append("App Version: $vName (code $vCode)\n")
        } catch (_: Exception) { }
        append("Device: ${Build.MANUFACTURER} ${Build.MODEL} (${Build.DEVICE})\n")
        append("Hardware: ${Build.HARDWARE}, SoC: ${if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) Build.SOC_MODEL else "unknown"}\n")
        append("Android OS: ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})\n")
        append("Supported ABIs: ${Build.SUPPORTED_ABIS.joinToString(", ")}\n")
        val rt = Runtime.getRuntime()
        append("JVM Memory: free=${rt.freeMemory() / 1024} KiB, total=${rt.totalMemory() / 1024} KiB, max=${rt.maxMemory() / 1024} KiB\n\n")

        // Crash history if present
        try {
            if (crashFile?.exists() == true) {
                val crashText = crashFile?.readText(Charsets.UTF_8) ?: ""
                if (crashText.isNotEmpty()) {
                    append("--- LAST CRASH REPORT ---\n")
                    append(crashText.trimEnd())
                    append("\n\n")
                }
            }
        } catch (_: Exception) { }

        // In-memory event logs
        append("--- RECENT EVENT LOGS ---\n")
        val memoryLines = synchronized(ringBuffer) { ringBuffer.toList() }
        if (memoryLines.isEmpty()) {
            append("(No recorded event logs)\n")
        } else {
            memoryLines.forEach { append(it).append('\n') }
        }
        append("\n")

        // Native / System logcat
        append("--- SYSTEM LOGCAT (recent) ---\n")
        append(collectLogcat().trimEnd())
        append("\n=== END OF REPORT ===\n")
    }

    fun shareLogs(activity: Activity) {
        val report = buildDiagnosticLog(activity)
        val intent = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_SUBJECT, "Emulator Logs - ${Build.MODEL}")
            putExtra(Intent.EXTRA_TEXT, report)
        }
        activity.startActivity(Intent.createChooser(intent, "Share emulator logs"))
    }

    fun showLogDialog(activity: Activity) {
        val report = buildDiagnosticLog(activity)
        val ui = AppUi(activity)
        val dp = { v: Int -> (v * activity.resources.displayMetrics.density).toInt() }

        val root = LinearLayout(activity).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(16), dp(16), dp(16))
        }

        val textView = TextView(activity).apply {
            text = report
            textSize = 12f
            typeface = Typeface.MONOSPACE
            setTextColor(ui.ink)
            setTextIsSelectable(true)
        }

        val scroll = ScrollView(activity).apply {
            addView(textView)
            layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(340))
        }
        root.addView(scroll)

        AlertDialog.Builder(activity)
            .setTitle("Emulator Runtime Logs")
            .setView(root)
            .setPositiveButton("Share logs") { _, _ ->
                shareLogs(activity)
            }
            .setNeutralButton("Copy to clipboard") { _, _ ->
                val cm = activity.getSystemService(ClipboardManager::class.java)
                cm.setPrimaryClip(ClipData.newPlainText("Emulator logs", report))
                Toast.makeText(activity, "Logs copied to clipboard", Toast.LENGTH_SHORT).show()
            }
            .setNegativeButton("Close", null)
            .show()
    }
}
