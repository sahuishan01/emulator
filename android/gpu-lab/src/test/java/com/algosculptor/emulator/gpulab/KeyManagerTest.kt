package com.algosculptor.emulator.gpulab

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

class KeyManagerTest {
    @get:Rule
    val tempFolder = TemporaryFolder()

    @Test
    fun countValidKeys_countsValidLinesAndIgnoresComments() {
        val file = tempFolder.newFile("test_prod.keys")
        file.writeText(
            """
            # Sample comment
            header_key = 1111111111111111111111111111111111111111111111111111111111111111
            # Another comment
            master_key_00 = 22222222222222222222222222222222 # inline comment
            invalid_key = 1234
            bad_hex = zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz
            key_area_key_application_00 = 33333333333333333333333333333333
            
            """.trimIndent()
        )

        val count = KeyManager.countValidKeys(file)
        assertEquals(3, count)
    }

    @Test
    fun countValidKeys_returnsZeroForEmptyOrMissingFile() {
        val missing = File(tempFolder.root, "non_existent.keys")
        assertEquals(0, KeyManager.countValidKeys(missing))

        val empty = tempFolder.newFile("empty.keys")
        assertEquals(0, KeyManager.countValidKeys(empty))
    }
}
