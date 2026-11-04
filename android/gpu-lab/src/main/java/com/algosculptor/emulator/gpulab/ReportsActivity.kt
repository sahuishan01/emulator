package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.view.View
import android.widget.Button
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView
import java.text.DateFormat
import java.util.Date

class ReportsActivity : Activity() {
    private val reports get() = (application as EmulatorApplication).reportUploads
    private lateinit var automatic: Switch
    private lateinit var status: TextView
    private lateinit var history: TextView
    private lateinit var test: Button
    private lateinit var upload: Button
    private lateinit var open: Button
    private var rendering = false
    private val observer: (ReportUploadStatus) -> Unit = { render(it) }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val ui = AppUi(this); val root = ui.column()
        root.addView(ui.button("Back") { finish() }); ui.space(root, 20)
        root.addView(ui.text("GitHub reports", 30f, bold = true)); ui.space(root, 10)
        root.addView(ui.text("Send completed test reports to your private emulator repository so they can be collected automatically.", 16f, ui.muted)); ui.space(root, 16)
        status = ui.text("Loading report uploads…", 17f).apply { accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE }; root.addView(status)
        history = ui.text("", 14f, ui.muted); root.addView(history); ui.space(root, 12)
        automatic = Switch(this).apply {
            text = "Upload completed reports automatically"; textSize = 16f; minHeight = ui.dp(64); setTextColor(ui.ink)
            setOnCheckedChangeListener { _, value -> if (!rendering) reports.setAutomatic(value) }
        }; root.addView(automatic)
        test = ui.button("Test report upload access") { reports.testAccess() }; root.addView(test)
        upload = ui.button("Upload latest JSON", true) { reports.uploadLatest() }; root.addView(upload)
        open = ui.button("Open uploaded report") { val value = reports.status.lastUrl; if (value.isNotEmpty()) startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(value))) }; root.addView(open)
        root.addView(ui.button("Cancel pending uploads") { reports.cancelPending() }); ui.space(root, 20)
        root.addView(ui.label("GITHUB ACCESS")); ui.space(root, 8)
        root.addView(ui.text("Use the token saved in App updates. Give it Issues: read and write on sahuishan01/emulator. Keep Contents: read-only for APK updates.", 15f, ui.muted))
        root.addView(ui.button("Manage GitHub connection") { startActivity(Intent(this, UpdatesActivity::class.java)) }); ui.space(root, 20)
        root.addView(ui.text("Reports include device and driver details, diagnostic results and rendered pixel evidence. They go to private GitHub Issues, outside source history. Games, firmware, keys and GitHub tokens are not included. Local JSON exports remain in Downloads/Emulator.", 14f, ui.muted))
        ui.install(ScrollView(this).apply { addView(root) })
    }
    private fun render(value: ReportUploadStatus) {
        rendering = true; automatic.isChecked = reports.enabled; automatic.isEnabled = !value.busy; rendering = false
        status.text = value.message
        val last = if (value.lastUploadedAt > 0) DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(value.lastUploadedAt)) else "Never"
        val access = if (value.accessVerifiedAt > 0) DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(value.accessVerifiedAt)) else "Never"
        history.text = "Pending reports: ${value.pending}\nUpload access last verified: $access\nLast uploaded: $last"
        test.isEnabled = !value.busy; upload.isEnabled = !value.busy; open.visibility = if (value.lastUrl.isEmpty()) View.GONE else View.VISIBLE
    }
    override fun onStart() { super.onStart(); reports.observe(observer) }
    override fun onStop() { reports.unobserve(observer); super.onStop() }
}
