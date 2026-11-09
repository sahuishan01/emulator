package com.algosculptor.emulator.gpulab

import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.CancellationException
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

internal class UpdateTransfer(val background: Boolean = false) {
    val cancelled = AtomicBoolean(false)
    private val connection = AtomicReference<HttpURLConnection?>()
    private val started = System.nanoTime()
    fun bind(value: HttpURLConnection) { connection.set(value); check() }
    fun unbind(value: HttpURLConnection) { connection.compareAndSet(value, null); value.disconnect() }
    fun cancel() { cancelled.set(true); connection.getAndSet(null)?.disconnect() }
    fun check() {
        if (cancelled.get()) throw CancellationException()
        if (System.nanoTime() - started > 5L * 60 * 1_000_000_000) throw UpdateFailure("Update request timed out. Try again.")
    }
}
internal interface UpdateNetwork {
    fun releases(token: String, transfer: UpdateTransfer): String
    fun asset(release: UpdateRelease, token: String, transfer: UpdateTransfer, consume: (InputStream) -> Unit)
    fun downloadUrl(release: UpdateRelease, token: String, transfer: UpdateTransfer): String = throw UpdateFailure("A background download link could not be obtained. Retry or open the release page.")
}
internal class GithubUpdateNetwork(private val open: (URL) -> HttpURLConnection = { it.openConnection() as HttpURLConnection }) : UpdateNetwork {
    private fun connection(url: String, token: String?, binary: Boolean, transfer: UpdateTransfer): HttpURLConnection {
        UpdatePolicy.checkUrl(url, token != null); transfer.check()
        val c = open(URL(url)); c.instanceFollowRedirects = false; c.connectTimeout = 15_000; c.readTimeout = 20_000; c.useCaches = false
        c.setRequestProperty("Accept", if (binary) "application/octet-stream" else "application/vnd.github+json")
        c.setRequestProperty("User-Agent", "OriginalEmulatorUpdater/1")
        c.setRequestProperty("Cache-Control", "no-cache")
        if (token != null) { c.setRequestProperty("Authorization", "Bearer $token"); c.setRequestProperty("X-GitHub-Api-Version", "2022-11-28") }
        transfer.bind(c); return c
    }
    override fun downloadUrl(release: UpdateRelease, token: String, transfer: UpdateTransfer): String {
        val c = connection(UpdatePolicy.apiAsset(release.asset), token, true, transfer)
        try {
            if (c.responseCode in listOf(301,302,303,307,308)) {
                val next = c.getHeaderField("Location") ?: throw UpdateFailure("GitHub did not provide a download link. Retry.")
                UpdatePolicy.checkUrl(next, false); transfer.check(); return next
            }
            checkStatus(c)
            // Never pass a PAT to DownloadManager: it may forward custom headers
            // across redirects. A direct API stream needs the authenticated browser.
            throw UpdateFailure("GitHub returned a direct stream instead of a background link. Open the release page to download this update.")
        } catch (e: java.net.SocketTimeoutException) { throw UpdateFailure("GitHub did not respond in time. Retry the download.") }
        catch (e: java.io.IOException) { throw UpdateFailure("Could not reach GitHub to prepare the download. Check your connection and retry.") }
        finally { transfer.unbind(c) }
    }
    private fun checkStatus(connection: HttpURLConnection) {
        when (connection.responseCode) {
            200 -> Unit
            401 -> throw UpdateFailure("GitHub rejected this token. Check that it is valid and has not expired.", accessDenied = true)
            403 -> if (connection.getHeaderField("X-RateLimit-Remaining") == "0" || connection.getHeaderField("Retry-After") != null) throw UpdateFailure("GitHub rate limit reached. Try again later.")
                else throw UpdateFailure("GitHub denied access. Allow sahuishan01/emulator with Contents: read-only in the token.", accessDenied = true)
            404 -> throw UpdateFailure("Private releases were not found. Allow sahuishan01/emulator in the token's repository access.", accessDenied = true)
            429 -> throw UpdateFailure("GitHub rate limit reached. Try again later.")
            else -> throw UpdateFailure("Update server is unavailable. Try again later.")
        }
    }
    override fun releases(token: String, transfer: UpdateTransfer): String {
        val c = connection("${UpdatePolicy.API}/releases?per_page=30", token, false, transfer)
        try {
            checkStatus(c)
            if (c.contentLengthLong > UpdatePolicy.MAX_METADATA) throw UpdateFailure("Update response is too large.")
            val bytes = c.inputStream.use { input ->
                val output = java.io.ByteArrayOutputStream(); val buffer = ByteArray(8192)
                while (true) { transfer.check(); val n = input.read(buffer); if (n < 0) break; if (output.size() + n > UpdatePolicy.MAX_METADATA) throw UpdateFailure("Update response is too large."); output.write(buffer, 0, n) }; output.toByteArray()
            }
            return String(bytes, Charsets.UTF_8)
        } finally { transfer.unbind(c) }
    }
    override fun asset(release: UpdateRelease, token: String, transfer: UpdateTransfer, consume: (InputStream) -> Unit) {
        var url = UpdatePolicy.apiAsset(release.asset); var credential: String? = token
        repeat(4) {
            val c = connection(url, credential, true, transfer)
            try {
                val status = c.responseCode
                if (status in listOf(301, 302, 303, 307, 308)) {
                    val next = c.getHeaderField("Location") ?: throw UpdateFailure("Update redirect is missing.")
                    UpdatePolicy.checkUrl(next, false); url = next; credential = null
                } else {
                    checkStatus(c)
                    if (c.contentLengthLong >= 0 && c.contentLengthLong != release.size) throw UpdateFailure("Update download size changed. Check again.")
                    c.inputStream.use(consume); return
                }
            } finally { transfer.unbind(c) }
        }
        throw UpdateFailure("Too many update redirects.")
    }
}
