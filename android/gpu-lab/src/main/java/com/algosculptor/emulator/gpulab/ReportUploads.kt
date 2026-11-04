package com.algosculptor.emulator.gpulab

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.AtomicFile
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.atomic.AtomicInteger

internal data class ReportUploadStatus(val message: String, val busy: Boolean = false, val pending: Int = 0, val lastUrl: String = "", val lastUploadedAt: Long = 0, val accessVerifiedAt: Long = 0)
internal class ReportUploads(private val app: Context, private val network: ReportUploadNetwork = GithubReportNetwork(), private val vault: UpdateVault = UpdateVault(app)) {
    val prefs = app.getSharedPreferences("report-uploads", Context.MODE_PRIVATE)
    private val worker = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())
    private val observers = mutableSetOf<(ReportUploadStatus) -> Unit>()
    private val active = AtomicReference<UpdateTransfer?>()
    private val epoch = AtomicInteger()
    private val choiceLock = Any()
    private val folder = File(app.filesDir, "report-uploads").apply { mkdirs() }
    private val state = AtomicFile(File(folder, "state.json"))
    private data class Pending(val sha: String, var issue: Long = 0, var attempts: Int = 0, var retryAt: Long = 0, var serverWait: Long = 0) {
        fun json() = JSONObject().put("sha", sha).put("issue", issue).put("attempts", attempts).put("retryAt", retryAt).put("serverWait", serverWait)
    }
    private val pending = mutableListOf<Pending>()
    private val uploaded = linkedMapOf<String, String>()
    @Volatile var status = ReportUploadStatus("Loading report uploads…"); private set
    val enabled: Boolean get() = prefs.getBoolean("automatic", false)
    val connected: Boolean get() = vault.exists()
    init { worker.execute { try { restore() } catch (e: Exception) { fail(ReportUploadFailure("The report queue could not be opened. Local JSON exports remain available.", blocked = true)) } } }
    fun observe(observer: (ReportUploadStatus) -> Unit) { observers.add(observer); observer(status) }
    fun unobserve(observer: (ReportUploadStatus) -> Unit) { observers.remove(observer) }
    private fun publish(message: String, busy: Boolean = false) {
        val value = ReportUploadStatus(message, busy, pending.size, prefs.getString("lastUrl", "") ?: "", prefs.getLong("lastUploadedAt", 0), prefs.getLong("accessVerifiedAt", 0))
        main.post { status = value; observers.toList().forEach { it(value) } }
    }
    private fun file(sha: String) = File(folder, "$sha.json")
    private fun persist() {
        val root = JSONObject().put("pending", JSONArray().apply { pending.forEach { put(it.json()) } })
            .put("uploaded", JSONObject().apply { uploaded.forEach { (sha, url) -> put(sha, url) } })
            .put("lastUrl", prefs.getString("lastUrl", "")).put("lastUploadedAt", prefs.getLong("lastUploadedAt", 0))
        val output = state.startWrite()
        try { output.write(root.toString().toByteArray()); state.finishWrite(output) } catch (e: Exception) { state.failWrite(output); throw e }
        prefs.edit().putInt("pending", pending.size).putLong("nextAttempt", pending.minOfOrNull { it.retryAt } ?: 0).apply()
    }
    private fun restore() {
        try {
            val root = state.openRead().use { require(it.channel.size() <= 128 * 1024); JSONObject(it.readBytes().toString(Charsets.UTF_8)) }
            val entries = root.getJSONArray("pending"); require(entries.length() <= ReportUploadPolicy.MAX_PENDING)
            for (i in 0 until entries.length()) {
                val value = entries.getJSONObject(i); val sha = value.getString("sha"); require(UpdatePolicy.validDigest(sha))
                pending.add(Pending(sha, value.optLong("issue").coerceAtLeast(0), value.optInt("attempts").coerceIn(0, 100), value.optLong("retryAt").coerceAtLeast(0), value.optLong("serverWait").coerceAtLeast(0)))
            }
            val done = root.getJSONObject("uploaded"); require(done.length() <= 64)
            done.keys().forEach { sha -> require(UpdatePolicy.validDigest(sha)); uploaded[sha] = done.getString(sha) }
            val url = root.optString("lastUrl")
            if (url.matches(Regex("${Regex.escape(ReportUploadPolicy.WEB)}/issues/[1-9][0-9]*/artifacts/[1-9][0-9]*")))
                prefs.edit().putString("lastUrl", url).putLong("lastUploadedAt", root.optLong("lastUploadedAt")).apply()
        } catch (_: Exception) { pending.clear(); uploaded.clear() }
        // Recover a report written before a process stopped while publishing its
        // queue index. Already acknowledged reports are never enqueued again.
        folder.listFiles()?.filter { it.name.matches(Regex("[0-9a-f]{64}\\.json")) }?.forEach { saved ->
            val sha = saved.name.removeSuffix(".json")
            if (uploaded.containsKey(sha)) saved.delete()
            else if (pending.none { it.sha == sha } && pending.size < ReportUploadPolicy.MAX_PENDING) pending.add(Pending(sha))
        }
        persist(); publish(prefs.getString("message", if (enabled) "Completed reports will upload to private GitHub." else "Automatic report uploads are off.") ?: "Report uploads ready.")
        main.post { ReportUploadSchedule.sync(app) }
    }
    private fun enqueue(bytes: ByteArray): Boolean {
        val report = ReportUploadPolicy.parse(bytes)
        if (uploaded.containsKey(report.sha) || pending.any { it.sha == report.sha }) return false
        if (pending.size >= ReportUploadPolicy.MAX_PENDING) throw ReportUploadFailure("The upload queue is full. Reports are still saved in Downloads; retry after pending uploads finish.")
        val output = AtomicFile(file(report.sha)); val stream = output.startWrite()
        try { stream.write(bytes); output.finishWrite(stream) } catch (e: Exception) { output.failWrite(stream); throw e }
        pending.add(Pending(report.sha)); persist(); return true
    }
    fun enqueueAutomatic(bytes: ByteArray) {
        if (!enabled) return
        val ticket = epoch.get()
        worker.execute {
            if (!enabled || ticket != epoch.get()) return@execute
            try { enqueue(bytes); publish("Report queued for private GitHub upload.") }
            catch (e: Exception) { fail(e) }
            main.post { ReportUploadSchedule.sync(app) }
        }
    }
    fun uploadLatest() = execute("Preparing the latest report…") { transfer ->
        val text = ReportStore.read(app, "last-report.json") ?: throw ReportUploadFailure("Run the diagnostic tests first.")
        enqueue(text.toByteArray(Charsets.UTF_8)); uploadNext(transfer, manual = true)
    }
    fun setAutomatic(value: Boolean) {
        if (!value) {
            synchronized(choiceLock) { epoch.incrementAndGet(); prefs.edit().putBoolean("automatic", false).apply(); active.get()?.cancel() }
            worker.execute { publish("Automatic report uploads are off. Saved reports remain on this device."); main.post { ReportUploadSchedule.sync(app) } }
        } else {
            val ticket = epoch.get()
            execute("Verifying private GitHub report access…") { transfer ->
            network.verifyAccess(token(), transfer); transfer.check()
            synchronized(choiceLock) {
                if (ticket != epoch.get()) throw java.util.concurrent.CancellationException()
                transfer.check()
                prefs.edit().putBoolean("automatic", true).putBoolean("blocked", false).putLong("accessVerifiedAt", System.currentTimeMillis()).apply()
            }
            try { ReportStore.read(app, "last-report.json")?.let { enqueue(it.toByteArray(Charsets.UTF_8)) } } catch (_: IllegalArgumentException) { }
            publish("Automatic report uploads enabled. GitHub write access verified.")
            uploadNext(transfer)
            }
        }
    }
    fun testAccess() = execute("Testing GitHub report write access…") { transfer ->
        network.verifyAccess(token(), transfer); transfer.check()
        prefs.edit().putBoolean("blocked", false).putLong("accessVerifiedAt", System.currentTimeMillis()).apply()
        publish("GitHub report upload access verified.")
    }
    private fun token(): String {
        if (!connected) throw ReportUploadFailure("Connect GitHub in App updates, then give the token Issues: read and write for emulator.", blocked = true)
        return try { vault.read() } catch (_: Exception) { throw ReportUploadFailure("Saved GitHub access could not be read. Reconnect in App updates.", blocked = true) }
    }
    fun runPending(finished: () -> Unit): Boolean {
        if (!enabled || prefs.getBoolean("blocked", false) || !connected || prefs.getInt("pending", 0) == 0) return false
        execute("Uploading queued report…", finished) { uploadNext(it) }; return true
    }
    private fun execute(message: String, finished: () -> Unit = {}, action: (UpdateTransfer) -> Unit) {
        val ticket = epoch.get()
        worker.execute {
            if (ticket != epoch.get()) { main.post(finished); return@execute }
            val transfer = UpdateTransfer(); if (!active.compareAndSet(null, transfer)) { main.post(finished); return@execute }
            publish(message, true)
            try {
                if (prefs.getLong("serverWaitUntil", 0) > System.currentTimeMillis()) publish("GitHub asked uploads to wait. Saved reports remain queued.")
                else action(transfer)
            }
            catch (e: Exception) { if (transfer.cancelled.get()) publish("Upload stopped. The report remains queued.") else fail(e) }
            finally { active.compareAndSet(transfer, null); main.post { ReportUploadSchedule.sync(app); finished() } }
        }
    }
    private fun uploadNext(transfer: UpdateTransfer, manual: Boolean = false) {
        val now = System.currentTimeMillis()
        val item = pending.firstOrNull { it.serverWait <= now && (manual || it.retryAt <= now) }
        if (item == null) { publish(if (pending.isEmpty()) "No reports waiting for upload." else "Reports are queued. GitHub uploads will retry later."); return }
        if (!manual && !enabled) { publish("Automatic report uploads are off."); return }
        try {
            val saved = file(item.sha); require(saved.length() in 1..ReportUploadPolicy.MAX_BYTES.toLong())
            val report = ReportUploadPolicy.parse(saved.readBytes()); require(report.sha == item.sha) { "Saved report checksum changed." }
            publish("Uploading report ${report.runId}…", true)
            val receipt = network.upload(token(), report, item.issue, transfer) { issue -> item.issue = issue; persist() }
            transfer.check()
            uploaded[item.sha] = receipt.url; while (uploaded.size > 64) uploaded.remove(uploaded.keys.first())
            prefs.edit().putString("lastUrl", receipt.url).putLong("lastUploadedAt", System.currentTimeMillis()).putLong("accessVerifiedAt", System.currentTimeMillis()).putBoolean("blocked", false).remove("message").apply()
            pending.remove(item); persist(); saved.delete()
            publish("Report uploaded and verified on private GitHub.")
        } catch (e: Exception) {
            if (!transfer.cancelled.get()) {
                item.attempts = (item.attempts + 1).coerceAtMost(100)
                item.serverWait = (e as? ReportUploadFailure)?.retryAt ?: 0
                if (item.serverWait > 0) prefs.edit().putLong("serverWaitUntil", maxOf(item.serverWait, prefs.getLong("serverWaitUntil", 0))).apply()
                item.retryAt = maxOf(item.serverWait, now + (60_000L shl item.attempts.coerceAtMost(8)).coerceAtMost(6 * 60 * 60 * 1000L))
                persist()
                if (e is IllegalArgumentException || e is org.json.JSONException) throw ReportUploadFailure("A queued report could not be read. Clear pending uploads and run the tests again.", blocked = true)
            }
            throw e
        }
    }
    private fun fail(error: Exception) {
        val message = (error as? ReportUploadFailure)?.message ?: "Report upload could not finish. The local report is safe; retry when connected."
        if (error is ReportUploadFailure && error.blocked) prefs.edit().putBoolean("blocked", true).apply()
        if (error is ReportUploadFailure && error.retryAt > 0) prefs.edit().putLong("serverWaitUntil", maxOf(error.retryAt, prefs.getLong("serverWaitUntil", 0))).apply()
        prefs.edit().putString("message", message).apply(); publish(message)
    }
    fun cancelPending() {
        epoch.incrementAndGet()
        active.get()?.cancel()
        worker.execute { pending.forEach { file(it.sha).delete() }; pending.clear(); persist(); prefs.edit().putBoolean("blocked", false).remove("message").apply(); publish("Pending uploads cancelled. Downloads and completed GitHub reports were kept."); main.post { ReportUploadSchedule.sync(app) } }
    }
    fun stop() { epoch.incrementAndGet(); active.get()?.cancel() }
    fun disconnected() {
        epoch.incrementAndGet()
        active.get()?.cancel()
        prefs.edit().putBoolean("blocked", true).remove("accessVerifiedAt").apply()
        worker.execute { publish("GitHub disconnected. Saved reports remain queued; reconnect and test report upload access."); main.post { ReportUploadSchedule.sync(app) } }
    }
}
