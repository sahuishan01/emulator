package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import android.text.InputType
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.ProgressBar
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView

class UpdatesActivity : Activity() {
    private val updates get() = (application as EmulatorApplication).updates
    private lateinit var ui: AppUi
    private lateinit var status: TextView
    private lateinit var notes: TextView
    private lateinit var progress: ProgressBar
    private lateinit var check: Button
    private lateinit var download: Button
    private lateinit var install: Button
    private lateinit var cancel: Button
    private lateinit var connect: Button
    private lateinit var disconnect: Button
    private var installRequested = false
    private var resumed = false
    private val observer: (UpdateStatus) -> Unit = { render(it) }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState); ui = AppUi(this); installRequested = savedInstanceState?.getBoolean("installRequested") ?: false
        val root = ui.column(); root.addView(ui.button("Back to settings") { finish() }); ui.space(root, 20)
        root.addView(ui.text("App updates", 30f, bold = true)); ui.space(root, 6)
        root.addView(ui.text("Installed ${updates.installedVersion}", 15f, ui.muted)); ui.space(root, 20)
        status = ui.text("Preparing updates…", 17f); root.addView(status)
        progress = ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal).apply { max = 100 }; root.addView(progress)
        check = ui.button("Check for updates") { updates.checkForUpdates() }; root.addView(check)
        download = ui.button("Download update", true) { updates.download() }; root.addView(download)
        install = ui.button("Install update", true) { beginInstall() }; root.addView(install)
        cancel = ui.button("Cancel download") { updates.cancel() }; root.addView(cancel)
        notes = ui.text("", 14f, ui.muted); root.addView(notes); ui.space(root, 20)
        root.addView(ui.label("AUTOMATIC UPDATES"))
        fun toggle(label: String, key: String) {
            root.addView(Switch(this).apply {
                text = label; textSize = 16f; minHeight = ui.dp(56); isChecked = updates.prefs.getBoolean(key, true)
                setOnCheckedChangeListener { _, enabled -> updates.prefs.edit().putBoolean(key, enabled).apply(); UpdateSchedule.sync(this@UpdatesActivity) }
            })
        }
        toggle("Check automatically", "automatic"); toggle("Download on Wi-Fi", "downloadWifi")
        root.addView(ui.text("Checks run when you open the app and periodically when Android allows. Downloads use unmetered networks automatically. Android asks before installing.", 14f, ui.muted)); ui.space(root, 24)
        root.addView(ui.label("PRIVATE RELEASE ACCESS")); ui.space(root, 8)
        root.addView(ui.text("Updates come from your private GitHub repository. Connect once with a token limited to sahuishan01/emulator and Contents: read-only. Access stays encrypted on this device.", 14f, ui.muted))
        connect = ui.button("Connect GitHub") { connectDialog() }; root.addView(connect)
        root.addView(ui.button("Create access token") { startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://github.com/settings/personal-access-tokens/new"))) })
        disconnect = ui.button("Disconnect GitHub") { AlertDialog.Builder(this).setMessage("Remove private update access and downloaded APKs from this device?").setNegativeButton("Cancel", null).setPositiveButton("Disconnect") { _, _ -> updates.disconnect() }.show() }; root.addView(disconnect)
        ui.install(ScrollView(this).apply { addView(root) })
    }
    private fun render(value: UpdateStatus) {
        status.text = value.message; notes.text = value.release?.notes.orEmpty(); progress.visibility = if (value.phase == UpdatePhase.DOWNLOADING) View.VISIBLE else View.GONE; progress.progress = value.percent
        check.visibility = if (updates.connected) View.VISIBLE else View.GONE; check.isEnabled = !value.busy
        download.visibility = if (value.release != null && value.phase in setOf(UpdatePhase.AVAILABLE, UpdatePhase.ERROR)) View.VISIBLE else View.GONE
        install.visibility = if (value.phase == UpdatePhase.READY) View.VISIBLE else View.GONE
        cancel.visibility = if (value.busy) View.VISIBLE else View.GONE; cancel.text = if (value.phase == UpdatePhase.CHECKING) "Cancel check" else "Cancel download"
        connect.isEnabled = !value.busy; connect.text = if (updates.connected) "Replace GitHub access" else "Connect GitHub"; disconnect.visibility = if (updates.connected) View.VISIBLE else View.GONE
    }
    private fun connectDialog() {
        val input = EditText(this).apply { hint = "GitHub access token"; isSingleLine = true; inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD; importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO; isSaveEnabled = false }
        val dialog = AlertDialog.Builder(this).setTitle("Connect private updates").setMessage("Use a fine-grained token for sahuishan01/emulator with Contents read-only. The app never includes or shares this token in reports.").setView(input).setNegativeButton("Cancel") { _, _ -> input.text.clear() }.setPositiveButton("Connect", null).create()
        dialog.setOnShowListener { dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener {
            val token = input.text.toString().trim()
            if (token.length !in 20..512 || token.any { it.code !in 33..126 }) { input.error = "Enter a valid access token"; return@setOnClickListener }
            input.text.clear(); updates.connect(token); dialog.dismiss()
        } }; dialog.setOnDismissListener { input.text.clear() }; dialog.show()
    }
    private fun beginInstall() {
        if (!packageManager.canRequestPackageInstalls()) { installRequested = true; startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:$packageName"))); return }
        updates.verifiedForInstall { file, release, error ->
            if (isDestroyed || isFinishing || !resumed || file == null || release == null) { if (error != null && !isDestroyed) status.text = error; return@verifiedForInstall }
            val uri = Uri.parse("content://$packageName.updates/apk/${release.sha}.apk")
            try { startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(uri, "application/vnd.android.package-archive").addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION).apply { clipData = ClipData.newRawUri("Emulator update", uri) }) }
            catch (_: Exception) { status.text = "Android could not open the installer. Try again." }
        }
    }
    override fun onStart() { super.onStart(); updates.observe(observer) }
    override fun onResume() { super.onResume(); resumed = true; if (installRequested) { installRequested = false; if (packageManager.canRequestPackageInstalls()) beginInstall() } }
    override fun onPause() { resumed = false; super.onPause() }
    override fun onStop() { updates.unobserve(observer); super.onStop() }
    override fun onSaveInstanceState(outState: Bundle) { outState.putBoolean("installRequested", installRequested); super.onSaveInstanceState(outState) }
}
