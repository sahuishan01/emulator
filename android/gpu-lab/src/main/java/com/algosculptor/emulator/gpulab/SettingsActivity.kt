package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView

class SettingsActivity : Activity() {
    companion object {
        private const val REQ_PROD_KEYS = 101
        private const val REQ_TITLE_KEYS = 102
    }

    private lateinit var keyStatus: TextView
    private lateinit var clearKeysBtn: Button

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
        root.addView(speed); ui.space(root, 24)

        root.addView(ui.label("DECRYPTION KEYS")); ui.space(root, 10)
        keyStatus = ui.text("Checking decryption keys…", 15f, ui.muted)
        root.addView(keyStatus); ui.space(root, 12)
        root.addView(ui.button("Install prod.keys", true) {
            startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE); type = "*/*"
            }, REQ_PROD_KEYS)
        })
        root.addView(ui.text("Required for decrypting Nintendo Switch NSP packages.", 13f, ui.muted)); ui.space(root, 12)
        root.addView(ui.button("Install title.keys") {
            startActivityForResult(Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE); type = "*/*"
            }, REQ_TITLE_KEYS)
        })
        root.addView(ui.text("Optional external title keys for rights-protected packages.", 13f, ui.muted)); ui.space(root, 12)
        clearKeysBtn = ui.button("Clear installed keys") {
            AlertDialog.Builder(this).setTitle("Clear Decryption Keys")
                .setMessage("Are you sure you want to remove installed prod.keys and title.keys?")
                .setPositiveButton("Clear") { _, _ ->
                    KeyManager.clearKeys(this)
                    updateKeyStatus()
                }.setNegativeButton("Cancel", null).show()
        }
        root.addView(clearKeysBtn); ui.space(root, 24)

        root.addView(ui.label("STORAGE")); ui.space(root, 10)
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
        updateKeyStatus()
    }

    override fun onResume() {
        super.onResume()
        updateKeyStatus()
    }

    private fun updateKeyStatus() {
        val info = KeyManager.getInfo(this)
        val prodText = if (info.hasProdKeys) "prod.keys: Installed (${info.prodKeyCount} keys)" else "prod.keys: Not installed"
        val titleText = if (info.hasTitleKeys) "title.keys: Installed (${info.titleKeyCount} keys)" else "title.keys: Not installed"
        keyStatus.text = "$prodText\n$titleText"
        clearKeysBtn.isEnabled = info.hasProdKeys || info.hasTitleKeys
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (resultCode != RESULT_OK || data?.data == null) return
        val isTitleKeys = (requestCode == REQ_TITLE_KEYS)
        if (requestCode == REQ_PROD_KEYS || isTitleKeys) {
            val (ok, message) = KeyManager.installFromUri(this, data.data!!, isTitleKeys)
            AlertDialog.Builder(this)
                .setTitle(if (ok) "Keys Installed" else "Installation Failed")
                .setMessage(message)
                .setPositiveButton("OK", null)
                .show()
            updateKeyStatus()
        }
    }
}
