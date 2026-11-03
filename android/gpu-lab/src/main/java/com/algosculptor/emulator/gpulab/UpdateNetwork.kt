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
}
internal class GithubUpdateNetwork(private val open: (URL) -> HttpURLConnection = { it.openConnection() as HttpURLConnection }) : UpdateNetwork {
    private fun connection(url: String, token: String?, binary: Boolean, transfer: UpdateTransfer): HttpURLConnection {
        UpdatePolicy.checkUrl(url, token != null); transfer.check()
        val c = open(URL(url)); c.instanceFollowRedirects = false; c.connectTimeout = 15_000; c.readTimeout = 20_000; c.useCaches = false
        c.setRequestProperty("Accept", if (binary) "application/octet-stream" else "application/vnd.github+json")
        c.setRequestProperty("User-Agent", "OriginalEmulatorUpdater/1")
        if (token != null) { c.setRequestProperty("Authorization", "Bearer $token"); c.setRequestProperty("X-GitHub-Api-Version", "2022-11-28") }
        transfer.bind(c); return c
    }
    private fun checkStatus(code: Int) {
        when (code) {
            200 -> Unit
            401, 403, 404 -> throw UpdateFailure("Cannot access private releases. Check the token's repository access and Contents read permission.")
            429 -> throw UpdateFailure("GitHub rate limit reached. Try again later.")
            else -> throw UpdateFailure("Update server is unavailable. Try again later.")
        }
    }
    override fun releases(token: String, transfer: UpdateTransfer): String {
        val c = connection("${UpdatePolicy.API}/releases?per_page=30", token, false, transfer)
        try {
            checkStatus(c.responseCode)
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
                    checkStatus(status)
                    if (c.contentLengthLong >= 0 && c.contentLengthLong != release.size) throw UpdateFailure("Update download size changed. Check again.")
                    c.inputStream.use(consume); return
                }
            } finally { transfer.unbind(c) }
        }
        throw UpdateFailure("Too many update redirects.")
    }
}
