package com.algosculptor.emulator.gpulab

import org.json.JSONArray
import org.json.JSONObject

internal data class UpdateRelease(val tag: String, val version: String, val asset: Long, val size: Long, val sha: String, val notes: String) {
    val page: String get() = UpdatePolicy.releaseUrl(tag)
    fun json(): String = JSONObject().put("tag", tag).put("version", version).put("asset", asset).put("size", size).put("sha", sha).put("notes", notes).toString()
    companion object {
        fun restore(text: String): UpdateRelease {
            val x = JSONObject(text); val r = UpdateRelease(x.getString("tag"), x.getString("version"), x.getLong("asset"), x.getLong("size"), x.getString("sha"), x.optString("notes").take(2000))
            require(UpdatePolicy.version(r.tag) != null && r.version == r.tag.removePrefix("v") && r.asset > 0 && r.size in 1..UpdatePolicy.MAX_APK && UpdatePolicy.validDigest(r.sha)); return r
        }
        fun latest(text: String, installed: String): UpdateRelease? {
            val current = UpdatePolicy.version(installed) ?: throw UpdateFailure("Installed version cannot be compared.")
            val all = JSONArray(text); if (all.length() > 30) throw UpdateFailure("Update response contains too many releases.")
            val candidates = (0 until all.length()).map { all.getJSONObject(it) }.filter { !it.optBoolean("draft", true) && UpdatePolicy.version(it.optString("tag_name"))?.let { v -> v > current } == true }
            val release = candidates.maxByOrNull { UpdatePolicy.version(it.getString("tag_name"))!! } ?: return null
            val tag = release.getString("tag_name"); val matches = (release.optJSONArray("assets") ?: JSONArray()).let { a -> (0 until a.length()).map { a.getJSONObject(it) }.filter { it.optString("name") == "emulator.apk" } }
            if (matches.size != 1) throw UpdateFailure("New release does not contain one verified APK yet. Try again later.")
            val asset = matches.single(); val id = asset.optLong("id"); val size = asset.optLong("size"); val digest = asset.optString("digest")
            if (asset.optString("state") != "uploaded" || id <= 0 || size !in 1..UpdatePolicy.MAX_APK || !digest.startsWith("sha256:") || !UpdatePolicy.validDigest(digest.removePrefix("sha256:")) || asset.optString("url") != UpdatePolicy.apiAsset(id)) throw UpdateFailure("New release is missing a valid APK checksum or size.")
            if (release.optString("html_url") != UpdatePolicy.releaseUrl(tag)) throw UpdateFailure("Release address does not match the update source.")
            return UpdateRelease(tag, tag.removePrefix("v"), id, size, digest.removePrefix("sha256:"), release.optString("body").take(2000))
        }
    }
}
