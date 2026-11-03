package com.algosculptor.emulator.gpulab

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.util.AtomicFile
import java.io.File
import java.io.FileNotFoundException

/** One read-only content URI, granted only to Android's installer on request. */
class UpdateApkProvider : ContentProvider() {
    override fun onCreate(): Boolean = true
    private fun file(uri: Uri): File {
        val app = context ?: throw FileNotFoundException()
        if (uri.scheme != "content" || uri.authority != app.packageName + ".updates" || uri.query != null || uri.fragment != null) throw FileNotFoundException()
        val directory = File(app.filesDir, "updates")
        val release = try { AtomicFile(File(directory, "ready.json")).openRead().use { check(it.channel.size() <= 8192); UpdateRelease.restore(String(it.readBytes(), Charsets.UTF_8)) } } catch (_: Exception) { throw FileNotFoundException() }
        if (uri.encodedPath != "/apk/${release.sha}.apk") throw FileNotFoundException()
        return File(directory, "ready.apk").takeIf { it.isFile && it.length() == release.size } ?: throw FileNotFoundException()
    }
    override fun getType(uri: Uri): String { file(uri); return "application/vnd.android.package-archive" }
    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor { if (mode != "r") throw FileNotFoundException(); return ParcelFileDescriptor.open(file(uri), ParcelFileDescriptor.MODE_READ_ONLY) }
    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor {
        val file = file(uri); val columns = projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        if (columns.any { it !in setOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE) }) throw IllegalArgumentException("Unsupported update query")
        return MatrixCursor(columns).apply { addRow(columns.map { if (it == OpenableColumns.DISPLAY_NAME) "emulator-update.apk" else file.length() }) }
    }
    override fun insert(uri: Uri, values: ContentValues?): Uri = throw UnsupportedOperationException()
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = throw UnsupportedOperationException()
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int = throw UnsupportedOperationException()
}
