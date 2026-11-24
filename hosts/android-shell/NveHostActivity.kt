package dev.nve.host

/**
 * Minimal Kotlin bridge for the Prebuilt Android Fabric Host Shell.
 * Loads and hot-swaps compiled Rust app `.so` generations via `NveJniBridge.cpp`.
 */
object NveNativeBridge {
    init {
        System.loadLibrary("nve_android_shell")
    }

    external fun nativeCreateHost(
        surfaceId: Int,
        width: Float,
        height: Float,
        scaleFactor: Float,
    ): Long

    external fun nativeHotSwapApp(handlePtr: Long, soPath: String): Boolean
}
