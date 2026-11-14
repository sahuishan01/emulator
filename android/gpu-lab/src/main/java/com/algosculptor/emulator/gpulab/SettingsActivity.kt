package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.os.Bundle
import android.widget.Switch
import android.widget.ScrollView

class SettingsActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val ui = AppUi(this); val root = ui.column(); val prefs = AppSettings.prefs(this)
        root.addView(ui.button("Back to library") { finish() }); ui.space(root, 20)
        root.addView(ui.text("Settings", 30f, bold = true)); ui.space(root, 12)
        root.addView(ui.button("App updates", true) { startActivity(Intent(this, UpdatesActivity::class.java)) })
        root.addView(ui.text("Check for updates and test GitHub access.", 14f, ui.muted)); ui.space(root, 24)
        root.addView(ui.button("GitHub reports") { startActivity(Intent(this, ReportsActivity::class.java)) }); ui.space(root, 16)
        root.addView(ui.label("PLAYER")); ui.space(root, 12)
        fun toggle(label: String, key: String, default: Boolean) {
            root.addView(Switch(this).apply { text = label; textSize = 16f; setTextColor(ui.ink); minHeight = ui.dp(56)
                isChecked = prefs.getBoolean(key, default); setOnCheckedChangeListener { _, checked -> prefs.edit().putBoolean(key, checked).apply() } })
        }
        toggle("Keep screen awake", "keepAwake", true)
        toggle("Show frame and GPU details", "showStats", false)
        val speedNames = arrayOf("Slow", "Normal", "Fast"); val speeds = intArrayOf(120, 80, 50)
        val speed = ui.button("Held control speed: ${speedNames[speeds.indexOf(prefs.getInt("repeatMs", 80)).coerceAtLeast(0)]}") { }
        speed.setOnClickListener {
            AlertDialog.Builder(this).setTitle("Held control speed").setSingleChoiceItems(speedNames, speeds.indexOf(prefs.getInt("repeatMs", 80))) { dialog, which ->
                prefs.edit().putInt("repeatMs", speeds[which]).apply(); speed.text = "Held control speed: ${speedNames[which]}"; dialog.dismiss()
            }.setNegativeButton("Cancel", null).show()
        }
        root.addView(speed); ui.space(root, 24); root.addView(ui.label("STORAGE")); ui.space(root, 10)
        val storage = ui.text("Loading storage…", 15f, ui.muted); root.addView(storage)
        (application as EmulatorApplication).library.list { entries ->
            if (!isDestroyed) storage.text = "Imported files: ${entries.sumOf { it.bytes } / 1024} KiB / 256 MiB\n${entries.count { !it.included }} of 128 import slots used"
        }
        ui.space(root, 24); root.addView(ui.label("SUPPORT")); ui.space(root, 10)
        root.addView(ui.button("Diagnostics") { startActivity(Intent(this, MainActivity::class.java)) })
        root.addView(ui.text("Run the engine and GPU checks, then export a report when something goes wrong.", 14f, ui.muted))
        ui.space(root, 12)
        root.addView(ui.button("View & share logs", true) { AppLog.showLogDialog(this) })
        root.addView(ui.text("View runtime logs, crash traces, and share diagnostic logs if something goes wrong.", 14f, ui.muted))
        ui.space(root, 20)
        root.addView(ui.text("Version ${packageManager.getPackageInfo(packageName, 0).versionName}\nHomebrew support in development", 13f, ui.muted))
        ui.install(ScrollView(this).apply { addView(root) })
    }
}
