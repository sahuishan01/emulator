package com.algosculptor.emulator.gpulab

/** Each upload/session is confined to the calling worker thread. */
object NativePrograms {
    init { System.loadLibrary("emulator_gpu_lab") }
    external fun inspect(path: String): ByteArray
    external fun start(path: String): ByteArray
    external fun step(budget: Int): ByteArray
    external fun stop()
}
