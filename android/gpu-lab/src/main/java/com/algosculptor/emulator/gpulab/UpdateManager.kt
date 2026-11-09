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
internal enum class UpdateAccessResult { NONE, SAVED, VERIFIED, DENIED, ERROR }
internal data class UpdateAccessStatus(val saved: Boolean, val result: UpdateAccessResult, val lastVerifiedAt: Long)
internal data class UpdateStatus(val phase: UpdatePhase, val message: String, val release: UpdateRelease? = null, val percent: Int = 0) {
    val busy: Boolean get() = phase == UpdatePhase.CHECKING || phase == UpdatePhase.DOWNLOADING
}
internal class UpdateManager(private val app: Context, private val network: UpdateNetwork = GithubUpdateNetwork(), private val vault: UpdateVault = UpdateVault(app), private val downloads: UpdateDownloads = AndroidUpdateDownloads(app)) {
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
    private val pendingFile = AtomicFile(File(directory, "pending.json"))
    private val generation = java.util.concurrent.atomic.AtomicLong()
    private data class Pending(val release: UpdateRelease, val nonce: String, val id: Long, val wifiOnly: Boolean)
    private val polling = java.util.concurrent.atomic.AtomicBoolean()
    private fun readPending(): Pending? = try {
        pendingFile.openRead().use { input ->
            kotlin.check(input.channel.size() <= 12_000)
            val x = org.json.JSONObject(input.readBytes().toString(Charsets.UTF_8))
            val nonce=x.getString("nonce"); require(nonce.matches(Regex("[a-f0-9-]{36}")))
            val id=x.getLong("id"); require(id>=0)
            Pending(UpdateRelease.restore(x.getJSONObject("release").toString()),nonce,id,x.getBoolean("wifiOnly"))
        }
    } catch (_: java.io.FileNotFoundException) { null }
    private fun savePending(value: Pending) {
        val data=org.json.JSONObject().put("release",org.json.JSONObject(value.release.json())).put("nonce",value.nonce).put("id",value.id).put("wifiOnly",value.wifiOnly)
        val out=pendingFile.startWrite()
        try { out.write(data.toString().toByteArray()); pendingFile.finishWrite(out) } catch(e:Exception){pendingFile.failWrite(out);throw e}
        prefs.edit().putBoolean("downloadPending",true).commit()
        main.post { UpdateSchedule.recover(app) }
    }
    private fun clearPending() {
        val saved=try { readPending() } catch (_:Exception) { null }
        // Recover the enqueue -> journal crash gap without storing signed URLs.
        val id=saved?.let { if(it.id>0)it.id else downloads.find(it.nonce) }
        if(id!=null)try { downloads.remove(id) } catch (_:Exception) { }
        pendingFile.delete(); prefs.edit().putBoolean("downloadPending",false).commit()
        main.post { UpdateSchedule.recover(app) }
    }
    private fun poll() {
        if (!polling.compareAndSet(false,true)) return
        main.postDelayed({ polling.set(false); if (observers.isNotEmpty() && prefs.getBoolean("downloadPending",false)) reconcileDownload() },1000)
    }
    init { worker.execute { restore() } }
    val connected: Boolean get() = vault.exists()
    val access: UpdateAccessStatus get() {
        val saved = connected
        val result = try { UpdateAccessResult.valueOf(prefs.getString("accessResult", "") ?: "") }
            catch (_: IllegalArgumentException) { if (saved) UpdateAccessResult.SAVED else UpdateAccessResult.NONE }
        return UpdateAccessStatus(saved, if (!saved && result == UpdateAccessResult.VERIFIED) UpdateAccessResult.NONE else result, prefs.getLong("accessVerifiedAt", 0))
    }
    val lastSuccessfulCheck: Long get() = prefs.getLong("lastSuccessfulCheck", 0)
    private fun accessVerified() {
        val now = System.currentTimeMillis()
        prefs.edit().putString("accessResult", UpdateAccessResult.VERIFIED.name).putLong("accessVerifiedAt", now).putLong("lastSuccessfulCheck", now).apply()
    }
    private fun accessFailed(error: Exception) {
        if (error !is CancellationException) prefs.edit().putString("accessResult", if (error is UpdateFailure && error.accessDenied) UpdateAccessResult.DENIED.name else UpdateAccessResult.ERROR.name).apply()
    }
    val installedVersion: String get() = app.packageManager.getPackageInfo(app.packageName, 0).versionName ?: ""
    fun observe(observer: (UpdateStatus) -> Unit) { observers.add(observer); observer(status); if(prefs.getBoolean("downloadPending",false)) reconcileDownload() }
    fun unobserve(observer: (UpdateStatus) -> Unit) { observers.remove(observer) }
    private fun publish(value: UpdateStatus) { main.post { status = value; observers.toList().forEach { it(value) } } }
    private fun errorMessage(e: Exception): String = if (e is UpdateFailure) e.message ?: "Update failed." else "Update could not finish. Check your connection and try again."
    private fun readMetadata(): UpdateRelease? = try { metadata.openRead().use { kotlin.check(it.channel.size() <= 8192); UpdateRelease.restore(String(it.readBytes(), Charsets.UTF_8)) } } catch (_: Exception) { null }
    private fun clearReady() { ready.delete(); metadata.delete() }
    private fun restore() {
        partial.delete()
        val release = readMetadata()
        if (release != null && UpdatePolicy.version(release.version)!! > (UpdatePolicy.version(installedVersion) ?: return)) {
            try { verifyApk(ready, release); clearPending(); publish(UpdateStatus(UpdatePhase.READY, "Version ${release.version} is ready to install.", release)); return } catch (_: Exception) { clearReady() }
        } else clearReady()
        if (pendingFile.baseFile.exists() || prefs.getBoolean("downloadPending",false)) {
            try { if(readPending()!=null) { prefs.edit().putBoolean("downloadPending",true).commit(); reconcileNow(); return } }
            catch (_:Exception) { clearPending(); publish(UpdateStatus(UpdatePhase.ERROR,"Download state could not be restored. Retry the update.")); return }
            clearPending()
        }
        publish(if (connected) UpdateStatus(UpdatePhase.IDLE, "Ready to check for updates.") else UpdateStatus(UpdatePhase.DISCONNECTED, "Connect GitHub to receive private updates."))
    }
    fun connect(token: String) {
        if (active.get() != null) return
        val transfer = UpdateTransfer(); if (!active.compareAndSet(null, transfer)) return
        val replacing = connected
        publish(UpdateStatus(UpdatePhase.CHECKING, "Testing GitHub access before saving the token…"))
        worker.execute {
            try {
                require(token.length in 20..512 && token.all { it.code in 33..126 })
                val release = UpdateRelease.latest(network.releases(token, transfer), installedVersion); transfer.check(); vault.write(token)
                accessVerified()
                prefs.edit().putLong("lastAttempt", System.currentTimeMillis()).apply()
                publish(if (release == null) UpdateStatus(UpdatePhase.IDLE, "GitHub access verified. You're up to date.") else UpdateStatus(UpdatePhase.AVAILABLE, "GitHub access verified. Version ${release.version} is available.", release))
                main.post { UpdateSchedule.sync(app); ReportUploadSchedule.sync(app) }
            } catch (e: Exception) {
                if (!replacing) accessFailed(e)
                val message = if (e is CancellationException) "Access test cancelled." else if (e is IllegalArgumentException) "Enter a valid GitHub access token." else errorMessage(e)
                publish(UpdateStatus(if (e is CancellationException) if (connected) UpdatePhase.IDLE else UpdatePhase.DISCONNECTED else UpdatePhase.ERROR,
                    message + if (replacing) " Existing token was kept." else " Token was not saved."))
            }
            finally { active.compareAndSet(transfer, null) }
        }
    }
    fun disconnect() {
        generation.incrementAndGet(); active.get()?.cancel()
        worker.execute {
            clearPending(); vault.clear(); clearReady(); partial.delete(); prefs.edit().remove("lastAttempt").remove("accessResult").remove("accessVerifiedAt").remove("lastSuccessfulCheck").apply()
            val reportPrefs = app.getSharedPreferences("report-uploads", Context.MODE_PRIVATE)
            if (reportPrefs.getBoolean("automatic", false) || reportPrefs.getLong("accessVerifiedAt", 0) > 0)
                (app.applicationContext as EmulatorApplication).reportUploads.disconnected()
            publish(UpdateStatus(UpdatePhase.DISCONNECTED, "GitHub disconnected. Downloaded updates were removed.")); main.post { UpdateSchedule.sync(app); ReportUploadSchedule.sync(app) }
        }
    }
    fun automatic(finished: () -> Unit = {}): Boolean {
        if (!connected || !prefs.getBoolean("automatic", true)) return false
        val last = prefs.getLong("lastAttempt", 0); val elapsed = System.currentTimeMillis() - last
        if (elapsed in 0 until INTERVAL) return false
        return checkForUpdates(true, finished)
    }
    fun testAccess(): Boolean = checkForUpdates(accessOnly = true)
    fun checkForUpdates(background: Boolean = false, finished: () -> Unit = {}, accessOnly: Boolean = false): Boolean {
        if(prefs.getBoolean("downloadPending",false)) { reconcileDownload(); return false }
        if (!connected) { publish(UpdateStatus(UpdatePhase.DISCONNECTED, "Connect GitHub to receive private updates.")); return false }
        val transfer = UpdateTransfer(background); if (!active.compareAndSet(null, transfer)) return false
        prefs.edit().putLong("lastAttempt", System.currentTimeMillis()).apply(); publish(UpdateStatus(UpdatePhase.CHECKING, if (accessOnly) "Testing GitHub access…" else "Checking for updates…"))
        worker.execute {
            var releasesVerified = false
            try {
                val token = vault.read(); val release = UpdateRelease.latest(network.releases(token, transfer), installedVersion); transfer.check()
                accessVerified(); releasesVerified = true
                val prefix = if (accessOnly) "GitHub access verified. " else ""
                if (release == null) publish(UpdateStatus(UpdatePhase.IDLE, prefix + "You're up to date."))
                else if (readMetadata() == release && ready.isFile) {
                    verifyApk(ready, release); publish(UpdateStatus(UpdatePhase.READY, "Version ${release.version} is ready to install.", release, 100))
                } else {
                    publish(UpdateStatus(UpdatePhase.AVAILABLE, prefix + "Version ${release.version} is available.", release))
                    if (!accessOnly && prefs.getBoolean("downloadWifi", true) && !(app.getSystemService(ConnectivityManager::class.java).isActiveNetworkMetered)) queueDownload(release, token, transfer, wifiOnly = true)
                }
            } catch (e: Exception) {
                if (!releasesVerified) accessFailed(e)
                val cached = readMetadata()
                val preserved = if (cached == null) null else try { verifyApk(ready, cached); UpdateStatus(UpdatePhase.READY, "Version ${cached.version} is ready. " + if (transfer.cancelled.get()) "Check cancelled." else "Could not check for a newer update.", cached, 100) } catch (_: Exception) { null }
                publish(preserved ?: if (transfer.cancelled.get()) UpdateStatus(UpdatePhase.IDLE, "Update cancelled.") else UpdateStatus(UpdatePhase.ERROR, errorMessage(e)))
            }
            finally { partial.delete(); active.compareAndSet(transfer, null); main.post(finished) }
        }; return true
    }
    fun download() {
        if(prefs.getBoolean("downloadPending",false)) { reconcileDownload(); return }
        val release = status.release ?: return; if (!connected) return
        val transfer = UpdateTransfer(); if (!active.compareAndSet(null, transfer)) return
        val epoch=generation.get()
        publish(UpdateStatus(UpdatePhase.DOWNLOADING,"Preparing GitHub download link…",release,-1))
        worker.execute {
            try { transfer.check(); if(epoch!=generation.get())throw CancellationException(); queueDownload(release,vault.read(),transfer,false) }
            catch(e:Exception) {
                if(transfer.cancelled.get() || epoch!=generation.get()) { clearPending(); publish(UpdateStatus(UpdatePhase.AVAILABLE,"Download cancelled. You can retry.",release)) }
                else { clearPending(); publish(UpdateStatus(UpdatePhase.ERROR,errorMessage(e),release)) }
            }
            finally { active.compareAndSet(transfer,null) }
        }
    }
    private fun queueDownload(release: UpdateRelease, token: String, transfer: UpdateTransfer, wifiOnly: Boolean) {
        transfer.check()
        val pending=Pending(release,java.util.UUID.randomUUID().toString(),0,wifiOnly)
        savePending(pending)
        prepare(pending,token,transfer)
    }
    private fun prepare(pending: Pending, token: String, transfer: UpdateTransfer) {
        publish(UpdateStatus(UpdatePhase.DOWNLOADING,"Preparing GitHub download link…",pending.release,-1))
        var id=downloads.find(pending.nonce)
        if(id==null) {
            val url=network.downloadUrl(pending.release,token,transfer)
            UpdatePolicy.checkUrl(url,false); transfer.check()
            id=downloads.enqueue(url,pending.release,pending.nonce,pending.wifiOnly)
            if(id<=0)throw UpdateFailure("Android could not queue the download. Retry.")
        }
        savePending(pending.copy(id=id))
        transfer.check()
        publish(UpdateStatus(UpdatePhase.DOWNLOADING,"Queued with Android. You can leave the app; downloading continues in the background.",pending.release,-1))
        main.post { poll() }
    }
    fun reconcileDownload(finished: () -> Unit = {}) {
        worker.execute {
            try { reconcileNow() }
            catch(e:Exception) { val r=try { readPending()?.release } catch (_:Exception) { null };clearPending();partial.delete();publish(UpdateStatus(UpdatePhase.ERROR,errorMessage(e),r)) }
            finally { main.post { UpdateSchedule.recover(app);finished() } }
        }
    }
    private fun reconcileNow() {
        val pending=readPending() ?: return
        if(pending.id==0L) {
            if(!connected) { clearPending();publish(UpdateStatus(UpdatePhase.ERROR,"Reconnect GitHub to resume preparing this update.",pending.release));return }
            val transfer=UpdateTransfer(true)
            if(!active.compareAndSet(null,transfer))return
            val epoch=generation.get()
            try { prepare(pending,vault.read(),transfer) }
            catch(e:Exception) {
                if(transfer.cancelled.get() && epoch==generation.get())publish(UpdateStatus(UpdatePhase.DOWNLOADING,"Download preparation paused. Android will retry.",pending.release,-1))
                else throw e
            } finally { active.compareAndSet(transfer,null) }
            return
        }
        val row=downloads.query(pending.id)
        if(row==null) { clearPending();publish(UpdateStatus(UpdatePhase.AVAILABLE,"Download cancelled or removed. You can retry.",pending.release));return }
        if(row.bytes>pending.release.size || row.total>0 && row.total!=pending.release.size)throw UpdateFailure("Download size changed. The file was discarded; retry the update.")
        val p=if(row.bytes>0)(row.bytes*100/pending.release.size).coerceIn(0,100).toInt() else -1
        when(row.state) {
            android.app.DownloadManager.STATUS_SUCCESSFUL -> {
                val epoch=generation.get()
                publish(UpdateStatus(UpdatePhase.DOWNLOADING,"Verifying downloaded APK…",pending.release,-1))
                downloads.open(pending.id).use { input -> FileOutputStream(partial).use { output ->
                    UpdatePolicy.copyVerified(input,output,pending.release.size,pending.release.sha,{epoch!=generation.get()},{})
                    output.fd.sync()
                } }
                if(epoch!=generation.get())throw CancellationException()
                verifyApk(partial,pending.release)
                if(epoch!=generation.get())throw CancellationException()
                if(!partial.renameTo(ready))throw UpdateFailure("Could not save the verified update.")
                val out=metadata.startWrite()
                try { out.write(pending.release.json().toByteArray());metadata.finishWrite(out) }
                catch(e:Exception) { metadata.failWrite(out);clearReady();throw e }
                clearPending();publish(UpdateStatus(UpdatePhase.READY,"Version ${pending.release.version} is ready to install.",pending.release,100))
            }
            android.app.DownloadManager.STATUS_FAILED -> {
                val message=when(row.reason) {
                    400,401,403,404,410 -> "GitHub's download link expired or was refused. Retry to get a fresh link."
                    android.app.DownloadManager.ERROR_INSUFFICIENT_SPACE -> "Not enough storage for the update. Free space and retry."
                    else -> "Android could not finish the download. Retry on a working connection."
                };throw UpdateFailure(message)
            }
            else -> {
                val message=when {
                    row.state==android.app.DownloadManager.STATUS_PAUSED -> if(row.reason==android.app.DownloadManager.PAUSED_QUEUED_FOR_WIFI)"Waiting for unmetered Wi-Fi. Android will continue automatically." else "Waiting for the network. Android will continue automatically."
                    row.state==android.app.DownloadManager.STATUS_PENDING -> "Queued with Android. Downloading continues when you leave the app."
                    else -> "Downloading ${pending.release.version}… ${row.bytes/1024} KB of ${pending.release.size/1024} KB" + if(p>=0)" · $p%" else " · waiting for data"
                }
                publish(UpdateStatus(UpdatePhase.DOWNLOADING,message,pending.release,p));main.post { poll() }
            }
        }
    }
    fun cancel(backgroundOnly: Boolean = false) {
        if(backgroundOnly) { active.get()?.let { if(it.background)it.cancel() }; return }
        generation.incrementAndGet();active.get()?.cancel()
        worker.execute { val release=try { readPending()?.release ?: status.release }catch (_:Exception){status.release};clearPending();partial.delete();publish(UpdateStatus(if(release!=null)UpdatePhase.AVAILABLE else UpdatePhase.IDLE,"Download cancelled. You can retry.",release)) }
    }
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
