package com.algosculptor.emulator.gpulab

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** The access token is encrypted with a non-exportable, device-local key. */
internal class UpdateVault(context: Context) {
    private val storage = AtomicFile(File(context.noBackupFilesDir, "updates-access.bin"))
    private val alias = "emulator-update-access-v1"
    fun exists(): Boolean = storage.baseFile.exists() || File(storage.baseFile.path + ".bak").exists()
    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT).setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        }.generateKey()
    }
    @Synchronized fun write(token: String) {
        require(token.length in 20..512 && token.all { it.code in 33..126 }) { "Enter a valid GitHub access token." }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()); updateAAD(alias.toByteArray()) }
        val bytes = byteArrayOf(1, cipher.iv.size.toByte()) + cipher.iv + cipher.doFinal(token.toByteArray(Charsets.UTF_8))
        val output = storage.startWrite()
        try { output.write(bytes); storage.finishWrite(output) } catch (e: Exception) { storage.failWrite(output); throw e }
    }
    @Synchronized fun read(): String {
        val data = storage.openRead().use { check(it.channel.size() <= 1024); it.readBytes() }
        check(data.size >= 30 && data[0] == 1.toByte() && data[1] == 12.toByte())
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, data.copyOfRange(2, 14))); updateAAD(alias.toByteArray()) }
        return String(cipher.doFinal(data.copyOfRange(14, data.size)), Charsets.UTF_8)
    }
    @Synchronized fun clear() { storage.delete(); KeyStore.getInstance("AndroidKeyStore").apply { load(null); deleteEntry(alias) } }
}
