package com.algosculptor.emulator.gpulab

import android.app.Application
import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import android.net.Uri
import android.os.CancellationSignal
import android.os.Handler
import android.os.Looper
import android.provider.OpenableColumns
import org.json.JSONObject
import java.io.File
import java.io.FileOutputStream
import java.io.InputStream
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.CancellationException
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

class EmulatorApplication : Application() {
    val library by lazy { LibraryStore(this) }
    internal val updates by lazy { UpdateManager(this) }
    internal val reportUploads by lazy { ReportUploads(this) }
    override fun onCreate() {
        super.onCreate()
        AppLog.init(this)
        UpdateSchedule.sync(this)
        if (getSharedPreferences("report-uploads", MODE_PRIVATE).getBoolean("automatic", false)) reportUploads
        else ReportUploadSchedule.sync(this)
    }
}

data class LibraryEntry(val id: String, val title: String, val filename: String, val bytes: Long,
    val mode: String, val lastPlayed: Long, val launches: Int, val result: String) {
    val included: Boolean get() = id == "builtin:catch" || id == "builtin:textures"
}
data class ImportStatus(val busy: Boolean = false, val message: String = "", val error: Boolean = false)

/** One writer owns import/file/catalog transitions. Activities observe without owning jobs. */
class LibraryStore(private val app: Context) {
    companion object {
        const val MAX_NRO_FILE = 32L * 1024 * 1024
        const val MAX_PACKAGE_FILE = 64L * 1024 * 1024 * 1024
        const val MAX_FILE = MAX_PACKAGE_FILE
        const val MAX_LIBRARY = 512L * 1024 * 1024 * 1024
    }
    private val worker = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())
    private val observers = mutableSetOf<(ImportStatus) -> Unit>()
    private var status = ImportStatus()
    private class ImportJob {
        val cancelled = AtomicBoolean(false)
        val stream = AtomicReference<InputStream?>()
        val signal = CancellationSignal()
    }
    private val activeImport = AtomicReference<ImportJob?>()
    private val directory = File(app.filesDir, "library").apply { mkdirs() }
    private val db = object : SQLiteOpenHelper(app, "library.db", null, 1) {
        override fun onCreate(db: SQLiteDatabase) {
            db.execSQL("CREATE TABLE titles (id TEXT PRIMARY KEY, title TEXT NOT NULL, filename TEXT NOT NULL, bytes INTEGER NOT NULL, mode TEXT NOT NULL, last_played INTEGER NOT NULL DEFAULT 0, launches INTEGER NOT NULL DEFAULT 0, result TEXT NOT NULL DEFAULT '')")
            db.execSQL("INSERT INTO titles(id,title,filename,bytes,mode) VALUES('builtin:catch','Catch','',0,'catch')")
        }
        override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) { error("Unknown library schema") }
    }
    init {
        worker.execute {
            db.writableDatabase.execSQL("INSERT OR IGNORE INTO titles(id,title,filename,bytes,mode) VALUES('builtin:textures','Texture scene','',0,'textures')")
            val indexed = entries().map { it.filename }.toSet()
            directory.listFiles()?.forEach { file ->
                if (file.name.startsWith(".import-") || (file.name.matches(Regex("[0-9a-f]{64}\\.(nro|nsp)")) && file.name !in indexed)) file.delete()
            }
        }
    }
    fun observe(observer: (ImportStatus) -> Unit) { observers.add(observer); observer(status) }
    fun unobserve(observer: (ImportStatus) -> Unit) { observers.remove(observer) }
    private fun publish(value: ImportStatus) { main.post { status = if (activeImport.get() != null) ImportStatus(true, "Importing…") else value.copy(busy = false); observers.toList().forEach { it(status) } } }
    fun dismissStatus() { if (!status.busy) { status = ImportStatus(); observers.toList().forEach { it(status) } } }
    private fun entries(): List<LibraryEntry> = db.readableDatabase.query("titles", null, null, null, null, null, "last_played DESC, title COLLATE NOCASE").use { c ->
        buildList { while (c.moveToNext()) add(LibraryEntry(c.getString(c.getColumnIndexOrThrow("id")), c.getString(c.getColumnIndexOrThrow("title")),
            c.getString(c.getColumnIndexOrThrow("filename")), c.getLong(c.getColumnIndexOrThrow("bytes")), c.getString(c.getColumnIndexOrThrow("mode")),
            c.getLong(c.getColumnIndexOrThrow("last_played")), c.getInt(c.getColumnIndexOrThrow("launches")), c.getString(c.getColumnIndexOrThrow("result")))) }
    }
    fun list(callback: (List<LibraryEntry>) -> Unit) { worker.execute { val result = entries(); main.post { callback(result) } } }
    fun find(id: String, callback: (LibraryEntry?) -> Unit) { worker.execute { val result = entries().firstOrNull { it.id == id }; main.post { callback(result) } } }
    fun file(entry: LibraryEntry): File? = if (entry.filename.matches(Regex("[0-9a-f]{64}\\.(nro|nsp)"))) File(directory, entry.filename).takeIf { it.isFile } else null
    fun recordLaunch(id: String) { worker.execute { db.writableDatabase.execSQL("UPDATE titles SET last_played=?, launches=launches+1 WHERE id=?", arrayOf(System.currentTimeMillis(), id)) } }
    fun recordResult(id: String, result: String) { worker.execute { db.writableDatabase.update("titles", ContentValues().apply { put("result", result.take(512)) }, "id=?", arrayOf(id)) } }
    fun remove(entry: LibraryEntry) {
        if (entry.included) return
        worker.execute {
            // Delete our copy first. If deletion fails, keep its catalog row.
            val file = file(entry)
            if (file != null && !file.delete()) { publish(ImportStatus(message = "Could not remove this file.", error = true)); return@execute }
            db.writableDatabase.delete("titles", "id=?", arrayOf(entry.id))
            AppLog.i("LibraryStore", "Removed title: ${entry.title} (${entry.id})")
            publish(ImportStatus(message = "Removed from library."))
        }
    }
    fun cancelImport() {
        val job = activeImport.get() ?: return
        job.cancelled.set(true); job.signal.cancel()
        try { job.stream.getAndSet(null)?.close() } catch (_: Exception) { }
    }
    fun import(uri: Uri) {
        val job = ImportJob()
        if (!activeImport.compareAndSet(null, job)) return
        status = ImportStatus(true, "Importing title…")
        observers.toList().forEach { it(status) }
        val cancellation = job.signal
        worker.execute {
            val temporary = File(directory, ".import-${UUID.randomUUID()}")
            var published: File? = null
            var outcome = ImportStatus(message = "Import failed.", error = true)
            try {
                val current = entries()
                require(current.count { !it.included } < 128) { "The library can contain up to 128 imported files." }
                val used = current.sumOf { it.bytes }
                val name = app.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null, cancellation)?.use { c ->
                    if (c.moveToFirst()) c.getString(0) else null
                }?.replace(Regex("\\s+"), " ")?.trim()?.take(100) ?: "Program.nro"
                val ext = name.substringAfterLast('.', "").lowercase()
                require(ext in listOf("nro", "nsp")) { "Choose a .nro homebrew or .nsp package file." }
                AppLog.i("LibraryStore", "Importing file $name (extension: $ext)")
                val digest = MessageDigest.getInstance("SHA-256")
                var size = 0L
                val maxFile = if (ext == "nro") MAX_NRO_FILE else MAX_PACKAGE_FILE
                app.contentResolver.openAssetFileDescriptor(uri, "r", cancellation)?.use { descriptor ->
                    descriptor.createInputStream().use { input ->
                        job.stream.set(input)
                        FileOutputStream(temporary).use { output ->
                            val buffer = ByteArray(262144)
                            var emptyReads = 0
                            var lastReport = 0L
                            while (true) {
                                if (job.cancelled.get()) throw CancellationException()
                                val count = input.read(buffer)
                                if (count < 0) break
                                if (count == 0) { check(++emptyReads <= 16) { "The document provider did not return data." }; continue }
                                emptyReads = 0
                                size += count
                                require(size <= maxFile) { if (ext == "nro") "NRO files can be up to 32 MiB." else "Packages can be up to 64 GiB." }
                                require(used + size <= MAX_LIBRARY) { "The library storage limit is 512 GiB. Remove a title before importing more." }
                                output.write(buffer, 0, count); digest.update(buffer, 0, count)
                                val now = System.currentTimeMillis()
                                if (now - lastReport >= 500) {
                                    lastReport = now
                                    val mb = size / (1024 * 1024)
                                    publish(ImportStatus(true, "Importing… ($mb MB)"))
                                }
                            }
                            output.fd.sync()
                        }
                    }
                } ?: error("Could not open this document.")
                if (job.cancelled.get()) throw CancellationException()
                val metadata = JSONObject(String(NativePrograms.inspect(temporary.absolutePath), Charsets.UTF_8))
                require(metadata.optBoolean("ok")) { metadata.optString("error", "This file format is not supported yet.") }
                if (job.cancelled.get()) throw CancellationException()
                val id = digest.digest().joinToString("") { "%02x".format(it) }
                val mode = metadata.optString("mode", if (ext == "nsp") "package" else "static")
                AppLog.i("LibraryStore", "Inspected metadata for $name: format=${metadata.optString("format")}, mode=$mode, size=$size bytes")
                if (mode == "catch" || current.any { it.id == id }) {
                    outcome = ImportStatus(message = if (mode == "catch") "Catch is already included." else "This title is already in your library.")
                    return@execute
                }
                val destination = File(directory, "$id.$ext")
                check(!destination.exists()) { "An unfinished copy exists. Restart the app and try again." }
                check(temporary.renameTo(destination)) { "Could not save this title." }; published = destination
                db.writableDatabase.insertOrThrow("titles", null, ContentValues().apply {
                    put("id", id); put("title", name); put("filename", destination.name); put("bytes", size); put("mode", mode)
                })
                published = null
                AppLog.i("LibraryStore", "Successfully added title $name (mode=$mode, id=$id)")
                outcome = ImportStatus(message = "Added $name")
            } catch (e: Exception) {
                published?.delete()
                AppLog.e("LibraryStore", "Import failed for $uri: ${e.message}", e)
                outcome = ImportStatus(message = if (job.cancelled.get()) "Import cancelled." else (e.message ?: "Import failed.").take(512), error = !job.cancelled.get())
            } finally {
                job.stream.set(null); temporary.delete()
                NativePrograms.stop()
                activeImport.compareAndSet(job, null)
                publish(outcome)
            }
        }
    }
}
