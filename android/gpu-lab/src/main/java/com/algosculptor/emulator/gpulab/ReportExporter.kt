package com.algosculptor.emulator.gpulab

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import org.json.JSONObject

/** Publish an app-owned Downloads entry only after the complete JSON is written. */
object ReportExporter {
    data class Saved(val uri: Uri, val filename: String)
    fun save(context: Context, report: JSONObject, runId: String): Saved {
        check(runId.matches(Regex("[A-Za-z0-9-]{1,80}")))
        val filename = "emulator-gpu-report-$runId.json"
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, filename)
            put(MediaStore.Downloads.MIME_TYPE, "application/json")
            put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/Emulator")
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val resolver = context.contentResolver
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: error("Could not create the report in Downloads")
        try {
            report.put("automaticExport", JSONObject().put("uri", uri.toString()).put("filename", filename).put("directory", "Downloads/Emulator"))
            val text = report.toString(2)
            val bytes = text.toByteArray(Charsets.UTF_8)
            check(bytes.size <= 1024 * 1024) { "Report exceeds the export limit" }
            ReportStore.write(context, "last-report.json", text)
            resolver.openOutputStream(uri, "wt")?.use { it.write(bytes) } ?: error("Could not write the report")
            check(resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null) == 1) { "Could not publish the report" }
            // Publication is complete before optional networking is queued. A
            // report-upload setup failure must never retract a local export.
            if (context.getSharedPreferences("report-uploads", Context.MODE_PRIVATE).getBoolean("automatic", false)) {
                try { (context.applicationContext as EmulatorApplication).reportUploads.enqueueAutomatic(bytes) } catch (_: Exception) { }
            }
            return Saved(uri, filename)
        } catch (error: Exception) {
            try { resolver.delete(uri, null, null) } catch (_: Exception) { }
            throw error
        }
    }
    fun published(context: Context, uri: Uri): Boolean = try {
        context.contentResolver.query(uri, arrayOf(MediaStore.Downloads.IS_PENDING), null, null, null)?.use {
            it.moveToFirst() && it.getInt(0) == 0
        } ?: false
    } catch (_: Exception) { false }
}
