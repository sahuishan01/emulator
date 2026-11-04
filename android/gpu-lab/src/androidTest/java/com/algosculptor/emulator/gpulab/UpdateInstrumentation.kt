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

/** QA-only runner; no endpoint override or test activity is shipped in the app. */
class UpdateInstrumentation : Instrumentation() {
    private val checks = mutableListOf<String>()
    private val dummy = "QA_ONLY_FAKE_GITHUB_TOKEN_NOT_A_CREDENTIAL"
    private var accessUi = false
    private fun sha(data: ByteArray) = MessageDigest.getInstance("SHA-256").digest(data).joinToString("") { "%02x".format(it) }
    private fun until(message: String, predicate: () -> Boolean) {
        val end = System.currentTimeMillis() + 30_000
        while (System.currentTimeMillis() < end) { if (predicate()) { Thread.sleep(80); return }; Thread.sleep(20) }
        error(message)
    }
    private fun check(name: String, action: () -> Unit) { action(); checks.add(name); sendStatus(0, Bundle().apply { putString("stream", "Verified $name\n") }) }
    private fun rejected(action: () -> Unit) { var rejected = false; try { action() } catch (_: Exception) { rejected = true }; require(rejected) }
    override fun onCreate(arguments: Bundle?) { super.onCreate(arguments); accessUi = arguments?.getString("mode") == "access-ui"; start() }
    override fun onStart() {
        if (accessUi) { UpdateAccessQa(this).run(); return }
        val app = targetContext.applicationContext as EmulatorApplication
        val folder = File(app.filesDir, "updates").apply { mkdirs() }
        try {
            until("Initial updater did not restore") { app.updates.status.phase != UpdatePhase.IDLE || app.updates.status.message != "Preparing updates…" }
            val vault = UpdateVault(app); val secretFile = File(app.noBackupFilesDir, "updates-access.bin")
            check("Keystore encrypted token roundtrip") { vault.write(dummy); require(vault.read() == dummy); require(!String(secretFile.readBytes(), Charsets.ISO_8859_1).contains(dummy)) }
            check("Modified ciphertext fails authentication") { val original=secretFile.readBytes();val bytes=original.clone();bytes[bytes.lastIndex]=(bytes.last().toInt() xor 1).toByte();secretFile.writeBytes(bytes);rejected { vault.read() };secretFile.writeBytes(original);require(vault.read()==dummy) }
            check("Disconnect erases encrypted token") { vault.clear(); require(!vault.exists()) }
            val apk = context.assets.open("update.apk").use { it.readBytes() }
            val release = UpdateRelease("v0.27.2-dev", "0.27.2-dev", 123, apk.size.toLong(), sha(apk), "QA update")
            fun listing(r: UpdateRelease): String = JSONArray().put(JSONObject().put("tag_name",r.tag).put("draft",false).put("prerelease",true).put("html_url",r.page).put("body",r.notes).put("assets",JSONArray().put(JSONObject().put("name","emulator.apk").put("id",r.asset).put("state","uploaded").put("size",r.size).put("digest","sha256:${r.sha}").put("url",UpdatePolicy.apiAsset(r.asset))))).toString()
            class Fake : UpdateNetwork {
                var json="[]";var bytes=apk;var fail=false;var requests=0;val slow=AtomicBoolean(false)
                override fun releases(token:String,transfer:UpdateTransfer):String { requests++; transfer.check(); if(fail)throw UpdateFailure("Controlled test outage."); return json }
                override fun asset(release:UpdateRelease,token:String,transfer:UpdateTransfer,consume:(InputStream)->Unit) {
                    val input=object:FilterInputStream(ByteArrayInputStream(bytes)) {
                        override fun read(buffer:ByteArray,off:Int,len:Int):Int { transfer.check();if(slow.get())Thread.sleep(15);return super.read(buffer,off,minOf(len,16384)) }
                    };input.use(consume)
                }
            }
            val net=Fake(); val manager=UpdateManager(app,net,vault)
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
            check("Cancellation discards partial bytes and permits retry") { offer();net.slow.set(true);manager.download();until("No partial download"){File(folder,"download.part").length()>0};manager.cancel();until("Cancellation not reflected"){manager.status.phase==UpdatePhase.AVAILABLE};require(!File(folder,"download.part").exists());net.slow.set(false) }
            check("Disabled automatic checks do not run") { require(!manager.automatic()) }
            check("Periodic job has network battery storage and reboot constraints") {
                manager.prefs.edit().putBoolean("automatic",true).commit();runOnMainSync{UpdateSchedule.sync(app)}
                val job=app.getSystemService(JobScheduler::class.java).getPendingJob(UpdateSchedule.ID)!!;require(job.isPeriodic&&job.isPersisted&&job.isRequireBatteryNotLow&&job.isRequireStorageNotLow);require(job.intervalMillis==UpdateManager.INTERVAL)
            }
            check("Disabling automatic checks cancels periodic work") { manager.prefs.edit().putBoolean("automatic",false).commit();runOnMainSync{UpdateSchedule.sync(app)};require(app.getSystemService(JobScheduler::class.java).getPendingJob(UpdateSchedule.ID)==null) }
            check("Successful retry leaves a real installable update") { offer();manager.download();until("Retry did not verify"){manager.status.phase==UpdatePhase.READY};require(File(folder,"ready.apk").isFile) }
            vault.clear() // Synthetic QA access never survives the test; keep only the verified candidate for real installer UI QA.
            manager.prefs.edit().remove("lastAttempt").commit()
            val result=JSONObject().put("passed",true).put("checks",JSONArray(checks)).put("candidateVersion",release.version).put("candidateSha256",release.sha)
            File(app.filesDir,"update-qa-result.json").writeText(result.toString(2));finish(Activity.RESULT_OK,Bundle().apply{putString("stream",result.toString()+"\n")})
        } catch(e:Exception) {
            finish(Activity.RESULT_CANCELED,Bundle().apply{putString("stream","Updater QA failed after ${checks.size} checks: ${e.javaClass.simpleName}: ${e.message}\n")})
        }
    }
}
