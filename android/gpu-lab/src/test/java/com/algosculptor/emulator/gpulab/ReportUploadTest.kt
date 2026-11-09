package com.algosculptor.emulator.gpulab

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.net.HttpURLConnection
import java.net.URL

class ReportUploadTest {
    private fun root(): JSONObject {
        val value = JSONObject().put("schemaVersion", 4).put("scope", "original-engine-gpu-core-and-host-vulkan")
            .put("appVersion", "0.27.4-dev").put("capturedAtEpochMs", 123).put("allPassed", false).put("phoneModel", "QA हिन्दी")
        for (key in listOf("engine", "nv", "gpuCore", "shader", "gpu")) value.put(key, JSONObject().put("tests", JSONArray().put(JSONObject().put("name", "owned-test").put("passed", key != "gpu"))).put("allPassed", key != "gpu"))
        return value
    }
    private fun report() = ReportUploadPolicy.parse(root().toString(2).toByteArray(Charsets.UTF_8))
    private fun rejected(run: () -> Unit) { var failed = false; try { run() } catch (_: Exception) { failed = true }; assertTrue(failed) }
    @Test fun exactUtf8AndWhitespaceArePreserved() {
        val bytes = root().toString(2).toByteArray(Charsets.UTF_8); val doc = ReportUploadPolicy.parse(bytes)
        assertArrayEquals(bytes, doc.bytes); assertEquals(ReportUploadPolicy.sha(bytes), doc.sha)
        assertNotEquals(doc.sha, ReportUploadPolicy.parse(root().toString().toByteArray()).sha)
        assertEquals(4, doc.passed); assertEquals(5, doc.total); assertEquals("manual-123", doc.runId)
    }
    @Test fun failedCompletedPatternReportsAreAccepted() {
        val value = root().put("patternBatch", JSONObject().put("status", "complete").put("runId", "123-abcd").put("totalPatterns", 2).put("passedPatterns", 1)
            .put("patterns", JSONArray().put(JSONObject().put("passed", true)).put(JSONObject().put("passed", false))))
        val doc = ReportUploadPolicy.parse(value.toString().toByteArray()); assertEquals(1, doc.patternsPassed); assertEquals(2, doc.patternsTotal)
    }
    @Test fun unsupportedGpuInitializationErrorsCanStillBeUploaded() {
        val value = root().put("gpu", JSONObject().put("tests", JSONArray()).put("error", "No Vulkan device").put("allPassed", false))
        assertEquals(4, ReportUploadPolicy.parse(value.toString().toByteArray()).total)
    }
    @Test fun invalidAndIncompleteReportsAreRejected() {
        for (key in listOf("engine", "nv", "gpuCore", "shader", "gpu")) { val value = root(); value.remove(key); rejected { ReportUploadPolicy.parse(value.toString().toByteArray()) } }
        rejected { ReportUploadPolicy.parse(root().put("schemaVersion", 3).toString().toByteArray()) }
        rejected { ReportUploadPolicy.parse(root().put("scope", "a-file").toString().toByteArray()) }
        rejected { ReportUploadPolicy.parse(root().put("gpu", JSONObject().put("tests", JSONArray())).toString().toByteArray()) }
    }
    @Test fun sizeUtf8AndTrailingDataLimitsFailClosed() {
        rejected { ReportUploadPolicy.parse(ByteArray(ReportUploadPolicy.MAX_BYTES + 1)) }
        rejected { ReportUploadPolicy.parse(byteArrayOf(0xc3.toByte(), 0x28)) }
        rejected { ReportUploadPolicy.parse(root().toString().toByteArray() + "{}".toByteArray()) }
        rejected { ReportUploadPolicy.parse(byteArrayOf()) }
    }
    @Test fun incompletePatternsAndUnsafeIdentifiersAreRejected() {
        for (id in listOf("../outside", "a\nb", "x".repeat(81))) {
            val value = root().put("patternBatch", JSONObject().put("status", "complete").put("runId", id).put("totalPatterns", 1).put("passedPatterns", 1).put("patterns", JSONArray().put(JSONObject().put("passed", true))))
            rejected { ReportUploadPolicy.parse(value.toString().toByteArray()) }
        }
        val value = root().put("patternBatch", JSONObject().put("status", "running")); rejected { ReportUploadPolicy.parse(value.toString().toByteArray()) }
    }
    private class Connection(url: URL, val code: Int, val body: String, val retry: String? = null, val advertised: Long? = null) : HttpURLConnection(url) {
        val written = ByteArrayOutputStream(); var closed = false
        override fun connect() {}
        override fun disconnect() { closed = true }
        override fun usingProxy() = false
        override fun getResponseCode() = code
        override fun getContentLengthLong() = advertised ?: body.toByteArray().size.toLong()
        override fun getInputStream() = ByteArrayInputStream(body.toByteArray())
        override fun getOutputStream() = written
        override fun getHeaderField(name: String): String? = if (name == "Retry-After") retry else null
    }
    private val repo = JSONObject().put("full_name", ReportUploadPolicy.REPO).put("private", true).put("has_issues", true).toString()
    private val inbox = JSONObject().put("number", 1).put("body", "<!-- emulator-report-inbox:v1 -->").toString()
    @Test fun accessTestRequiresPrivateDestinationAndActualWriteAcknowledgment() {
        val calls = mutableListOf<Connection>()
        val network = GithubReportNetwork { url ->
            val body = when (calls.size) { 0 -> repo; 1 -> inbox; else -> JSONObject().put("id", 1).put("body", "<!-- emulator-report-access:v1 -->\nPrivate Android report upload access verified. No diagnostic data was uploaded by this access test.").toString() }
            Connection(url, if (calls.size == 2) 201 else 200, body).also { calls.add(it) }
        }
        network.verifyAccess("QA_FAKE_TOKEN", UpdateTransfer())
        assertEquals(listOf("GET", "GET", "POST"), calls.map { it.requestMethod }); assertTrue(calls.all { it.closed })
        assertTrue(calls.all { it.url.host == "api.github.com" && it.getRequestProperty("Authorization") == "Bearer QA_FAKE_TOKEN" })
    }
    @Test fun wrongRepositoryNeverReceivesAReportOrProbe() {
        var count = 0
        val network = GithubReportNetwork { url -> count++; Connection(url, 200, JSONObject(repo).put("full_name", "other/repo").toString()) }
        rejected { network.verifyAccess("QA_FAKE_TOKEN", UpdateTransfer()) }; assertEquals(1, count)
    }
    @Test fun permissionsLimitsRedirectsAndOversizedResponsesAreControlled() {
        for (code in listOf(401, 403, 404, 410, 429, 302, 500)) {
            val connection = Connection(URL(ReportUploadPolicy.API), code, "QA_FAKE_TOKEN", if (code == 429) "120" else null)
            try { GithubReportNetwork { connection }.verifyAccess("QA_FAKE_TOKEN", UpdateTransfer()); fail() }
            catch (e: ReportUploadFailure) {
                assertFalse(e.message!!.contains("QA_FAKE_TOKEN"))
                if (code == 429) assertTrue(e.retryAt >= System.currentTimeMillis() + 100_000)
                if (code in listOf(401, 403, 404, 410)) assertTrue(e.blocked)
            }
            assertTrue(connection.closed); assertFalse(connection.instanceFollowRedirects)
        }
        val huge = Connection(URL(ReportUploadPolicy.API), 200, "{}", advertised = 9L * ReportUploadPolicy.MAX_BYTES)
        rejected { GithubReportNetwork { huge }.verifyAccess("QA_FAKE_TOKEN", UpdateTransfer()) }; assertTrue(huge.closed)
    }
    private fun upload(existing: Boolean, corrupt: Boolean = false): Pair<ReportReceipt?, List<Connection>> {
        val doc = report(); val calls = mutableListOf<Connection>(); var callback = 0L
        val responses = mutableListOf(200 to repo, 200 to inbox)
        if (!existing) responses += listOf(200 to "[]", 201 to "{\"number\":2}")
        responses += 200 to JSONObject().put("number", 2).put("body", doc.issueBody).toString()
        responses += 200 to (if (existing) JSONArray().put(JSONObject().put("number", 1).put("name", doc.artifactName)).toString() else "[]")
        if (!existing) responses += 201 to "{\"number\":1}"
        responses += 200 to JSONObject().put("artifact", JSONObject().put("number", 1).put("name", doc.artifactName).put("body", if (corrupt) "{}" else doc.text)).toString()
        val network = GithubReportNetwork { url -> val next = responses.removeAt(0); Connection(url, next.first, next.second).also { calls.add(it) } }
        var receipt: ReportReceipt? = null
        if (corrupt) rejected { network.upload("QA_FAKE_TOKEN", doc, if (existing) 2 else 0, UpdateTransfer()) { callback = it } }
        else receipt = network.upload("QA_FAKE_TOKEN", doc, if (existing) 2 else 0, UpdateTransfer()) { callback = it }
        assertTrue(responses.isEmpty()); assertTrue(calls.all { it.closed }); if (!existing) assertEquals(2, callback)
        return receipt to calls
    }
    @Test fun fullUploadCreatesIssueThenArtifactAndVerifiesExactReadback() {
        val (receipt, calls) = upload(false)
        assertEquals("${ReportUploadPolicy.WEB}/issues/2/artifacts/1", receipt!!.url)
        val sent = calls.filter { it.requestMethod == "POST" }; assertEquals(2, sent.size)
        assertEquals(report().text, JSONObject(sent.last().written.toString("UTF-8")).getString("body"))
    }
    @Test fun retryOfKnownIssueAndArtifactDoesNotCreateDuplicates() { val (receipt, calls) = upload(true); assertNotNull(receipt); assertTrue(calls.none { it.requestMethod == "POST" }) }
    @Test fun corruptedReadbackNeverAcknowledgesUpload() { assertNull(upload(false, corrupt = true).first) }
}
