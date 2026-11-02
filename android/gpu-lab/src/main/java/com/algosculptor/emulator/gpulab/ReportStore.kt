package com.algosculptor.emulator.gpulab

import android.content.Context
import android.util.AtomicFile
import java.io.File

object ReportStore {
    @Synchronized fun read(context: Context, name: String): String? = try {
        AtomicFile(File(context.filesDir, name)).openRead().use {
            check(it.channel.size() <= 1024 * 1024) { "Cached report is too large" }
            String(it.readBytes(), Charsets.UTF_8)
        }
    } catch (_: java.io.FileNotFoundException) { null }
    @Synchronized fun write(context: Context, name: String, text: String) {
        val file = AtomicFile(File(context.filesDir, name)); val output = file.startWrite()
        try { output.write(text.toByteArray(Charsets.UTF_8)); file.finishWrite(output) }
        catch (error: Exception) { file.failWrite(output); throw error }
    }
}
