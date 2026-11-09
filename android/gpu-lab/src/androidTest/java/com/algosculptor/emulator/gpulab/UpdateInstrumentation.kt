package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.app.Instrumentation
import android.app.job.JobScheduler
import android.content.Context
import android.net.Uri
import android.os.Bundle
import android.util.AtomicFile
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayInputStream
import java.io.File
import java.io.FileOutputStream
import java.io.FilterInputStream
import java.io.InputStream
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean

private class FakeUpdateDownloads(private val bytes:()->ByteArray):UpdateDownloads {
    private data class Row(val release:UpdateRelease,val nonce:String,val data:()->ByteArray,var state:Int,var reason:Int)
    private val rows=linkedMapOf<Long,Row>();private var nextId=1L
    @Volatile var nextState=android.app.DownloadManager.STATUS_SUCCESSFUL
    @Volatile var nextReason=0
    @Synchronized override fun enqueue(url:String,release:UpdateRelease,nonce:String,wifiOnly:Boolean):Long {
        UpdatePolicy.checkUrl(url,false);val id=nextId++;rows[id]=Row(release,nonce,bytes,nextState,nextReason);nextState=android.app.DownloadManager.STATUS_SUCCESSFUL;nextReason=0;return id
    }
    @Synchronized override fun find(nonce:String):Long?=rows.entries.firstOrNull{it.value.nonce==nonce}?.key
    @Synchronized override fun query(id:Long):DownloadSnapshot?=rows[id]?.let{r->DownloadSnapshot(r.state,if(r.state==android.app.DownloadManager.STATUS_RUNNING)r.release.size/2 else if(r.state==android.app.DownloadManager.STATUS_SUCCESSFUL)r.release.size else 0,r.release.size,r.reason)}
    @Synchronized override fun open(id:Long):InputStream=ByteArrayInputStream(rows[id]?.data?.invoke()?:error("Missing fake download"))
    @Synchronized override fun remove(id:Long){rows.remove(id)}
    @Synchronized fun contains(id:Long)=rows.containsKey(id)
    @Synchronized fun setState(id:Long,state:Int,reason:Int=0){rows[id]?.let{it.state=state;it.reason=reason}}
}

