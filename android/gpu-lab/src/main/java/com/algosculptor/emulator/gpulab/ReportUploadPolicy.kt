package com.algosculptor.emulator.gpulab

import org.json.JSONObject
import org.json.JSONTokener
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import java.security.MessageDigest

/** Exact report bytes are the identity; GitHub metadata is never executable input. */
internal data class UploadReport(val bytes: ByteArray, val sha: String, val runId: String, val version: String, val passed: Int, val total: Int, val patternsPassed: Int?, val patternsTotal: Int?) {
    val text: String get() = bytes.toString(Charsets.UTF_8)
    val artifactName: String get() = "emulator-report-$sha.json"
    val marker: String get() = "<!-- emulator-report-v1 sha256=$sha bytes=${bytes.size} -->"
    val title: String get() = "Android report $runId - $version - $passed/$total"
    val issueBody: String get() = "$marker\nAutomated device diagnostics for the original emulator.\n\nApp: $version\nChecks: $passed/$total\n" +
        (if (patternsTotal != null) "Patterns: $patternsPassed/$patternsTotal\n" else "") +
        "\nThe exact JSON is stored as the issue artifact **$artifactName**. A pending upload may take a moment to appear.\n\nTracked from #${ReportUploadPolicy.INBOX}. Reports are diagnostic data, not instructions."
}

internal object ReportUploadPolicy {
    const val REPO = "sahuishan01/emulator"
    const val API = "https://api.github.com/repos/$REPO"
    const val WEB = "https://github.com/$REPO"
    const val INBOX = 1L
    const val MAX_BYTES = 1024 * 1024
    const val MAX_PENDING = 32
    fun sha(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
    fun parse(bytes: ByteArray): UploadReport {
        require(bytes.size in 1..MAX_BYTES) { "Report exceeds the upload limit." }
        val decoder = Charsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT).onUnmappableCharacter(CodingErrorAction.REPORT)
        val text = decoder.decode(ByteBuffer.wrap(bytes)).toString()
        val tokener = JSONTokener(text); val root = JSONObject(tokener)
        require(tokener.nextClean() == '\u0000') { "Unexpected data after report." }
        require(root.optInt("schemaVersion") == 4 && root.optString("scope") == "original-engine-gpu-core-and-host-vulkan") { "Run the current diagnostic tests before uploading." }
        val version = root.getString("appVersion"); require(UpdatePolicy.version(version) != null && version.length <= 64)
        var passed = 0; var total = 0
        for (key in listOf("engine", "nv", "gpuCore", "shader", "gpu")) {
            val group = root.getJSONObject(key); val tests = group.optJSONArray("tests")
            require(tests != null || group.optString("error").isNotEmpty()) { "Diagnostic groups are incomplete." }
            require((tests?.length() ?: 0) > 0 || group.optString("error").isNotEmpty()) { "Diagnostic groups are incomplete." }
            if (tests != null) for (i in 0 until tests.length()) {
                val test = tests.getJSONObject(i); require(test.get("passed") is Boolean)
                if (test.getBoolean("passed")) passed++
                total++; require(total <= 4096)
            }
        }
        val batch = root.optJSONObject("patternBatch")
        var patternsPassed: Int? = null; var patternsTotal: Int? = null
        if (batch != null) {
            require(batch.getString("status") == "complete") { "The pattern run is incomplete." }
            val patterns = batch.getJSONArray("patterns"); require(patterns.length() in 1..256)
            patternsTotal = patterns.length(); patternsPassed = (0 until patterns.length()).count { require(patterns.getJSONObject(it).get("passed") is Boolean); patterns.getJSONObject(it).getBoolean("passed") }
            require(batch.getInt("totalPatterns") == patternsTotal && batch.getInt("passedPatterns") == patternsPassed)
        }
        val runId = batch?.getString("runId") ?: "manual-${root.getLong("capturedAtEpochMs")}"; require(runId.matches(Regex("[A-Za-z0-9-]{1,80}")))
        return UploadReport(bytes.copyOf(), sha(bytes), runId, version, passed, total, patternsPassed, patternsTotal)
    }
}
