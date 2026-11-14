package com.algosculptor.emulator.gpulab

import android.content.Context
import android.net.Uri
import java.io.File
import java.io.FileOutputStream

object KeyManager {
    data class KeyInfo(
        val hasProdKeys: Boolean,
        val prodKeyCount: Int,
        val hasTitleKeys: Boolean,
        val titleKeyCount: Int
    )

    fun keysDir(context: Context): File = File(context.filesDir, "keys").apply { mkdirs() }
    fun prodKeysFile(context: Context): File = File(keysDir(context), "prod.keys")
    fun titleKeysFile(context: Context): File = File(keysDir(context), "title.keys")

    fun countValidKeys(file: File): Int {
        if (!file.exists() || !file.isFile || file.length() == 0L) return 0
        return try {
            file.useLines { lines ->
                lines.count { raw ->
                    val line = raw.trim()
                    if (line.isEmpty() || line.startsWith("#")) false
                    else {
                        val parts = line.split("=", limit = 2)
                        if (parts.size != 2) false
                        else {
                            val name = parts[0].trim()
                            val valueWithComment = parts[1].trim()
                            val value = valueWithComment.split("#", limit = 2)[0].trim()
                            name.isNotEmpty() && (value.length == 32 || value.length == 64) &&
                                    value.all { (it in '0'..'9') || (it in 'a'..'f') || (it in 'A'..'F') }
                        }
                    }
                }
            }
        } catch (_: Exception) {
            0
        }
    }

    fun getInfo(context: Context): KeyInfo {
        autoDiscoverKeys(context)
        val prodCount = countValidKeys(prodKeysFile(context))
        val titleCount = countValidKeys(titleKeysFile(context))
        return KeyInfo(
            hasProdKeys = prodCount > 0,
            prodKeyCount = prodCount,
            hasTitleKeys = titleCount > 0,
            titleKeyCount = titleCount
        )
    }

    fun autoDiscoverKeys(context: Context): Boolean {
        val prod = prodKeysFile(context)
        if (prod.exists() && prod.length() > 0L) return false

        val candidates = listOf(
            "/sdcard/switch/prod.keys",
            "/storage/emulated/0/switch/prod.keys",
            "/sdcard/Download/prod.keys",
            "/storage/emulated/0/Download/prod.keys",
            "/storage/emulated/0/Android/data/org.yuzu.yuzu_emu/files/keys/prod.keys",
            "/storage/emulated/0/Android/data/dev.lemon.lemon_emu/files/keys/prod.keys",
            "/storage/emulated/0/Android/data/org.suyu.suyu_emu/files/keys/prod.keys",
            "/storage/emulated/0/Android/data/org.sudachi.sudachi_emu/files/keys/prod.keys"
        )
        for (path in candidates) {
            val f = File(path)
            if (f.exists() && f.canRead() && f.length() > 0L) {
                try {
                    val count = countValidKeys(f)
                    if (count > 0) {
                        f.copyTo(prod, overwrite = true)
                        f.copyTo(File(context.filesDir, "prod.keys"), overwrite = true)
                        AppLog.i("KeyManager", "Auto-discovered and imported $count keys from $path")
                        return true
                    }
                } catch (e: Exception) {
                    AppLog.w("KeyManager", "Failed reading candidate keys from $path: ${e.message}")
                }
            }
        }
        return false
    }

    fun installFromUri(context: Context, uri: Uri, isTitleKeys: Boolean): Pair<Boolean, String> {
        val target = if (isTitleKeys) titleKeysFile(context) else prodKeysFile(context)
        val sibling = if (isTitleKeys) File(context.filesDir, "title.keys") else File(context.filesDir, "prod.keys")
        val temp = File(keysDir(context), ".import_keys_${System.currentTimeMillis()}")
        try {
            context.contentResolver.openInputStream(uri)?.use { input ->
                FileOutputStream(temp).use { output ->
                    input.copyTo(output)
                }
            } ?: return Pair(false, "Could not open document.")

            val count = countValidKeys(temp)
            if (count == 0) {
                temp.delete()
                return Pair(false, "No valid keys found in selected file. Ensure it is a valid ${if (isTitleKeys) "title.keys" else "prod.keys"} format.")
            }

            if (target.exists()) target.delete()
            if (!temp.renameTo(target)) {
                temp.copyTo(target, overwrite = true)
                temp.delete()
            }
            try { target.copyTo(sibling, overwrite = true) } catch (_: Exception) {}

            val kind = if (isTitleKeys) "title.keys" else "prod.keys"
            AppLog.i("KeyManager", "Installed $kind with $count valid entries.")
            return Pair(true, "Successfully installed $kind ($count keys loaded).")
        } catch (e: Exception) {
            temp.delete()
            AppLog.e("KeyManager", "Failed to install keys: ${e.message}", e)
            return Pair(false, "Failed to install keys: ${e.message ?: "Unknown error"}")
        }
    }

    fun clearKeys(context: Context): Boolean {
        var any = false
        val f1 = prodKeysFile(context)
        val f2 = titleKeysFile(context)
        val f3 = File(context.filesDir, "prod.keys")
        val f4 = File(context.filesDir, "title.keys")
        if (f1.exists()) { f1.delete(); any = true }
        if (f2.exists()) { f2.delete(); any = true }
        if (f3.exists()) { f3.delete(); any = true }
        if (f4.exists()) { f4.delete(); any = true }
        AppLog.i("KeyManager", "Cleared installed decryption keys.")
        return any
    }
}
