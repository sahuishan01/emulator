package com.algosculptor.emulator.gpulab

import android.app.Activity
import android.app.Instrumentation
import android.app.job.JobScheduler
import android.content.Intent
import android.content.pm.ActivityInfo
import android.graphics.Bitmap
import android.graphics.Rect
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.Button
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.InputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/** Runs only inside the QA instrumentation package, with synthetic credentials. */
internal class UpdateAccessQa(private val test: Instrumentation) {
    private val app get() = test.targetContext.applicationContext as EmulatorApplication
    private val checks = mutableListOf<String>()
    private val dummy = "QA_ONLY_FAKE_ACCESS_TOKEN_NOT_A_CREDENTIAL"
    private lateinit var activity: UpdatesActivity
    private fun until(message: String, condition: () -> Boolean) {
        val end = System.currentTimeMillis() + 30_000
        while (System.currentTimeMillis() < end) { if (condition()) { test.waitForIdleSync(); return }; Thread.sleep(25) }
        error(message)
    }
    private fun check(name: String, run: () -> Unit) {
        run(); checks.add(name); test.sendStatus(0, Bundle().apply { putString("stream", "Verified $name\n") })
    }
    private fun views(view: View): List<View> = listOf(view) + if (view is ViewGroup) (0 until view.childCount).flatMap { views(view.getChildAt(it)) } else emptyList()
    private fun button(label: String): Button = views(activity.window.decorView).filterIsInstance<Button>().single { it.text.toString() == label }
    private fun textContains(text: String): Boolean {
        var found = false
        test.runOnMainSync { found = views(activity.window.decorView).filterIsInstance<TextView>().any { it.text.contains(text) } }
        return found
    }
    private fun click(label: String) { test.runOnMainSync { require(button(label).isEnabled); require(button(label).performClick()) }; test.waitForIdleSync() }
    private fun visible(label: String) { test.runOnMainSync { require(button(label).getGlobalVisibleRect(Rect())) } }
    private fun dialogButton(label: String) {
        var node: AccessibilityNodeInfo? = null
        until("Missing dialog control $label") { node = test.uiAutomation.rootInActiveWindow?.findAccessibilityNodeInfosByText(label)?.firstOrNull { it.text?.toString()?.equals(label, ignoreCase = true) == true && it.className.toString().endsWith("Button") }; node != null }
        require(node!!.performAction(AccessibilityNodeInfo.ACTION_CLICK)); test.waitForIdleSync()
    }
    private fun enterToken(value: String) {
        fun input(root: AccessibilityNodeInfo): AccessibilityNodeInfo? {
            if (root.className.toString().endsWith("EditText")) return root
            for (i in 0 until root.childCount) root.getChild(i)?.let { input(it)?.let { return it } }
            return null
        }
        var node: AccessibilityNodeInfo? = null
        until("Token input missing") { node = test.uiAutomation.rootInActiveWindow?.let { input(it) }; node != null }
        require(node!!.isPassword)
        require(node!!.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, value) }))
    }
    private fun connect(value: String, network: Fake, manager: UpdateManager) {
        val before = network.requests.get()
        click(if (manager.connected) "Replace GitHub access" else "Connect GitHub")
        enterToken(value); dialogButton("Verify and save")
        until("Access verification did not finish") { network.requests.get() > before && !manager.status.busy }
    }
    private fun snapshot(name: String) {
        val folder = File(app.filesDir, "access-ui-qa").apply { mkdirs() }
        File(folder, "$name.png").outputStream().use { output -> require(test.uiAutomation.takeScreenshot().compress(Bitmap.CompressFormat.PNG, 100, output)) }
    }
    private class Fake : UpdateNetwork {
        @Volatile var failure: UpdateFailure? = null
        @Volatile var listing = "[]"
        @Volatile var gate: CountDownLatch? = null
        val requests = AtomicInteger()
        val assets = AtomicInteger()
        override fun releases(token: String, transfer: UpdateTransfer): String {
            requests.incrementAndGet(); transfer.check()
            gate?.let { while (!it.await(20, TimeUnit.MILLISECONDS)) transfer.check() }
            transfer.check(); failure?.let { throw it }; return listing
        }
        override fun asset(release: UpdateRelease, token: String, transfer: UpdateTransfer, consume: (InputStream) -> Unit) {
            assets.incrementAndGet(); throw UpdateFailure("A token access test must not download an APK.")
        }
    }
    fun run() {
        val field = EmulatorApplication::class.java.getDeclaredField("updates\$delegate").apply { isAccessible = true }
        val original = field.get(app)
        val vault = UpdateVault(app); val network = Fake(); var manager: UpdateManager? = null
        var passed = false
        var stream = "Access UI QA did not complete.\n"
        try {
            until("Initial updater not restored") { app.updates.status.message != "Preparing updates…" }
            test.runOnMainSync { app.getSystemService(JobScheduler::class.java).cancel(UpdateSchedule.ID) }
            vault.clear(); app.getSharedPreferences("updates", Activity.MODE_PRIVATE).edit().clear().putBoolean("automatic", false).putBoolean("downloadWifi", false).commit()
            val current = UpdateManager(app, network, vault); manager = current
            until("Test updater not restored") { current.status.phase == UpdatePhase.DISCONNECTED }
            test.runOnMainSync { field.set(app, lazy { current }) }
            activity = test.startActivitySync(Intent(app, UpdatesActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as UpdatesActivity
            check("Manual check and token test are visible without saved access") {
                visible("Check for updates"); visible("Test GitHub access")
                require(textContains("GitHub: Not connected") && textContains("Last successful check: Never"))
                snapshot("disconnected")
            }
            check("Disconnected manual check opens masked token verification") {
                click("Check for updates"); enterToken(""); dialogButton("Verify and save")
                require(test.uiAutomation.rootInActiveWindow.findAccessibilityNodeInfosByText("Connect private updates").isNotEmpty())
                dialogButton("Cancel"); require(network.requests.get() == 0)
            }
            check("Rejected token shows failure and is not saved") {
                network.failure = UpdateFailure("GitHub rejected this token. Check that it is valid and has not expired.", true)
                connect(dummy, network, current)
                require(!vault.exists() && current.access.result == UpdateAccessResult.DENIED && current.lastSuccessfulCheck == 0L)
                require(textContains("Connection failed") && textContains("Token was not saved.")); visible("Check for updates"); snapshot("rejected")
            }
            check("Successful token access is explicit even when already up to date") {
                network.failure = null; connect(dummy, network, current)
                require(current.connected && vault.read() == dummy && current.access.result == UpdateAccessResult.VERIFIED && current.lastSuccessfulCheck > 0)
                require(textContains("GitHub: Access verified") && textContains("GitHub access verified. You're up to date."))
                require(!textContains("Last successful check: Never")); visible("Check for updates"); snapshot("verified")
            }
            check("Manual checks run with automatic checks disabled and bypass throttle") {
                require(!current.automatic()); val before = network.requests.get(); click("Check for updates")
                until("Manual check not completed") { network.requests.get() == before + 1 && !current.status.busy }
                require(current.status.phase == UpdatePhase.IDLE && current.access.result == UpdateAccessResult.VERIFIED)
            }
            check("Token access test never downloads an available update") {
                current.prefs.edit().putBoolean("downloadWifi", true).commit()
                val r = UpdateRelease("v0.99.0-dev", "0.99.0-dev", 123, 200, "ab".repeat(32), "QA-only offer")
                network.listing = JSONArray().put(JSONObject().put("tag_name", r.tag).put("draft", false).put("html_url", r.page)
                    .put("assets", JSONArray().put(JSONObject().put("name", "emulator.apk").put("state", "uploaded").put("id", r.asset).put("size", r.size).put("digest", "sha256:${r.sha}").put("url", UpdatePolicy.apiAsset(r.asset))))).toString()
                val before = network.requests.get(); click("Test GitHub access")
                until("Access test did not complete") { network.requests.get() == before + 1 && current.status.phase == UpdatePhase.AVAILABLE }
                require(network.assets.get() == 0 && current.access.result == UpdateAccessResult.VERIFIED && textContains("GitHub access verified."))
                network.listing = "[]"; current.prefs.edit().putBoolean("downloadWifi", false).commit()
            }
            check("In-flight checks disable duplicate controls and show progress") {
                val gate = CountDownLatch(1); network.gate = gate; val before = network.requests.get(); click("Check for updates")
                until("Request did not start") { network.requests.get() == before + 1 && current.status.busy }
                test.runOnMainSync { require(!button("Check for updates").isEnabled && !button("Test GitHub access").isEnabled && !button("Replace GitHub access").isEnabled) }
                require(textContains("Checking for updates…")); gate.countDown(); network.gate = null
                until("Check did not finish") { !current.status.busy }
            }
            check("Network failure preserves token and last verified time without claiming success") {
                val verified = current.access.lastVerifiedAt; val success = current.lastSuccessfulCheck
                network.failure = UpdateFailure("Update could not finish. Check your connection and try again.")
                val before = network.requests.get(); click("Test GitHub access")
                until("Offline failure missing") { network.requests.get() == before + 1 && current.status.phase == UpdatePhase.ERROR }
                require(current.access.result == UpdateAccessResult.ERROR && current.access.lastVerifiedAt == verified && current.lastSuccessfulCheck == success && vault.read() == dummy)
                require(textContains("GitHub: Access could not be checked") && textContains("Last verified:")); snapshot("offline")
            }
            check("Revoked saved token shows access denied and permits retry") {
                network.failure = UpdateFailure("GitHub denied access. Allow the repository with Contents: read-only.", true)
                val before = network.requests.get(); click("Test GitHub access")
                until("Denied access not reflected") { network.requests.get() == before + 1 && current.status.phase == UpdatePhase.ERROR && current.access.result == UpdateAccessResult.DENIED }
                require(vault.read() == dummy && textContains("GitHub: Access denied")); visible("Check for updates"); snapshot("denied")
                network.failure = null; val next = network.requests.get(); click("Test GitHub access")
                until("Retry did not verify") { network.requests.get() == next + 1 && current.access.result == UpdateAccessResult.VERIFIED && !current.status.busy }
            }
            check("Failed replacement keeps existing token and its verified state") {
                val verified = current.access.lastVerifiedAt
                network.failure = UpdateFailure("GitHub rejected the replacement token.", true); connect("ANOTHER_QA_FAKE_TOKEN_NOT_A_CREDENTIAL", network, current)
                require(vault.read() == dummy && current.access.result == UpdateAccessResult.VERIFIED && current.access.lastVerifiedAt == verified && textContains("Existing token was kept."))
                network.failure = null
            }
            check("Verification and check timestamps survive updater recreation") {
                val verified = current.access.lastVerifiedAt; val success = current.lastSuccessfulCheck
                test.runOnMainSync { activity.finish() }
                val restored = UpdateManager(app, network, vault); manager = restored
                until("Restored manager not ready") { restored.status.message != "Preparing updates…" }
                test.runOnMainSync { field.set(app, lazy { restored }) }
                activity = test.startActivitySync(Intent(app, UpdatesActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as UpdatesActivity
                require(restored.access.result == UpdateAccessResult.VERIFIED && restored.access.lastVerifiedAt == verified && restored.lastSuccessfulCheck == success)
                require(textContains("GitHub: Access verified") && textContains("Last verified:")); snapshot("restored")
            }
            check("Manual update controls remain reachable in landscape") {
                val monitor = test.addMonitor(UpdatesActivity::class.java.name, null, false)
                test.runOnMainSync { activity.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE }
                activity = test.waitForMonitorWithTimeout(monitor, 15_000) as UpdatesActivity
                test.removeMonitor(monitor); test.waitForIdleSync()
                require(textContains("GitHub: Access verified")); snapshot("landscape")
                val portrait = test.addMonitor(UpdatesActivity::class.java.name, null, false)
                test.runOnMainSync { activity.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_PORTRAIT }
                activity = test.waitForMonitorWithTimeout(portrait, 15_000) as UpdatesActivity
                test.removeMonitor(portrait); test.waitForIdleSync()
            }
            check("Disconnect clears stored access and verification history") {
                manager!!.disconnect(); until("Disconnect incomplete") { manager!!.status.phase == UpdatePhase.DISCONNECTED }
                require(!vault.exists() && manager!!.access.result == UpdateAccessResult.NONE && manager!!.access.lastVerifiedAt == 0L && manager!!.lastSuccessfulCheck == 0L)
            }
            check("Existing saved tokens are marked unverified until tested") {
                vault.write(dummy); val migrated = UpdateManager(app, network, vault)
                until("Migration restore incomplete") { migrated.status.message != "Preparing updates…" }
                require(migrated.access.result == UpdateAccessResult.SAVED && migrated.access.lastVerifiedAt == 0L)
            }
            val result = JSONObject().put("passed", true).put("checks", JSONArray(checks))
            File(app.filesDir, "access-qa-result.json").writeText(result.toString(2))
            passed = true; stream = result.toString() + "\n"
        } catch (error: Exception) {
            stream = "Access UI QA failed after ${checks.size} checks: ${error.javaClass.simpleName}: ${error.message}\n"
        } finally {
            network.gate?.countDown(); manager?.cancel()
            test.runOnMainSync { if (::activity.isInitialized) activity.finish(); field.set(app, original) }
            vault.clear(); app.getSharedPreferences("updates", Activity.MODE_PRIVATE).edit().clear().commit()
            app.getSystemService(JobScheduler::class.java).cancel(UpdateSchedule.ID)
        }
        test.finish(if (passed) Activity.RESULT_OK else Activity.RESULT_CANCELED, Bundle().apply { putString("stream", stream) })
    }
}