/** QA-only runner; no endpoint override or test activity is shipped in the app. */
class UpdateInstrumentation : Instrumentation() {
    private val checks = mutableListOf<String>()
    private val dummy = "QA_ONLY_FAKE_GITHUB_TOKEN_NOT_A_CREDENTIAL"
    private var accessUi = false
    private var reportQa = false
    private var updateVersion = "0.27.2-dev"
    private var mode = ""
    private fun sha(data: ByteArray) = MessageDigest.getInstance("SHA-256").digest(data).joinToString("") { "%02x".format(it) }
    private fun until(message: String, timeoutMillis:Long=30_000, predicate: () -> Boolean) {
        val end = System.currentTimeMillis() + timeoutMillis
        while (System.currentTimeMillis() < end) { if (predicate()) { Thread.sleep(80); return }; Thread.sleep(20) }
        error(message)
    }
    private fun check(name: String, action: () -> Unit) { action(); checks.add(name); sendStatus(0, Bundle().apply { putString("stream", "Verified $name\n") }) }
    private fun rejected(action: () -> Unit) { var rejected = false; try { action() } catch (_: Exception) { rejected = true }; require(rejected) }
    override fun onCreate(arguments: Bundle?) { super.onCreate(arguments); mode=arguments?.getString("mode")?:"";accessUi = mode == "access-ui"; reportQa = mode == "reports"; updateVersion = arguments?.getString("qaUpdateVersion") ?: updateVersion; start() }
    override fun onStart() {
        if (reportQa) { ReportUploadQa(this).run(); return }
        if (accessUi) { UpdateAccessQa(this).run(); return }
        if (mode == "background-dm-start") { startBackgroundDownload(); return }
        if (mode == "background-dm-verify") { verifyBackgroundDownload(); return }
        val app = targetContext.applicationContext as EmulatorApplication
        val folder = File(app.filesDir, "updates").apply { mkdirs() }
        try {
            val vault = UpdateVault(app);vault.clear()
            app.getSharedPreferences("updates",Activity.MODE_PRIVATE).edit().remove("downloadPending").remove("accessResult").remove("accessVerifiedAt").remove("lastSuccessfulCheck").commit()
            for(name in listOf("pending.json","ready.json","ready.json.bak","ready.apk","download.part"))File(folder,name).delete()
            until("Initial updater did not restore") { app.updates.status.phase != UpdatePhase.IDLE || app.updates.status.message != "Preparing updates…" }
            val secretFile = File(app.noBackupFilesDir, "updates-access.bin")
            check("Keystore encrypted token roundtrip") { vault.write(dummy); require(vault.read() == dummy); require(!String(secretFile.readBytes(), Charsets.ISO_8859_1).contains(dummy)) }
            check("Modified ciphertext fails authentication") { val original=secretFile.readBytes();val bytes=original.clone();bytes[bytes.lastIndex]=(bytes.last().toInt() xor 1).toByte();secretFile.writeBytes(bytes);rejected { vault.read() };secretFile.writeBytes(original);require(vault.read()==dummy) }
            check("Disconnect erases encrypted token") { vault.clear(); require(!vault.exists()) }
            val apk = context.assets.open("update.apk").use { it.readBytes() }
            val release = UpdateRelease("v$updateVersion", updateVersion, 123, apk.size.toLong(), sha(apk), "QA update")
            fun listing(r: UpdateRelease): String = JSONArray().put(JSONObject().put("tag_name",r.tag).put("draft",false).put("prerelease",true).put("html_url",r.page).put("body",r.notes).put("assets",JSONArray().put(JSONObject().put("name","emulator.apk").put("id",r.asset).put("state","uploaded").put("size",r.size).put("digest","sha256:${r.sha}").put("url",UpdatePolicy.apiAsset(r.asset))))).toString()
            class Fake : UpdateNetwork {
                var json="[]";var bytes=apk;var fail=false;var requests=0;val slow=AtomicBoolean(false)
                override fun releases(token:String,transfer:UpdateTransfer):String { requests++; transfer.check(); if(fail)throw UpdateFailure("Controlled test outage."); return json }
                override fun downloadUrl(release:UpdateRelease,token:String,transfer:UpdateTransfer):String { transfer.check(); return "https://release-assets.githubusercontent.com/qa/${release.asset}" }
                override fun asset(release:UpdateRelease,token:String,transfer:UpdateTransfer,consume:(InputStream)->Unit) {
                    val input=object:FilterInputStream(ByteArrayInputStream(bytes)) {
                        override fun read(buffer:ByteArray,off:Int,len:Int):Int { transfer.check();if(slow.get())Thread.sleep(15);return super.read(buffer,off,minOf(len,16384)) }
                    };input.use(consume)
                }
            }
            val net=Fake();val fakeDownloads=FakeUpdateDownloads{net.bytes};val manager=UpdateManager(app,net,vault,fakeDownloads)
            val observe: (UpdateStatus)->Unit = {};manager.observe(observe)
            manager.prefs.edit().putBoolean("automatic",false).putBoolean("downloadWifi",false).commit()
            until("Manager not initialized") { manager.status.phase==UpdatePhase.DISCONNECTED }
            fun connect() { manager.connect(dummy);until("Connect did not complete") { manager.status.phase==UpdatePhase.IDLE && vault.exists() } }
            fun offer(r:UpdateRelease=release) { net.fail=false;net.json=listing(r);manager.checkForUpdates();until("Release not offered: ${manager.status}"){manager.status.phase==UpdatePhase.AVAILABLE};require(manager.status.release==r) }
            fun clearFiles(){File(folder,"ready.apk").delete();AtomicFile(File(folder,"ready.json")).delete()}
            check("No background update without private access") { require(!manager.automatic()) }
            check("Successful connection stores only encrypted access") { connect();require(vault.read()==dummy) }
            check("Access errors preserve existing credential") { net.fail=true;manager.connect("ANOTHER_QA_DUMMY_TOKEN_NEVER_REAL");until("Expected connection error"){manager.status.phase==UpdatePhase.ERROR};require(vault.read()==dummy);net.fail=false }
            check("Newest development release is offered without downloading") { offer();require(!File(folder,"download.part").exists()&&!File(folder,"ready.apk").exists()) }
            check("SHA package version signer and ABI verified before ready") { manager.download();until("APK not ready: ${manager.status}"){manager.status.phase==UpdatePhase.READY};require(File(folder,"ready.apk").readBytes().contentEquals(apk)) }
            check("Installer metadata read never starts a release check or clears ready state") {
                val before=net.requests;val completed=AtomicBoolean(false)
                manager.verifiedForInstall { file,r,error -> require(file!=null&&r==release&&error==null);completed.set(true) }
                until("Installer verification did not complete"){completed.get()}
                require(net.requests==before&&manager.status.phase==UpdatePhase.READY)
            }
            val uri=Uri.parse("content://${app.packageName}.updates/apk/${release.sha}.apk")
            check("Installer content URI reads only the verified APK") { app.contentResolver.openInputStream(uri)!!.use{require(sha(it.readBytes())==release.sha)};require(app.contentResolver.getType(uri)=="application/vnd.android.package-archive") }
            check("Provider rejects write access traversal stale and unknown paths") {
                for(mode in listOf("w","rw","rwt"))rejected{app.contentResolver.openFileDescriptor(uri,mode)?.close()}
                for(path in listOf("/apk/../ready.json","/ready.json","/apk/${"0".repeat(64)}.apk","/apk/%2e%2e/ready.apk"))rejected{app.contentResolver.openInputStream(Uri.parse("content://${app.packageName}.updates$path"))?.close()}
            }
            check("Ready update survives a network outage") { net.fail=true;manager.checkForUpdates();until("Cached APK lost after check failure"){manager.status.phase==UpdatePhase.READY};require(File(folder,"ready.apk").isFile);net.fail=false }
            check("Verified download restores after manager recreation") { val restored=UpdateManager(app,net,vault);until("Ready metadata did not restore"){restored.status.phase==UpdatePhase.READY};require(restored.status.release==release) }
            check("Installer revalidation rejects later APK modification") { val file=File(folder,"ready.apk");val bytes=file.readBytes();bytes[bytes.lastIndex]=(bytes.last().toInt() xor 1).toByte();file.writeBytes(bytes);rejected{manager.verifyApk(file,release)};clearFiles() }
            check("Wrong release version is rejected") { val file=File(folder,"candidate.apk");file.writeBytes(apk);rejected{manager.verifyApk(file,release.copy(version="0.99.0-dev"))};file.delete() }
            check("Wrong package is rejected even with valid APK bytes") { val bytes=context.assets.open("wrong-package.apk").use{it.readBytes()};val file=File(folder,"candidate.apk");file.writeBytes(bytes);rejected{manager.verifyApk(file,release.copy(size=bytes.size.toLong(),sha=sha(bytes)))};file.delete() }
            check("Different signing certificate is rejected") { val bytes=context.assets.open("wrong-signer.apk").use{it.readBytes()};val file=File(folder,"candidate.apk");file.writeBytes(bytes);rejected{manager.verifyApk(file,release.copy(size=bytes.size.toLong(),sha=sha(bytes)))};file.delete() }
            check("Installed or older APK cannot downgrade the app") { val bytes=context.assets.open("old.apk").use{it.readBytes()};val file=File(folder,"candidate.apk");file.writeBytes(bytes);rejected{manager.verifyApk(file,release.copy(version="0.27.0-dev",size=bytes.size.toLong(),sha=sha(bytes)))};file.delete() }
            check("Checksum failure discards partial download") { offer(release.copy(sha="0".repeat(64)));manager.download();until("Bad checksum accepted"){manager.status.phase==UpdatePhase.ERROR};require(!File(folder,"download.part").exists()&&!File(folder,"ready.apk").exists()) }
            check("Truncated download is rejected and cleaned") { offer();net.bytes=apk.copyOf(1024);manager.download();until("Truncated update accepted"){manager.status.phase==UpdatePhase.ERROR};require(!File(folder,"download.part").exists());net.bytes=apk }
            check("Paused Wi-Fi transfer reports the system reason and completes after resume") {
                clearFiles();offer();fakeDownloads.nextState=android.app.DownloadManager.STATUS_PAUSED;fakeDownloads.nextReason=android.app.DownloadManager.PAUSED_QUEUED_FOR_WIFI
                manager.download();until("Wi-Fi pause not reported"){manager.status.message.contains("unmetered Wi-Fi")}
                val id=AtomicFile(File(folder,"pending.json")).openRead().use{JSONObject(it.readBytes().toString(Charsets.UTF_8)).getLong("id")}
                fakeDownloads.setState(id,android.app.DownloadManager.STATUS_SUCCESSFUL);manager.reconcileDownload()
                until("Resumed transfer did not verify"){manager.status.phase==UpdatePhase.READY};require(File(folder,"ready.apk").readBytes().contentEquals(apk))
            }
            check("Missing DownloadManager row clears stale journal and permits retry") {
                clearFiles();offer();fakeDownloads.nextState=android.app.DownloadManager.STATUS_RUNNING;manager.download()
                until("Running transfer not queued"){manager.status.message.startsWith("Queued with Android")}
                val id=AtomicFile(File(folder,"pending.json")).openRead().use{JSONObject(it.readBytes().toString(Charsets.UTF_8)).getLong("id")}
                fakeDownloads.remove(id);manager.reconcileDownload()
                until("Missing row did not return to retry state"){manager.status.phase==UpdatePhase.AVAILABLE}
                require(!manager.prefs.getBoolean("downloadPending",false))
            }
            check("System download HTTP failure is reported and cleaned") {
                offer();fakeDownloads.nextState=android.app.DownloadManager.STATUS_FAILED;fakeDownloads.nextReason=403;manager.download()
                until("Failed transfer not reported"){manager.status.phase==UpdatePhase.ERROR}
                require(!manager.prefs.getBoolean("downloadPending",false))
            }
            check("Cancellation removes the Android-owned queued download and permits retry") {
                offer();fakeDownloads.nextState=android.app.DownloadManager.STATUS_RUNNING;manager.download()
                until("Download was not handed to Android") { manager.status.message.startsWith("Queued with Android") }
                val pending=AtomicFile(File(folder,"pending.json")).openRead().use{JSONObject(it.readBytes().toString(Charsets.UTF_8)).getLong("id")}
                require(fakeDownloads.contains(pending));manager.cancel()
                until("Cancellation not reflected") { manager.status.phase==UpdatePhase.AVAILABLE }
                require(!fakeDownloads.contains(pending)&&!manager.prefs.getBoolean("downloadPending",false))
            }
            check("Disabled automatic checks do not run") { require(!manager.automatic()) }
            check("Periodic job has network battery storage and reboot constraints") {
                manager.prefs.edit().putBoolean("automatic",true).commit();runOnMainSync{UpdateSchedule.sync(app)}
                val job=app.getSystemService(JobScheduler::class.java).getPendingJob(UpdateSchedule.ID)!!;require(job.isPeriodic&&job.isPersisted&&job.isRequireBatteryNotLow&&job.isRequireStorageNotLow);require(job.intervalMillis==UpdateManager.INTERVAL)
            }
            check("Disabling automatic checks cancels periodic work") { manager.prefs.edit().putBoolean("automatic",false).commit();runOnMainSync{UpdateSchedule.sync(app)};require(app.getSystemService(JobScheduler::class.java).getPendingJob(UpdateSchedule.ID)==null) }
            check("Successful retry leaves a real installable update") { offer();manager.download();until("Retry did not verify"){manager.status.phase==UpdatePhase.READY};require(File(folder,"ready.apk").isFile) }
            vault.clear() // Synthetic QA access never survives the test; keep only the verified candidate for real installer UI QA.
            manager.unobserve(observe)
            manager.prefs.edit().remove("lastAttempt").commit()
            val result=JSONObject().put("passed",true).put("checks",JSONArray(checks)).put("candidateVersion",release.version).put("candidateSha256",release.sha)
            File(app.filesDir,"update-qa-result.json").writeText(result.toString(2));finish(Activity.RESULT_OK,Bundle().apply{putString("stream",result.toString()+"\n")})
        } catch(e:Exception) {
            UpdateVault(app).clear()
            app.getSharedPreferences("updates",Activity.MODE_PRIVATE).edit().remove("downloadPending").remove("accessResult").remove("accessVerifiedAt").remove("lastSuccessfulCheck").commit()
            for(name in listOf("pending.json","pending.json.bak","ready.json","ready.json.bak","ready.apk","download.part"))File(folder,name).delete()
            finish(Activity.RESULT_CANCELED,Bundle().apply{putString("stream","Updater QA failed after ${checks.size} checks: ${e.javaClass.simpleName}: ${e.message}\n")})
        }
    }

