package com.algosculptor.emulator.gpulab

import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

internal class ReportUploadFailure(message: String, val blocked: Boolean = false, val retryAt: Long = 0) : Exception(message)
internal data class ReportReceipt(val issue: Long, val artifact: Long) {
    val url: String get() = "${ReportUploadPolicy.WEB}/issues/$issue/artifacts/$artifact"
}
internal interface ReportUploadNetwork {
    fun verifyAccess(token: String, transfer: UpdateTransfer)
    fun upload(token: String, report: UploadReport, issue: Long, transfer: UpdateTransfer, saveIssue: (Long) -> Unit): ReportReceipt
}
internal class GithubReportNetwork(private val open: (URL) -> HttpURLConnection = { it.openConnection() as HttpURLConnection }) : ReportUploadNetwork {
    private fun request(method: String, path: String, token: String, transfer: UpdateTransfer, payload: JSONObject? = null): String {
        require(method in listOf("GET", "POST"))
        require(path == "" || path == "/issues" || path == "/issues?state=all&sort=created&direction=desc&per_page=25" ||
            path.matches(Regex("/issues/[1-9][0-9]*(/artifacts(/[1-9][0-9]*)?|/comments)?")))
        transfer.check(); val c = open(URL(ReportUploadPolicy.API + path))
        c.requestMethod = method; c.instanceFollowRedirects = false; c.connectTimeout = 15_000; c.readTimeout = 20_000; c.useCaches = false
        c.setRequestProperty("Accept", "application/vnd.github+json"); c.setRequestProperty("X-GitHub-Api-Version", "2026-03-10")
        c.setRequestProperty("Authorization", "Bearer $token"); c.setRequestProperty("User-Agent", "OriginalEmulatorReports/1")
        transfer.bind(c)
        try {
            if (payload != null) {
                val data = payload.toString().toByteArray(Charsets.UTF_8); require(data.size <= 7 * ReportUploadPolicy.MAX_BYTES)
                c.doOutput = true; c.setRequestProperty("Content-Type", "application/json; charset=utf-8"); c.setFixedLengthStreamingMode(data.size)
                c.outputStream.use { it.write(data) }
            }
            val code = c.responseCode
            if (code == 429 || code == 403 && (c.getHeaderField("X-RateLimit-Remaining") == "0" || c.getHeaderField("Retry-After") != null)) {
                val now = System.currentTimeMillis()
                val seconds = c.getHeaderField("Retry-After")?.toLongOrNull()?.coerceIn(1, 86400) ?: 60L
                val reset = c.getHeaderField("X-RateLimit-Reset")?.toLongOrNull()?.let { it.coerceIn(now / 1000, now / 1000 + 86400) * 1000 } ?: 0
                throw ReportUploadFailure("GitHub asked uploads to wait. The report remains queued.", retryAt = maxOf(now + seconds * 1000, reset))
            }
            if (code in listOf(401, 403)) throw ReportUploadFailure("Report upload needs Issues: read and write for sahuishan01/emulator. Check the token, then test upload access.", blocked = true)
            if (code in listOf(404, 410)) throw ReportUploadFailure("The private report destination was not found. Check repository access.", blocked = true)
            if (code !in (if (method == "POST") listOf(201) else listOf(200))) throw ReportUploadFailure("GitHub could not save the report. It remains queued.")
            if (c.contentLengthLong > 8L * ReportUploadPolicy.MAX_BYTES) throw ReportUploadFailure("GitHub's response exceeded the upload limit.")
            return c.inputStream.use { input ->
                val output = java.io.ByteArrayOutputStream(); val buffer = ByteArray(16384)
                while (true) { transfer.check(); val n = input.read(buffer); if (n < 0) break; if (output.size() + n > 8 * ReportUploadPolicy.MAX_BYTES) throw ReportUploadFailure("GitHub's response exceeded the upload limit."); output.write(buffer, 0, n) }
                output.toString(Charsets.UTF_8.name())
            }
        } finally { transfer.unbind(c) }
    }
    private fun verifyDestination(token: String, transfer: UpdateTransfer) {
        val repo = JSONObject(request("GET", "", token, transfer))
        if (repo.optString("full_name") != ReportUploadPolicy.REPO || !repo.optBoolean("has_issues"))
            throw ReportUploadFailure("Reports are only uploaded to the emulator repository with Issues enabled.", blocked = true)
        val inbox = JSONObject(request("GET", "/issues/${ReportUploadPolicy.INBOX}", token, transfer))
        if (inbox.optLong("number") != ReportUploadPolicy.INBOX || !inbox.optString("body").contains("<!-- emulator-report-inbox:v1 -->"))
            throw ReportUploadFailure("The report inbox could not be verified.", blocked = true)
    }
    override fun verifyAccess(token: String, transfer: UpdateTransfer) {
        verifyDestination(token, transfer)
        val body = "<!-- emulator-report-access:v1 -->\nPrivate Android report upload access verified. No diagnostic data was uploaded by this access test."
        val response = JSONObject(request("POST", "/issues/${ReportUploadPolicy.INBOX}/comments", token, transfer, JSONObject().put("body", body)))
        require(response.getLong("id") > 0 && response.getString("body") == body)
    }
    override fun upload(token: String, report: UploadReport, issue: Long, transfer: UpdateTransfer, saveIssue: (Long) -> Unit): ReportReceipt {
        verifyDestination(token, transfer)
        var number = issue
        if (number == 0L) {
            val recent = JSONArray(request("GET", "/issues?state=all&sort=created&direction=desc&per_page=25", token, transfer))
            for (i in 0 until recent.length()) { val value = recent.getJSONObject(i); if (!value.has("pull_request") && value.optString("body").startsWith(report.marker + "\n")) { number = value.getLong("number"); break } }
            if (number == 0L) number = JSONObject(request("POST", "/issues", token, transfer, JSONObject().put("title", report.title).put("body", report.issueBody))).getLong("number")
            require(number > 0); saveIssue(number)
        }
        val existing = JSONObject(request("GET", "/issues/$number", token, transfer))
        require(existing.getLong("number") == number && existing.getString("body").startsWith(report.marker + "\n"))
        // This issue contains one logical report. Recovery can safely inspect its
        // artifact list without reading every report ever uploaded to the inbox.
        val artifacts = JSONArray(request("GET", "/issues/$number/artifacts", token, transfer))
        var artifact = 0L
        for (i in 0 until artifacts.length()) { val value = artifacts.getJSONObject(i); if (value.optString("name") == report.artifactName) { artifact = value.getLong("number"); break } }
        if (artifact == 0L) {
            val response = JSONObject(request("POST", "/issues/$number/artifacts", token, transfer, JSONObject().put("type", "generic").put("name", report.artifactName).put("body", report.text)))
            artifact = response.getLong("number"); require(artifact > 0)
        }
        val stored = JSONObject(request("GET", "/issues/$number/artifacts/$artifact", token, transfer)).getJSONObject("artifact")
        require(stored.getString("name") == report.artifactName && stored.getLong("number") == artifact)
        val raw = stored.getString("body").toByteArray(Charsets.UTF_8)
        if (!raw.contentEquals(report.bytes) || ReportUploadPolicy.sha(raw) != report.sha) throw ReportUploadFailure("GitHub's saved report did not match. The original remains queued.")
        return ReportReceipt(number, artifact)
    }
}
