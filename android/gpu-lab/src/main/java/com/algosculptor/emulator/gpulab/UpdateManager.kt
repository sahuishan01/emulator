package com.algosculptor.emulator.gpulab

import android.content.Context
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.AtomicFile
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.security.MessageDigest
import java.util.concurrent.CancellationException
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicReference
import java.util.zip.ZipFile

internal enum class UpdatePhase { DISCONNECTED, IDLE, CHECKING, AVAILABLE, DOWNLOADING, READY, ERROR }
internal data class UpdateStatus(val phase: UpdatePhase, val message: String, val release: UpdateRelease? = null, val percent: Int = 0) {
    val busy: Boolean get() = phase == UpdatePhase.CHECKING || phase == UpdatePhase.DOWNLOADING
}
internal class UpdateManager(private val app: Context, private val network: UpdateNetwork = GithubUpdateNetwork(), private val vault: UpdateVault = UpdateVault(app)) {
    companion object { const val INTERVAL = 6L * 60 * 60 * 1000 }
    val prefs = app.getSharedPreferences("updates", Context.MODE_PRIVATE)
    private val worker = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())
    private val observers = mutableSetOf<(UpdateStatus) -> Unit>()
    private val active = AtomicReference<UpdateTransfer?>()
    @Volatile var status = UpdateStatus(UpdatePhase.IDLE, "Preparing updates…"); private set
    private val directory = File(app.filesDir, "updates").apply { mkdirs() }
    private val ready = File(directory, "ready.apk")
    private val partial = File(directory, "download.part")
    private val metadata = AtomicFile(File(directory, "ready.json"))
    init { worker.execute { restore() } }
    val connected: Boolean get() = vault.exists()
    val installedVersion: String get() = app.packageManager.getPackageInfo(app.packageName, 0).versionName ?: ""
    fun observe(observer: (UpdateStatus) -> Unit) { observers.add(observer); observer(status) }
    fun unobserve(observer: (UpdateStatus) -> Unit) { observers.remove(observer) }
    private fun publish(value: UpdateStatus) { main.post { status = value; observers.toList().forEach { it(value) } } }
    private fun errorMessage(e: Exception): String = if (e is UpdateFailure) e.message ?: "Update failed." else "Update could not finish. Check your connection and try again."
    private fun readMetadata(): UpdateRelease? = try { metadata.openRead().use { kotlin.check(it.channel.size() <= 8192); UpdateRelease.restore(String(it.readBytes(), Charsets.UTF_8)) } } catch (_: Exception) { null }
    private fun clearReady() { ready.delete(); metadata.delete() }
    private fun restore() {
        partial.delete()
        val release = readMetadata()
        if (release != null && UpdatePolicy.version(release.version)!! > (UpdatePolicy.version(installedVersion) ?: return)) {
            try { verifyApk(ready, release); publish(UpdateStatus(UpdatePhase.READY, "Version ${release.version} is ready to install.", release)); return } catch (_: Exception) { clearReady() }
        } else clearReady()
        publish(if (connected) UpdateStatus(UpdatePhase.IDLE, "Ready to check for updates.") else UpdateStatus(UpdatePhase.DISCONNECTED, "Connect GitHub to receive private updates."))
    }
    fun connect(token: String) {
        if (active.get() != null) return
        val transfer = UpdateTransfer(); if (!active.compareAndSet(null, transfer)) return
        publish(UpdateStatus(UpdatePhase.CHECKING, "Connecting to GitHub…"))
        worker.execute {
            try {
                require(token.length in 20..512 && token.all { it.code in 33..126 })
                val release = UpdateRelease.latest(network.releases(token, transfer), installedVersion); transfer.check(); vault.write(token)
                prefs.edit().putLong("lastAttempt", System.currentTimeMillis()).apply()
                publish(if (release == null) UpdateStatus(UpdatePhase.IDLE, "You're up to date.") else UpdateStatus(UpdatePhase.AVAILABLE, "Version ${release.version} is available.", release))
                main.post { UpdateSchedule.sync(app) }
            } catch (e: Exception) { publish(UpdateStatus(UpdatePhase.ERROR, if (e is IllegalArgumentException) "Enter a valid GitHub access token." else errorMessage(e))) }
            finally { active.compareAndSet(transfer, null) }
        }
    }
    fun disconnect() {
        active.get()?.cancel()
        worker.execute {
            vault.clear(); clearReady(); partial.delete(); prefs.edit().remove("lastAttempt").apply()
            publish(UpdateStatus(UpdatePhase.DISCONNECTED, "GitHub disconnected. Downloaded updates were removed.")); main.post { UpdateSchedule.sync(app) }
        }
    }
    fun automatic(finished: () -> Unit = {}): Boolean {
        if (!connected || !prefs.getBoolean("automatic", true)) return false
        val last = prefs.getLong("lastAttempt", 0); val elapsed = System.currentTimeMillis() - last
        if (elapsed in 0 until INTERVAL) return false
        return checkForUpdates(true, finished)
    }
    fun checkForUpdates(background: Boolean = false, finished: () -> Unit = {}): Boolean {
        if (!connected) { publish(UpdateStatus(UpdatePhase.DISCONNECTED, "Connect GitHub to receive private updates.")); return false }
        val transfer = UpdateTransfer(background); if (!active.compareAndSet(null, transfer)) return false
        prefs.edit().putLong("lastAttempt", System.currentTimeMillis()).apply(); publish(UpdateStatus(UpdatePhase.CHECKING, "Checking for updates…"))
        worker.execute {
            try {
                val token = vault.read(); val release = UpdateRelease.latest(network.releases(token, transfer), installedVersion); transfer.check()
                if (release == null) publish(UpdateStatus(UpdatePhase.IDLE, "You're up to date."))
                else if (readMetadata() == release && ready.isFile) {
                    verifyApk(ready, release); publish(UpdateStatus(UpdatePhase.READY, "Version ${release.version} is ready to install.", release, 100))
                } else {
                    publish(UpdateStatus(UpdatePhase.AVAILABLE, "Version ${release.version} is available.", release))
                    if (prefs.getBoolean("downloadWifi", true) && !(app.getSystemService(ConnectivityManager::class.java).isActiveNetworkMetered)) downloadNow(release, token, transfer)
                }
            } catch (e: Exception) {
                val cached = readMetadata()
                val preserved = if (cached == null) null else try { verifyApk(ready, cached); UpdateStatus(UpdatePhase.READY, "Version ${cached.version} is ready. " + if (transfer.cancelled.get()) "Check cancelled." else "Could not check for a newer update.", cached, 100) } catch (_: Exception) { null }
                publish(preserved ?: if (transfer.cancelled.get()) UpdateStatus(UpdatePhase.IDLE, "Update cancelled.") else UpdateStatus(UpdatePhase.ERROR, errorMessage(e)))
            }
            finally { partial.delete(); active.compareAndSet(transfer, null); main.post(finished) }
        }; return true
    }
    fun download() {
        val release = status.release ?: return; if (!connected) return
        val transfer = UpdateTransfer(); if (!active.compareAndSet(null, transfer)) return
        publish(UpdateStatus(UpdatePhase.DOWNLOADING, "Downloading ${release.version}…", release))
        worker.execute {
            try { downloadNow(release, vault.read(), transfer) }
            catch (e: Exception) { publish(if (transfer.cancelled.get()) UpdateStatus(UpdatePhase.AVAILABLE, "Download cancelled. You can retry.", release) else UpdateStatus(UpdatePhase.ERROR, errorMessage(e), release)) }
            finally { partial.delete(); active.compareAndSet(transfer, null) }
        }
    }
    private fun downloadNow(release: UpdateRelease, token: String, transfer: UpdateTransfer) {
        publish(UpdateStatus(UpdatePhase.DOWNLOADING, "Downloading ${release.version}…", release)); var last = -1
        FileOutputStream(partial).use { output -> network.asset(release, token, transfer) { input ->
            UpdatePolicy.copyVerified(input, output, release.size, release.sha, { transfer.check(); transfer.cancelled.get() }) { bytes ->
                val p = (bytes * 100 / release.size).toInt(); if (p != last) { last = p; publish(UpdateStatus(UpdatePhase.DOWNLOADING, "Downloading ${release.version}… $p%", release, p)) }
            }
        }; output.fd.sync() }
        transfer.check(); verifyApk(partial, release); transfer.check()
        if (!partial.renameTo(ready)) throw UpdateFailure("Could not save the verified update.")
        val output = metadata.startWrite()
        try { output.write(release.json().toByteArray(Charsets.UTF_8)); metadata.finishWrite(output) }
        catch (e: Exception) { metadata.failWrite(output); clearReady(); throw e }
        publish(UpdateStatus(UpdatePhase.READY, "Version ${release.version} is ready to install.", release, 100))
    }
    fun cancel(backgroundOnly: Boolean = false) { active.get()?.let { if (!backgroundOnly || it.background) it.cancel() } }
    fun verifiedForInstall(complete: (File?, UpdateRelease?, String?) -> Unit) {
        worker.execute {
            val release = readMetadata()
            try { if (release == null) throw UpdateFailure("Download the update again."); verifyApk(ready, release); main.post { complete(ready, release, null) } }
            catch (e: Exception) { clearReady(); val message = errorMessage(e); publish(UpdateStatus(UpdatePhase.ERROR, message)); main.post { complete(null, null, message) } }
        }
    }
    internal fun verifyApk(file: File, release: UpdateRelease) {
        if (!file.isFile || file.length() != release.size) throw UpdateFailure("Update file is missing or incomplete. Download it again.")
        FileInputStream(file).use { UpdatePolicy.copyVerified(it, object : java.io.OutputStream() { override fun write(b: Int) {}
            override fun write(b: ByteArray, off: Int, len: Int) {} }, release.size, release.sha, { false }, {}) }
        val pm = app.packageManager
        val next = pm.getPackageArchiveInfo(file.path, PackageManager.GET_SIGNING_CERTIFICATES) ?: throw UpdateFailure("Downloaded file is not a valid APK.")
        val current = pm.getPackageInfo(app.packageName, PackageManager.GET_SIGNING_CERTIFICATES)
        if (next.packageName != app.packageName || next.versionName != release.version || next.longVersionCode <= current.longVersionCode) throw UpdateFailure("Update package or version does not match this app.")
        val a = next.signingInfo?.apkContentsSigners?.map { it.toCharsString() }?.toSet() ?: emptySet()
        val b = current.signingInfo?.apkContentsSigners?.map { it.toCharsString() }?.toSet() ?: emptySet()
        if (a.isEmpty() || a != b) throw UpdateFailure("Update signing certificate does not match this app.")
        if ((next.applicationInfo?.minSdkVersion ?: Int.MAX_VALUE) > Build.VERSION.SDK_INT) throw UpdateFailure("This update needs a newer Android version.")
        ZipFile(file).use { zip -> if (Build.SUPPORTED_ABIS.none { zip.getEntry("lib/$it/libemulator_gpu_lab.so") != null }) throw UpdateFailure("Update does not support this device's processor.") }
    }
}
