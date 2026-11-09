package com.algosculptor.emulator.gpulab

import android.app.DownloadManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import java.io.InputStream

internal data class DownloadSnapshot(val state: Int, val bytes: Long, val total: Long, val reason: Int)
internal interface UpdateDownloads {
    fun enqueue(url: String, release: UpdateRelease, nonce: String, wifiOnly: Boolean): Long
    fun find(nonce: String): Long?
    fun query(id: Long): DownloadSnapshot?
    fun open(id: Long): InputStream
    fun remove(id: Long)
}

/** Android owns transfer lifetime. No PAT or Authorization header is given to it. */
internal class AndroidUpdateDownloads(private val context: Context) : UpdateDownloads {
    private val manager = context.getSystemService(DownloadManager::class.java)
    private fun filename(nonce: String): String { require(nonce.matches(Regex("[a-f0-9-]{36}"))); return "emulator-update-$nonce.apk" }
    override fun enqueue(url: String, release: UpdateRelease, nonce: String, wifiOnly: Boolean): Long {
        UpdatePolicy.checkUrl(url, false)
        val request = DownloadManager.Request(Uri.parse(url))
            .setTitle("Emulator ${release.version}").setDescription("App update — continues in the background")
            .setMimeType("application/vnd.android.package-archive")
            .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
            .setAllowedOverMetered(!wifiOnly).setAllowedOverRoaming(false)
            .setDestinationInExternalFilesDir(context, Environment.DIRECTORY_DOWNLOADS, filename(nonce))
        return manager.enqueue(request)
    }
    override fun find(nonce: String): Long? = manager.query(DownloadManager.Query())?.use { cursor ->
        val uriIndex = cursor.getColumnIndex(DownloadManager.COLUMN_LOCAL_URI)
        val idIndex = cursor.getColumnIndexOrThrow(DownloadManager.COLUMN_ID)
        val expected = filename(nonce)
        while (cursor.moveToNext()) {
            val uriStr = if (uriIndex >= 0) cursor.getString(uriIndex) else null
            if (uriStr != null && Uri.parse(uriStr).lastPathSegment == expected) return@use cursor.getLong(idIndex)
        }
        null
    }
    override fun query(id: Long): DownloadSnapshot? = manager.query(DownloadManager.Query().setFilterById(id))?.use { cursor ->
        if (!cursor.moveToFirst()) null else DownloadSnapshot(cursor.getInt(cursor.getColumnIndexOrThrow(DownloadManager.COLUMN_STATUS)),
            cursor.getLong(cursor.getColumnIndexOrThrow(DownloadManager.COLUMN_BYTES_DOWNLOADED_SO_FAR)),
            cursor.getLong(cursor.getColumnIndexOrThrow(DownloadManager.COLUMN_TOTAL_SIZE_BYTES)),
            cursor.getInt(cursor.getColumnIndexOrThrow(DownloadManager.COLUMN_REASON)))
    }
    override fun open(id: Long): InputStream = android.os.ParcelFileDescriptor.AutoCloseInputStream(manager.openDownloadedFile(id))
    override fun remove(id: Long) { if (id > 0) manager.remove(id) }
}

/** Protected system actions only; job rechecks the app-owned persisted request. */
class UpdateDownloadReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            DownloadManager.ACTION_DOWNLOAD_COMPLETE -> {
                val id = intent.getLongExtra(DownloadManager.EXTRA_DOWNLOAD_ID, -1L)
                val prefs = context.getSharedPreferences("updates", Context.MODE_PRIVATE)
                if (prefs.getBoolean("downloadPending", false)) {
                    UpdateSchedule.recover(context, immediate = true)
                }
            }
            DownloadManager.ACTION_NOTIFICATION_CLICKED -> context.startActivity(Intent(context, UpdatesActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }
}
