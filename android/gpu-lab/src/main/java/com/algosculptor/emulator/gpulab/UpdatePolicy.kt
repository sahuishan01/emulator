package com.algosculptor.emulator.gpulab

import java.io.InputStream
import java.io.OutputStream
import java.net.URI
import java.security.MessageDigest
import java.util.concurrent.CancellationException

/** Network-independent rules shared by the downloader, persistence and tests. */
internal object UpdatePolicy {
    const val API = "https://api.github.com/repos/sahuishan01/emulator"
    const val WEB = "https://github.com/sahuishan01/emulator"
    const val MAX_APK = 128L * 1024 * 1024
    const val MAX_METADATA = 1024 * 1024
    data class Version(val major: Long, val minor: Long, val patch: Long, val pre: List<String>) : Comparable<Version> {
        override fun compareTo(other: Version): Int {
            for ((a, b) in listOf(major to other.major, minor to other.minor, patch to other.patch)) if (a != b) return a.compareTo(b)
            if (pre.isEmpty() || other.pre.isEmpty()) return if (pre.isEmpty() == other.pre.isEmpty()) 0 else if (pre.isEmpty()) 1 else -1
            for ((a, b) in pre.zip(other.pre)) {
                val an = a.toLongOrNull(); val bn = b.toLongOrNull()
                val c = if (an != null && bn != null) an.compareTo(bn) else if (an != null) -1 else if (bn != null) 1 else a.compareTo(b)
                if (c != 0) return c
            }
            return pre.size.compareTo(other.pre.size)
        }
    }
    fun version(text: String): Version? {
        val m = Regex("^v?(0|[1-9][0-9]{0,8})\\.(0|[1-9][0-9]{0,8})\\.(0|[1-9][0-9]{0,8})(?:-([0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*))?$").matchEntire(text.takeIf { it.length <= 100 } ?: return null) ?: return null
        val pre = m.groupValues[4].takeIf { it.isNotEmpty() }?.split('.') ?: emptyList()
        if (pre.any { it.all(Char::isDigit) && (it.length > 1 && it[0] == '0' || it.toLongOrNull() == null) }) return null
        return Version(m.groupValues[1].toLong(), m.groupValues[2].toLong(), m.groupValues[3].toLong(), pre)
    }
    fun validDigest(value: String): Boolean = value.matches(Regex("[0-9a-f]{64}"))
    fun apiAsset(id: Long): String { require(id > 0); return "$API/releases/assets/$id" }
    fun releaseUrl(tag: String): String { require(version(tag) != null); return "$WEB/releases/tag/$tag" }
    fun checkUrl(text: String, authenticated: Boolean) {
        val u = try { URI(text) } catch (_: Exception) { throw UpdateFailure("Invalid update address.") }
        if (u.scheme != "https" || u.port !in listOf(-1, 443) || u.rawUserInfo != null || u.rawFragment != null) throw UpdateFailure("Update address is not allowed.")
        if (authenticated) {
            if (u.host != "api.github.com" || !(text == "$API/releases?per_page=30" || Regex("${Regex.escape(API)}/releases/assets/[1-9][0-9]*").matches(text))) throw UpdateFailure("Update address is not allowed.")
        } else if (u.host !in setOf("release-assets.githubusercontent.com", "objects.githubusercontent.com")) throw UpdateFailure("Update redirect is not allowed.")
    }
    fun copyVerified(input: InputStream, output: OutputStream, size: Long, sha: String, cancelled: () -> Boolean, progress: (Long) -> Unit) {
        if (size !in 1..MAX_APK || !validDigest(sha)) throw UpdateFailure("Invalid update size or checksum.")
        val digest = MessageDigest.getInstance("SHA-256"); val buffer = ByteArray(64 * 1024); var total = 0L
        while (true) {
            if (cancelled()) throw CancellationException()
            val n = input.read(buffer); if (n < 0) break
            total += n
            if (total > size) throw UpdateFailure("Update download is larger than expected.")
            output.write(buffer, 0, n); digest.update(buffer, 0, n); progress(total)
        }
        if (cancelled()) throw CancellationException()
        if (total != size) throw UpdateFailure("Update download was interrupted. Try again.")
        if (digest.digest().joinToString("") { "%02x".format(it) } != sha) throw UpdateFailure("Update checksum does not match. The file was discarded.")
    }
}
internal class UpdateFailure(message: String, val accessDenied: Boolean = false) : Exception(message)