    private fun backgroundRelease(): Pair<UpdateRelease,ByteArray> {
        val apk=context.assets.open("update.apk").use{it.readBytes()}
        return UpdateRelease("v$updateVersion",updateVersion,123,apk.size.toLong(),sha(apk),"Temporary lifecycle QA") to apk
    }
    private fun cleanupBackgroundQa(app:EmulatorApplication) {
        val folder=File(app.filesDir,"updates")
        val journal=AtomicFile(File(folder,"pending.json"))
        val id=try{journal.openRead().use{JSONObject(it.readBytes().toString(Charsets.UTF_8)).optLong("id",0)}}catch(_:Exception){0L}
        if(id>0)try{AndroidUpdateDownloads(app).remove(id)}catch(_:Exception){}
        journal.delete();AtomicFile(File(folder,"ready.json")).delete()
        for(name in listOf("ready.apk","download.part"))File(folder,name).delete()
        UpdateVault(app).clear()
        app.getSharedPreferences("updates",Activity.MODE_PRIVATE).edit().putBoolean("downloadPending",false).remove("accessResult").remove("accessVerifiedAt").remove("lastSuccessfulCheck").commit()
        UpdateSchedule.recover(app)
    }
    private fun startBackgroundDownload() {
        val app=targetContext.applicationContext as EmulatorApplication
        try {
            val vault=UpdateVault(app);vault.clear();vault.write(dummy)
            val prefs=app.getSharedPreferences("updates",Activity.MODE_PRIVATE)
            prefs.edit().putBoolean("automatic",false).putBoolean("downloadWifi",false).putBoolean("downloadPending",false).commit()
            val (release,_)=backgroundRelease()
            val urlFile=File(app.filesDir,"update-qa-signed-url")
            require(urlFile.isFile&&urlFile.length() in 1..8192L)
            val signedUrl=urlFile.readText(Charsets.UTF_8).trim();urlFile.delete()
            UpdatePolicy.checkUrl(signedUrl,false)
            val releases=JSONArray().put(JSONObject().put("tag_name",release.tag).put("draft",false).put("prerelease",true)
                .put("html_url",release.page).put("body",release.notes).put("assets",JSONArray().put(JSONObject()
                    .put("name","emulator.apk").put("id",release.asset).put("state","uploaded").put("size",release.size)
                    .put("digest","sha256:${release.sha}").put("url",UpdatePolicy.apiAsset(release.asset))))).toString()
            val network=object:UpdateNetwork {
                override fun releases(token:String,transfer:UpdateTransfer):String { transfer.check();return releases }
                override fun downloadUrl(r:UpdateRelease,token:String,transfer:UpdateTransfer):String { transfer.check();require(r==release);return signedUrl }
                override fun asset(r:UpdateRelease,token:String,transfer:UpdateTransfer,consume:(InputStream)->Unit)=throw UpdateFailure("The OS download test must use DownloadManager.")
            }
            val manager=UpdateManager(app,network,vault,AndroidUpdateDownloads(app))
            val observer:(UpdateStatus)->Unit={}
            manager.observe(observer)
            require(manager.checkForUpdates())
            until("Temporary release was not offered") { manager.status.phase==UpdatePhase.AVAILABLE }
            manager.download()
            until("DownloadManager did not accept the request") {
                val pending=File(app.filesDir,"updates/pending.json")
                pending.isFile&&AtomicFile(pending).openRead().use{JSONObject(it.readBytes().toString(Charsets.UTF_8)).optLong("id",0)>0}
            }
            val id=AtomicFile(File(app.filesDir,"updates/pending.json")).openRead().use{JSONObject(it.readBytes().toString(Charsets.UTF_8)).getLong("id")}
            check(id>0)
            val downloads=AndroidUpdateDownloads(app)
            until("Could not observe the offline queued transfer before process death",15_000) {
                downloads.query(id)?.let{it.bytes==0L&&it.state in setOf(android.app.DownloadManager.STATUS_PENDING,android.app.DownloadManager.STATUS_PAUSED)}==true
            }
            val state=downloads.query(id)?.state?:-1
            finish(Activity.RESULT_OK,Bundle().apply{putString("stream","Android DownloadManager persisted offline queue id $id (state $state); URL was removed from app storage.\n")})
        } catch(e:Exception) {
            File(app.filesDir,"update-qa-signed-url").delete();cleanupBackgroundQa(app)
            finish(Activity.RESULT_CANCELED,Bundle().apply{putString("stream","Background DownloadManager setup failed: ${e.javaClass.simpleName}: ${e.message}\n")})
        }
    }
    private fun verifyBackgroundDownload() {
        val app=targetContext.applicationContext as EmulatorApplication
        val observer:(UpdateStatus)->Unit={}
        try {
            app.updates.observe(observer)
            until("OS transfer or restored verifier did not complete: ${app.updates.status}",120_000) {
                app.updates.status.phase in setOf(UpdatePhase.READY,UpdatePhase.ERROR)
            }
            check(app.updates.status.phase==UpdatePhase.READY) { "Restored updater rejected the completed transfer: ${app.updates.status.message}" }
            val expected=UpdateRelease.restore(AtomicFile(File(app.filesDir,"updates/ready.json")).openRead().use{it.readBytes().toString(Charsets.UTF_8)})
            val ready=File(app.filesDir,"updates/ready.apk")
            app.updates.verifyApk(ready,expected)
            val result=JSONObject().put("passed",true).put("phase",app.updates.status.phase.name)
                .put("version",expected.version).put("size",ready.length()).put("sha256",sha(ready.readBytes()))
                .put("processRestored",true).put("downloadManagerContinued",true)
            File(app.filesDir,"background-download-qa-result.json").writeText(result.toString(2))
            check(!app.getSharedPreferences("updates",Activity.MODE_PRIVATE).getBoolean("downloadPending",true))
            cleanupBackgroundQa(app)
            finish(Activity.RESULT_OK,Bundle().apply{putString("stream",result.toString()+"\n")})
        } catch(e:Exception) {
            cleanupBackgroundQa(app)
            finish(Activity.RESULT_CANCELED,Bundle().apply{putString("stream","Background DownloadManager restore failed: ${e.javaClass.simpleName}: ${e.message}\n")})
        } finally { app.updates.unobserve(observer) }
    }
}
