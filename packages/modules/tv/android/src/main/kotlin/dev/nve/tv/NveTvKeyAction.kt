package dev.nve.tv

/**
 * Typed TV remote & D-Pad action with stable numeric code and W3C key name.
 */
enum class NveTvAction(
    val code: Int,
    val keyName: String,
) {
    DPAD_UP(1, "ArrowUp"),
    DPAD_DOWN(2, "ArrowDown"),
    DPAD_LEFT(3, "ArrowLeft"),
    DPAD_RIGHT(4, "ArrowRight"),
    SELECT(5, "Select"),
    BACK(6, "Back"),
    PLAY_PAUSE(7, "MediaPlayPause"),
    MENU(8, "Menu"),
    CHANNEL_UP(9, "ChannelUp"),
    CHANNEL_DOWN(10, "ChannelDown");

    companion object {
        fun fromCode(code: Int): NveTvAction? = entries.firstOrNull { it.code == code }
    }
}

/**
 * Pure-Kotlin Android `KeyEvent` keycode mapper for Android TV, Fire TV, and Leanback remotes.
 *
 * Uses raw integer constants matching `android.view.KeyEvent` so JVM unit tests execute with zero
 * Robolectric or Android framework runtime dependency.
 */
object NveTvKeyMapper {
    const val KEYCODE_BACK: Int = 4
    const val KEYCODE_DPAD_UP: Int = 19
    const val KEYCODE_DPAD_DOWN: Int = 20
    const val KEYCODE_DPAD_LEFT: Int = 21
    const val KEYCODE_DPAD_RIGHT: Int = 22
    const val KEYCODE_DPAD_CENTER: Int = 23
    const val KEYCODE_ENTER: Int = 66
    const val KEYCODE_MENU: Int = 82
    const val KEYCODE_MEDIA_PLAY_PAUSE: Int = 85
    const val KEYCODE_BUTTON_A: Int = 96
    const val KEYCODE_BUTTON_B: Int = 97
    const val KEYCODE_ESCAPE: Int = 111
    const val KEYCODE_NUMPAD_ENTER: Int = 160
    const val KEYCODE_CHANNEL_UP: Int = 166
    const val KEYCODE_CHANNEL_DOWN: Int = 167

    fun mapKeyCode(keyCode: Int): NveTvAction? = when (keyCode) {
        KEYCODE_DPAD_UP -> NveTvAction.DPAD_UP
        KEYCODE_DPAD_DOWN -> NveTvAction.DPAD_DOWN
        KEYCODE_DPAD_LEFT -> NveTvAction.DPAD_LEFT
        KEYCODE_DPAD_RIGHT -> NveTvAction.DPAD_RIGHT
        KEYCODE_DPAD_CENTER,
        KEYCODE_ENTER,
        KEYCODE_NUMPAD_ENTER,
        KEYCODE_BUTTON_A -> NveTvAction.SELECT
        KEYCODE_BACK,
        KEYCODE_ESCAPE,
        KEYCODE_BUTTON_B -> NveTvAction.BACK
        KEYCODE_MEDIA_PLAY_PAUSE -> NveTvAction.PLAY_PAUSE
        KEYCODE_MENU -> NveTvAction.MENU
        KEYCODE_CHANNEL_UP -> NveTvAction.CHANNEL_UP
        KEYCODE_CHANNEL_DOWN -> NveTvAction.CHANNEL_DOWN
        else -> null
    }

    fun isDirectional(action: NveTvAction): Boolean = when (action) {
        NveTvAction.DPAD_UP,
        NveTvAction.DPAD_DOWN,
        NveTvAction.DPAD_LEFT,
        NveTvAction.DPAD_RIGHT -> true
        else -> false
    }
}
