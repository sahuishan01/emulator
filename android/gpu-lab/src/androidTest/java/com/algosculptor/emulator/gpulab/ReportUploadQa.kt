package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.app.Instrumentation
import android.app.job.JobScheduler
import android.content.Intent
import android.graphics.Bitmap
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.widget.Switch
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.concurrent.atomic.AtomicInteger

/** Isolated emulator-only QA; fake network and credential never ship in main DEX. */
internal class ReportUploadQa(private val test: Instrumentation) {
    private val app get() = test.targetContext.applicationContext as EmulatorApplication
    private val checks = mutableListOf<String>()
    private fun until(message: String, condition: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 30_000
        while (System.currentTimeMillis() < deadline) { if (condition()) { test.waitForIdleSync(); return }; Thread.sleep(25) }
        error(message)
    }
    private fun check(name: String, action: () -> Unit) { action(); checks.add(name); test.sendStatus(0, Bundle().apply { putString("stream", "Verified $name\n") }) }
    private fun root(id: String) = JSONObject().put("schemaVersion", 4).put("scope", "original-engine-gpu-core-and-host-vulkan")
        .put("appVersion", app.packageManager.getPackageInfo(app.packageName, 0).versionName).put("capturedAtEpochMs", 123L)
        .put("patternBatch", JSONObject().put("status", "complete").put("runId", id).put("totalPatterns", 1).put("passedPatterns", 0)
            .put("patterns", JSONArray().put(JSONObject().put("variant", 42).put("passed", false))))
        .apply { for (key in listOf("engine", "nv", "gpuCore", "shader", "gpu")) put(key, JSONObject().put("tests", JSONArray().put(JSONObject().put("name", "QA Ω").put("passed", key != "gpu")))) }
    private class Fake : ReportUploadNetwork {
        val probes = AtomicInteger(); val uploads = AtomicInteger()
        @Volatile var failure: ReportUploadFailure? = null
        @Volatile var last: UploadReport? = null
        override fun verifyAccess(token: String, transfer: UpdateTransfer) { transfer.check(); probes.incrementAndGet(); failure?.let { throw it } }
        override fun upload(token: String, report: UploadReport, issue: Long, transfer: UpdateTransfer, saveIssue: (Long) -> Unit): ReportReceipt {
            transfer.check(); uploads.incrementAndGet(); last = report; saveIssue(77); failure?.let { throw it }; return ReportReceipt(77, 1)
        }
    }
    private fun views(v: View): List<View> = listOf(v) + if (v is ViewGroup) (0 until v.childCount).flatMap { views(v.getChildAt(it)) } else emptyList()
    fun run() {
        val vault = UpdateVault(app)
        val prefs = app.getSharedPreferences("report-uploads", 0)
        val backup = ReportStore.read(app, "last-report.json")
        val folder = File(app.filesDir, "report-uploads")
        var saved: ReportExporter.Saved? = null
        var manager: ReportUploads? = null
        var activity: ReportsActivity? = null
        val field = app.javaClass.getDeclaredField("reportUploads\$delegate").apply { isAccessible = true }
        val original = field.get(app)
        var passed = false; var failure = ""
        try {
            require(android.os.Build.FINGERPRINT.contains("generic") || android.os.Build.MODEL.contains("sdk")) { "QA runner is restricted to the owned emulator" }
            require(!vault.exists()) { "QA must not replace an existing credential" }
            prefs.edit().clear().commit(); folder.deleteRecursively()
            vault.write("QA_ONLY_REPORT_TOKEN_NOT_A_CREDENTIAL")
            app.getSharedPreferences("updates", 0).edit().putBoolean("automatic", false).commit()
            val net = Fake(); val reports = ReportUploads(app, net, vault); manager = reports
            field.set(app, lazyOf(reports))
            until("No initial report status") { !reports.status.message.startsWith("Loading") }
            check("Automatic uploads default off and do not enqueue") {
                require(!reports.enabled); reports.enqueueAutomatic(root("off").toString().toByteArray()); Thread.sleep(150)
                require(reports.status.pending == 0 && net.uploads.get() == 0)
            }
            check("Write access denial leaves automatic uploads off") {
                net.failure = ReportUploadFailure("Controlled Issues permission denial", blocked = true)
                reports.setAutomatic(true); until("No permission failure") { reports.status.message.contains("permission denial") }
                require(!reports.enabled && net.probes.get() == 1)
            }
            check("Actual access test records verified time without enabling uploads") {
                net.failure = null; reports.testAccess(); until("No successful access probe") { reports.status.message == "GitHub report upload access verified." }
                require(reports.status.accessVerifiedAt > 0 && !reports.enabled && net.uploads.get() == 0)
            }
            check("Manual upload works with automation off and deduplicates exact bytes") {
                val text = root("manual").toString(2) + "\n"; ReportStore.write(app, "last-report.json", text)
                reports.uploadLatest(); until("Manual report not acknowledged") { reports.status.lastUrl.endsWith("/77/artifacts/1") && !reports.status.busy }
                require(net.last!!.bytes.contentEquals(text.toByteArray()) && net.last!!.passed == 4)
                val before = net.uploads.get(); reports.uploadLatest(); until("Duplicate not completed") { reports.status.message == "No reports waiting for upload." }
                require(net.uploads.get() == before && ReportStore.read(app, "last-report.json") == text)
            }
            check("Enable verifies access and resumes completed report delivery") {
                reports.setAutomatic(true); until("Not enabled") { reports.enabled && !reports.status.busy && reports.status.message == "No reports waiting for upload." }
                require(net.probes.get() == 3)
            }
            check("Real Downloads export publishes exact immutable JSON and queues automatically") {
                saved = ReportExporter.save(app, root("auto-export"), "qa-report-auto-export")
                until("Export not queued") { reports.status.pending == 1 }
                require(ReportExporter.published(app, saved!!.uri))
                val raw = app.contentResolver.openInputStream(saved!!.uri)!!.use { it.readBytes() }
                require(raw.contentEquals(ReportStore.read(app, "last-report.json")!!.toByteArray()))
                require(reports.runPending {})
                until("Export not uploaded") { reports.status.pending == 0 && !reports.status.busy }
                require(net.last!!.bytes.contentEquals(raw))
                require(app.contentResolver.openInputStream(saved!!.uri)!!.use { it.readBytes().contentEquals(raw) })
            }
            check("Offline failure retains original bytes and persisted issue for retry") {
                net.failure = ReportUploadFailure("Controlled offline outage")
                ReportStore.write(app, "last-report.json", root("offline").toString())
                reports.uploadLatest(); until("No offline error") { reports.status.message == "Controlled offline outage" }
                require(reports.status.pending == 1)
                val state = JSONObject(File(folder, "state.json").readText()); require(state.getJSONArray("pending").getJSONObject(0).getLong("issue") == 77L)
                val sha = net.last!!.sha; require(File(folder, "$sha.json").readBytes().contentEquals(net.last!!.bytes))
            }
            check("Pending job requires network battery storage and survives reboot") {
                test.runOnMainSync { ReportUploadSchedule.sync(app) }
                val job = app.getSystemService(JobScheduler::class.java).getPendingJob(ReportUploadSchedule.ID)!!
                require(job.isPersisted && job.isRequireBatteryNotLow && job.isRequireStorageNotLow && !job.isPeriodic && job.minLatencyMillis > 0)
            }
            check("Queue and successful receipt restore after manager recreation") {
                val restored = ReportUploads(app, net, vault); manager = restored; field.set(app, lazyOf(restored))
                until("Queue did not restore") { restored.status.pending == 1 }
                require(restored.status.lastUrl.endsWith("/77/artifacts/1") && restored.status.lastUploadedAt > 0)
                net.failure = null; restored.uploadLatest(); until("Restored retry failed") { restored.status.pending == 0 && !restored.status.busy }
            }
            val current = manager!!
            check("Rate limit blocks immediate manual retries and access probes") {
                net.failure = ReportUploadFailure("Controlled rate limit", retryAt = System.currentTimeMillis()+120_000)
                ReportStore.write(app, "last-report.json", root("limited").toString()); current.uploadLatest()
                until("No rate limit") { current.status.message == "Controlled rate limit" }
                val uploads = net.uploads.get(); val probes = net.probes.get()
                current.uploadLatest(); until("No wait status") { current.status.message.contains("asked uploads to wait") }
                current.testAccess(); Thread.sleep(200); require(net.uploads.get() == uploads && net.probes.get() == probes)
                prefs.edit().remove("serverWaitUntil").commit()
            }
            check("Revoked access blocks background work until explicitly verified") {
                current.cancelPending(); until("Cancel pending failed") { current.status.pending == 0 }
                net.failure = ReportUploadFailure("Controlled revoked access", blocked = true)
                ReportStore.write(app, "last-report.json", root("revoked").toString()); current.uploadLatest()
                until("No revoked state") { current.status.message == "Controlled revoked access" }
                require(!current.runPending {})
                test.runOnMainSync { ReportUploadSchedule.sync(app) }
                require(app.getSystemService(JobScheduler::class.java).getPendingJob(ReportUploadSchedule.ID) == null)
                net.failure = null; current.testAccess(); until("Not verified again") { current.status.message == "GitHub report upload access verified." }
            }
            check("Cancellation removes queued copies while keeping Downloads and cached report") {
                val cached = ReportStore.read(app, "last-report.json"); current.cancelPending(); until("Not cancelled") { current.status.pending == 0 }
                require(ReportStore.read(app, "last-report.json") == cached && ReportExporter.published(app, saved!!.uri))
                require(folder.listFiles()!!.none { it.name.matches(Regex("[0-9a-f]{64}\\.json")) })
            }
            check("Report settings expose manual actions and recorded upload access history") {
                activity = test.startActivitySync(Intent(app, ReportsActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as ReportsActivity
                test.runOnMainSync {
                    val all = views(activity!!.window.decorView); val text = all.filterIsInstance<TextView>().map { it.text.toString() }
                    for (label in listOf("Test report upload access", "Upload latest JSON", "Open uploaded report", "Manage GitHub connection")) require(label in text)
                    require(text.any { it.contains("Upload access last verified:") && !it.contains("verified: Never") })
                    require(text.any { it.contains("Issues: read and write") })
                    require(all.filterIsInstance<Switch>().single().isChecked)
                }
                File(app.filesDir, "reports-qa.png").outputStream().use { require(test.uiAutomation.takeScreenshot().compress(Bitmap.CompressFormat.PNG,100,it)) }
            }
            check("Disabling automation cancels scheduled work") {
                current.setAutomatic(false); until("Not disabled") { !current.enabled && !current.status.busy }
                test.runOnMainSync { ReportUploadSchedule.sync(app) }; require(app.getSystemService(JobScheduler::class.java).getPendingJob(ReportUploadSchedule.ID) == null)
            }
            check("Disconnect cancels uploads and clears report access verification") {
                vault.clear(); current.disconnected(); until("No disconnected report status") { current.status.message.startsWith("GitHub disconnected.") }
                require(current.status.accessVerifiedAt == 0L && !current.connected && !current.runPending {})
            }
            check("Immediate opt-out invalidates an already queued enable request") {
                vault.write("QA_ONLY_REPORT_TOKEN_NOT_A_CREDENTIAL")
                test.runOnMainSync { current.setAutomatic(true); current.setAutomatic(false) }
                until("Opt-out not completed") { current.status.message.startsWith("Automatic report uploads are off.") }
                require(!current.enabled)
            }
            passed = true
        } catch (e: Exception) { failure = "${e.javaClass.simpleName}: ${e.message}" }
        finally {
            manager?.stop(); test.runOnMainSync { activity?.finish(); field.set(app, original) }
            app.getSystemService(JobScheduler::class.java).cancel(ReportUploadSchedule.ID)
            vault.clear(); prefs.edit().clear().commit(); folder.deleteRecursively()
            saved?.let { app.contentResolver.delete(it.uri, null, null) }
            if (backup != null) ReportStore.write(app,"last-report.json",backup) else File(app.filesDir,"last-report.json").delete()
        }
        val result = JSONObject().put("passed",passed).put("checks",JSONArray(checks)).put("failure",failure)
        File(app.filesDir,"report-upload-qa-result.json").writeText(result.toString(2))
        test.finish(if(passed)Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply { putString("stream", result.toString()+"\n") })
    }
}
