package com.algosculptor.emulator.gpulab

import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.CancellationException
import org.json.JSONArray
import org.json.JSONObject

class UpdatePolicyTest {
    private val data = ByteArray(131073) { (it * 17).toByte() }
    private val sha get() = MessageDigest.getInstance("SHA-256").digest(data).joinToString("") { "%02x".format(it) }
    private fun release(tag: String = "v0.28.0-dev"): JSONObject = JSONObject().put("draft", false).put("tag_name", tag).put("html_url", UpdatePolicy.releaseUrl(tag)).put("body", "Changes").put("assets", JSONArray().put(JSONObject().put("name", "emulator.apk").put("id", 123).put("state", "uploaded").put("size", data.size).put("digest", "sha256:$sha").put("url", UpdatePolicy.apiAsset(123))))
    private fun fails(action: () -> Unit) { try { action(); fail("Expected rejection") } catch (_: UpdateFailure) {} }
    private fun parse(value: JSONObject) = UpdateRelease.latest(JSONArray().put(value).toString(), "0.27.1-dev")
    @Test fun semanticOrderingIncludesDevelopmentReleases() {
        val ordered = listOf("0.9.9", "0.27.0-dev", "0.27.1-dev", "0.27.1-dev.1", "0.27.1-dev.2", "0.27.1-dev.10", "0.27.1", "0.28.0-dev", "1.0.0")
        ordered.zipWithNext().forEach { (a,b) -> assertTrue(UpdatePolicy.version(a)!! < UpdatePolicy.version(b)!!) }
        assertEquals(UpdatePolicy.version("v1.0.0"), UpdatePolicy.version("1.0.0"))
    }
    @Test fun malformedVersionsCannotOfferAnUpdate() { listOf("1", "01.2.3", "1.02.3", "1.2.03", "1.2.3-01", "1.2.3-", "1.2.3/a", "1.2.3+build", "9999999999.2.3", "1.2.3-9999999999999999999999").forEach { assertNull(it, UpdatePolicy.version(it)) } }
    @Test fun newestReleaseIsChosenInsteadOfApiOrder() { val a = JSONArray().put(release("v0.27.2-dev")).put(release("v0.29.0-dev")).put(release("v0.28.0-dev")); assertEquals("0.29.0-dev", UpdateRelease.latest(a.toString(),"0.27.1-dev")!!.version) }
    @Test fun currentAndOlderVersionsAreNeverOffered() { assertNull(UpdateRelease.latest(JSONArray().put(release("v0.27.1-dev")).put(release("v0.27.0-dev")).toString(), "0.27.1-dev")) }
    @Test fun draftsAreNeverOffered() { assertNull(parse(release().put("draft", true))) }
    @Test fun releasesMustHaveOneExactApk() { fails { parse(release().put("assets", JSONArray())) }; val r=release();r.getJSONArray("assets").put(r.getJSONArray("assets").getJSONObject(0)); fails { parse(r) } }
    @Test fun absentInvalidOrWrongAlgorithmDigestRejectsNewRelease() { for (digest in listOf("", "sha1:$sha", "sha256:abc", "sha256:"+"A".repeat(64))) { val r=release();r.getJSONArray("assets").getJSONObject(0).put("digest",digest);fails{parse(r)} } }
    @Test fun wrongAssetStateSizeIdAndAddressFailClosed() { for ((key,value) in listOf("state" to "new", "size" to 0, "size" to UpdatePolicy.MAX_APK+1, "id" to 0, "url" to "https://evil.invalid/apk")) { val r=release();r.getJSONArray("assets").getJSONObject(0).put(key,value);fails { parse(r) } } }
    @Test fun releaseWebAddressCannotBeReplaced() { fails { parse(release().put("html_url", "https://evil.invalid")) } }
    @Test fun cachedReleaseMetadataHasStrictRoundTrip() { val r=parse(release())!!;assertEquals(r,UpdateRelease.restore(r.json()));try { UpdateRelease.restore(JSONObject(r.json()).put("sha","bad").toString());fail() }catch (_: IllegalArgumentException){} }
    @Test fun exactStreamCopiesAndVerifiesAllBytes() { val out=ByteArrayOutputStream();var last=0L;UpdatePolicy.copyVerified(ByteArrayInputStream(data),out,data.size.toLong(),sha,{false}){last=it};assertArrayEquals(data,out.toByteArray());assertEquals(data.size.toLong(),last) }
    @Test fun truncatedStreamIsRejected() { fails { UpdatePolicy.copyVerified(ByteArrayInputStream(data.copyOf(42)),ByteArrayOutputStream(),data.size.toLong(),sha,{false},{}) } }
    @Test fun oversizedStreamIsRejectedBeforeExtraBytesAreWritten() { val out=ByteArrayOutputStream();fails { UpdatePolicy.copyVerified(ByteArrayInputStream(data),out,10,sha,{false},{}) };assertEquals(0,out.size()) }
    @Test fun corruptedStreamIsRejected() { fails { UpdatePolicy.copyVerified(ByteArrayInputStream(data.apply { this[7]=(this[7].toInt() xor 1).toByte() }),ByteArrayOutputStream(),131073,shaOfOriginal(),{false},{}) } }
    private fun shaOfOriginal(): String = MessageDigest.getInstance("SHA-256").digest(ByteArray(131073) { (it*17).toByte() }).joinToString(""){"%02x".format(it)}
    @Test fun cancellationAbortsBeforePublishing() { try { UpdatePolicy.copyVerified(ByteArrayInputStream(data),ByteArrayOutputStream(),data.size.toLong(),sha,{true},{});fail() } catch (_: CancellationException) {} }
    @Test fun invalidDownloadSizesAndChecksumsAreRejected() { for (size in listOf(-1L,0L,UpdatePolicy.MAX_APK+1)) fails { UpdatePolicy.copyVerified(ByteArrayInputStream(data),ByteArrayOutputStream(),size,sha,{false},{}) };fails { UpdatePolicy.copyVerified(ByteArrayInputStream(data),ByteArrayOutputStream(),1,"invalid",{false},{}) } }
    @Test fun credentialsAreRestrictedToExactRepositoryApiRoutes() { for (url in listOf("http://api.github.com/repos/sahuishan01/emulator/releases?per_page=30","https://api.github.com.evil.invalid/repos/sahuishan01/emulator/releases?per_page=30","https://api.github.com/repos/other/emulator/releases?per_page=30","https://user:password@api.github.com/repos/sahuishan01/emulator/releases?per_page=30","https://api.github.com:8443/repos/sahuishan01/emulator/releases?per_page=30",UpdatePolicy.apiAsset(123)+"#fragment",UpdatePolicy.apiAsset(123)+"?other=1","https://127.0.0.1/a")) fails { UpdatePolicy.checkUrl(url,true) } }
    @Test fun cdnRedirectsRequireHttpsAndApprovedHostWithoutCredentials() { for(url in listOf("https://evil.invalid/a","http://release-assets.githubusercontent.com/a","https://user@release-assets.githubusercontent.com/a","https://release-assets.githubusercontent.com:444/a","https://api.github.com/a","https://release-assets.githubusercontent.com.evil.invalid/a")) fails { UpdatePolicy.checkUrl(url,false) };UpdatePolicy.checkUrl("https://release-assets.githubusercontent.com/a?signature=test",false) }
    private class Connection(url:URL,val code:Int,val bytes:ByteArray=byteArrayOf(),val location:String?=null,val remaining:String?=null,val retry:String?=null):HttpURLConnection(url) {
        var closed=false
        override fun connect(){};override fun disconnect(){closed=true};override fun usingProxy()=false
        override fun getResponseCode()=code
        override fun getHeaderField(name:String):String?=when(name){"Location"->location;"X-RateLimit-Remaining"->remaining;"Retry-After"->retry;else->null}
        override fun getContentLengthLong()=if(code==200)bytes.size.toLong() else -1
        override fun getInputStream()=ByteArrayInputStream(bytes)
    }
    @Test fun tokenIsRemovedBeforeFollowingAssetRedirect() {
        val calls=mutableListOf<Connection>();val net=GithubUpdateNetwork{url->Connection(url,if(calls.isEmpty())302 else 200,if(calls.isEmpty())byteArrayOf() else data,"https://release-assets.githubusercontent.com/asset?signature=test").also{calls.add(it)}}
        val r=parse(release())!!;net.asset(r,"QA_DUMMY_ACCESS_TOKEN",UpdateTransfer()){assertArrayEquals(data,it.readBytes())}
        assertEquals(2,calls.size);assertEquals("Bearer QA_DUMMY_ACCESS_TOKEN",calls[0].getRequestProperty("Authorization"));assertNull(calls[1].getRequestProperty("Authorization"));assertTrue(calls.all{it.closed})
    }
    @Test fun untrustedRedirectNeverReceivesARequest() { var count=0;val net=GithubUpdateNetwork{url->count++;Connection(url,302,location="https://evil.invalid/asset")};fails{net.asset(parse(release())!!,"QA_DUMMY_ACCESS_TOKEN",UpdateTransfer()) {fail()}};assertEquals(1,count) }
    @Test fun metadataRedirectsAreRejectedAndConnectionCloses() { val c=Connection(URL(UpdatePolicy.API),302,location="https://evil.invalid");fails{GithubUpdateNetwork{c}.releases("QA_DUMMY_ACCESS_TOKEN",UpdateTransfer())};assertTrue(c.closed) }
    @Test fun unauthorizedAndMissingReleasesUseControlledErrors() { for(code in listOf(401,403,404,429,500,206)){val c=Connection(URL(UpdatePolicy.API),code);fails{GithubUpdateNetwork{c}.releases("QA_DUMMY_ACCESS_TOKEN",UpdateTransfer())};assertTrue(c.closed)} }
    @Test fun deniedCredentialsHaveDistinctActionableErrors() {
        for(code in listOf(401,403,404)) {
            val c=Connection(URL(UpdatePolicy.API),code)
            try { GithubUpdateNetwork{c}.releases("QA_DUMMY_ACCESS_TOKEN",UpdateTransfer());fail() }
            catch(e:UpdateFailure){assertTrue(e.accessDenied);assertTrue(e.message!!.contains(if(code==401)"expired" else if(code==403)"Contents: read-only" else "repository access"))}
            assertTrue(c.closed)
        }
    }
    @Test fun rateLimitsDoNotClaimTokenAccessWasDenied() {
        for(c in listOf(Connection(URL(UpdatePolicy.API),403,remaining="0"),Connection(URL(UpdatePolicy.API),429),Connection(URL(UpdatePolicy.API),403,remaining="10",retry="60"))) {
            try { GithubUpdateNetwork{c}.releases("QA_DUMMY_ACCESS_TOKEN",UpdateTransfer());fail() }
            catch(e:UpdateFailure){assertFalse(e.accessDenied);assertTrue(e.message!!.contains("rate limit"))}
        }
    }
    @Test fun accessErrorsNeverEchoResponseBodiesOrCredentials() {
        val secret="QA_DUMMY_ACCESS_TOKEN";val c=Connection(URL(UpdatePolicy.API),401,secret.toByteArray())
        try { GithubUpdateNetwork{c}.releases(secret,UpdateTransfer());fail() }
        catch(e:UpdateFailure){assertFalse(e.message!!.contains(secret));assertTrue(c.closed)}
    }
    @Test fun oversizedMetadataIsRejected() { val c=Connection(URL(UpdatePolicy.API),200,ByteArray(UpdatePolicy.MAX_METADATA+1));fails{GithubUpdateNetwork{c}.releases("QA_DUMMY_ACCESS_TOKEN",UpdateTransfer())};assertTrue(c.closed) }
    @Test fun cancellingTransferDisconnectsTheActiveSocket() { val transfer=UpdateTransfer();val c=Connection(URL(UpdatePolicy.API),200);transfer.bind(c);transfer.cancel();assertTrue(c.closed);try{transfer.check();fail()}catch (_:CancellationException){} }
}
