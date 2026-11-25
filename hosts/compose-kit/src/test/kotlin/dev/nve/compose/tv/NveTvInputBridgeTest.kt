package dev.nve.compose.tv

import dev.nve.compose.NveEventKind
import dev.nve.compose.NveEventPayload
import dev.nve.compose.NveEventRingCodec
import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertNotNull
import org.junit.jupiter.api.Assertions.assertNull
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets

class NveTvInputBridgeTest {
    @Test
    fun mapsAllAndroidTvAndGamepadKeycodesDeterministically() {
        assertEquals(NveTvAction.DPAD_UP, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_DPAD_UP))
        assertEquals(NveTvAction.DPAD_DOWN, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_DPAD_DOWN))
        assertEquals(NveTvAction.DPAD_LEFT, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_DPAD_LEFT))
        assertEquals(NveTvAction.DPAD_RIGHT, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_DPAD_RIGHT))

        // Select aliases
        assertEquals(NveTvAction.SELECT, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_DPAD_CENTER))
        assertEquals(NveTvAction.SELECT, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_ENTER))
        assertEquals(NveTvAction.SELECT, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_NUMPAD_ENTER))
        assertEquals(NveTvAction.SELECT, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_BUTTON_A))

        // Back aliases
        assertEquals(NveTvAction.BACK, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_BACK))
        assertEquals(NveTvAction.BACK, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_ESCAPE))
        assertEquals(NveTvAction.BACK, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_BUTTON_B))

        // Media & channel keys
        assertEquals(NveTvAction.PLAY_PAUSE, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_MEDIA_PLAY_PAUSE))
        assertEquals(NveTvAction.MENU, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_MENU))
        assertEquals(NveTvAction.CHANNEL_UP, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_CHANNEL_UP))
        assertEquals(NveTvAction.CHANNEL_DOWN, NveTvKeyMapper.mapKeyCode(NveTvKeyMapper.KEYCODE_CHANNEL_DOWN))

        // Unmapped keycodes return null
        assertNull(NveTvKeyMapper.mapKeyCode(0))
        assertNull(NveTvKeyMapper.mapKeyCode(29)) // KEYCODE_A
        assertNull(NveTvKeyMapper.mapKeyCode(-1))

        // Action code roundtrip & directional helper
        for (action in NveTvAction.entries) {
            assertEquals(action, NveTvAction.fromCode(action.code))
        }
        assertNull(NveTvAction.fromCode(999))
        assertTrue(NveTvKeyMapper.isDirectional(NveTvAction.DPAD_UP))
        assertTrue(NveTvKeyMapper.isDirectional(NveTvAction.DPAD_DOWN))
        assertTrue(NveTvKeyMapper.isDirectional(NveTvAction.DPAD_LEFT))
        assertTrue(NveTvKeyMapper.isDirectional(NveTvAction.DPAD_RIGHT))
        assertFalse(NveTvKeyMapper.isDirectional(NveTvAction.SELECT))
        assertFalse(NveTvKeyMapper.isDirectional(NveTvAction.BACK))
    }

    @Test
    fun nk1BinaryEncodingAndEventRingRoundtripMatchRustWireLayoutByteForByte() {
        val bridge = NveTvInputBridge()
        val ringBytes = bridge.encodeKeyDownToRing(
            surfaceId = 3,
            nodeId = 42,
            keyCode = NveTvKeyMapper.KEYCODE_DPAD_RIGHT,
            timestampUs = 0x0011223344556677L,
            meta = true,
            ctrl = false,
            shift = true,
            alt = true,
        )
        assertNotNull(ringBytes)
        val encoded = ringBytes!!

        // Verify exact NK1 payload bytes: [0x4E, 0x4B, 0x31, 0x01|0x04|0x08=0x0D, "ArrowRight"]
        val expectedNk1 = byteArrayOf(
            0x4E,
            0x4B,
            0x31,
            (NveNk1Codec.FLAG_META or NveNk1Codec.FLAG_SHIFT or NveNk1Codec.FLAG_ALT).toByte(),
        ) + "ArrowRight".toByteArray(StandardCharsets.UTF_8)

        val decodedEvents = NveEventRingCodec.decode(encoded)
        assertEquals(1, decodedEvents.size)
        val evt = decodedEvents[0]
        assertEquals(3, evt.surfaceId)
        assertEquals(42, evt.nodeId)
        assertEquals(0x0011223344556677L, evt.timestampUs)
        assertEquals(NveEventKind.CUSTOM, evt.kind)
        val custom = evt.payload as NveEventPayload.Custom
        assertEquals("keydown", custom.name)
        assertArrayEquals(expectedNk1, custom.data)

        // Verify little-endian EventRing envelope bytes directly
        val buf = ByteBuffer.wrap(encoded).order(ByteOrder.LITTLE_ENDIAN)
        assertEquals(1, buf.int) // event_count
        assertEquals(3, buf.int) // surface_id
        assertEquals(42, buf.int) // node_id
        assertEquals(0x0011223344556677L, buf.long) // timestamp_us
        assertEquals(NveEventKind.CUSTOM.code, buf.get().toInt() and 0xFF)
        assertEquals(5, buf.get().toInt() and 0xFF) // Custom payload tag

        // Decode NK1 payload back into typed struct
        val nk1 = NveNk1Codec.decode(custom.data)
        assertEquals(
            NveNk1KeyDown(key = "ArrowRight", meta = true, ctrl = false, shift = true, alt = true),
            nk1,
        )

        // Malformed NK1 payloads rejected strictly
        assertThrows(IllegalArgumentException::class.java) { NveNk1Codec.encode("") }
        assertNull(NveNk1Codec.decode(ByteArray(0)))
        assertNull(NveNk1Codec.decode(byteArrayOf(0x4E, 0x4B, 0x31, 0x00))) // missing key byte
        assertNull(NveNk1Codec.decode(byteArrayOf(0x4E, 0x4B, 0x32, 0x00, 0x41))) // wrong magic
        assertNull(NveNk1Codec.decode(byteArrayOf(0x4E, 0x4B, 0x31, 0x00, 0xFF.toByte(), 0xFE.toByte())))
    }

    @Test
    fun repeatKeyThrottlingFiltersRapidRepeatsAndAllowsFreshPresses() {
        val bridge = NveTvInputBridge(repeatThrottleUs = 100_000L)

        // Initial press (repeatCount = 0) always accepted
        val first = bridge.onKeyDown(1, 10, NveTvKeyMapper.KEYCODE_DPAD_DOWN, repeatCount = 0, timestampUs = 1_000_000L)
        assertNotNull(first)

        // Rapid repeats within 100ms throttled
        assertNull(bridge.onKeyDown(1, 10, NveTvKeyMapper.KEYCODE_DPAD_DOWN, repeatCount = 1, timestampUs = 1_030_000L))
        assertNull(bridge.onKeyDown(1, 10, NveTvKeyMapper.KEYCODE_DPAD_DOWN, repeatCount = 2, timestampUs = 1_099_999L))

        // Repeat on a different action is independent and accepted
        assertNotNull(bridge.onKeyDown(1, 10, NveTvKeyMapper.KEYCODE_DPAD_RIGHT, repeatCount = 1, timestampUs = 1_050_000L))

        // Repeat at exact 100ms boundary accepted
        val afterWindow = bridge.onKeyDown(1, 10, NveTvKeyMapper.KEYCODE_DPAD_DOWN, repeatCount = 3, timestampUs = 1_100_000L)
        assertNotNull(afterWindow)

        // Next rapid repeat after 1_100_000L is throttled again
        assertNull(bridge.onKeyDown(1, 10, NveTvKeyMapper.KEYCODE_DPAD_DOWN, repeatCount = 4, timestampUs = 1_150_000L))

        // Fresh press (repeatCount = 0) bypasses repeat throttle immediately
        assertNotNull(bridge.onKeyDown(1, 10, NveTvKeyMapper.KEYCODE_DPAD_DOWN, repeatCount = 0, timestampUs = 1_160_000L))

        // Reset clears throttle state
        bridge.resetThrottle()
        assertNotNull(bridge.onKeyDown(1, 10, NveTvKeyMapper.KEYCODE_DPAD_DOWN, repeatCount = 1, timestampUs = 1_165_000L))

        // Batch encoding multiple TV actions into one EventRing
        val batch = bridge.encodeBatchToRing(
            listOf(
                bridge.encodeAction(1, 5, NveTvAction.SELECT, timestampUs = 10L),
                bridge.encodeAction(1, 5, NveTvAction.BACK, timestampUs = 20L),
                bridge.encodeAction(1, 5, NveTvAction.PLAY_PAUSE, timestampUs = 30L),
            ),
        )
        val decoded = NveEventRingCodec.decode(batch)
        assertEquals(3, decoded.size)
        assertEquals("Select", NveNk1Codec.decode((decoded[0].payload as NveEventPayload.Custom).data)?.key)
        assertEquals("Back", NveNk1Codec.decode((decoded[1].payload as NveEventPayload.Custom).data)?.key)
        assertEquals("MediaPlayPause", NveNk1Codec.decode((decoded[2].payload as NveEventPayload.Custom).data)?.key)
    }

    @Test
    fun computesStandardFivePercentTvOverscanSafeAreaInsets() {
        val standard = NveTvOverscanInsets.STANDARD_1080P
        assertEquals(48f, standard.leftDp, 1e-4f)
        assertEquals(48f, standard.rightDp, 1e-4f)
        assertEquals(27f, standard.topDp, 1e-4f)
        assertEquals(27f, standard.bottomDp, 1e-4f)
        assertEquals(864f, standard.safeWidthDp(960f), 1e-4f)
        assertEquals(486f, standard.safeHeightDp(540f), 1e-4f)

        // 1920x1080 at 2.0x density (standard Android TV xhdpi) => 960x540dp => 48dp / 27dp
        val from1080p = NveTvOverscanInsets.forViewportPx(1920, 1080, density = 2.0f)
        assertEquals(standard, from1080p)

        // 3840x2160 at 4.0x density => 960x540dp => 48dp / 27dp
        val from4k = NveTvOverscanInsets.forViewportPx(3840, 2160, density = 4.0f)
        assertEquals(standard, from4k)

        // Invalid / non-finite dimensions clamp safely to 0
        val invalid = NveTvOverscanInsets.forViewportDp(Float.NaN, -100f)
        assertEquals(0f, invalid.leftDp, 1e-4f)
        assertEquals(0f, invalid.topDp, 1e-4f)
    }

    @Test
    fun detectsAndroidTvLeanbackAndFireTvEnvironmentsDeterministically() {
        assertFalse(NveTvEnvironmentDetector.isTvEnvironment(NveTvEnvironmentSignals()))

        assertTrue(
            NveTvEnvironmentDetector.isTvEnvironment(
                NveTvEnvironmentSignals(hasLeanbackFeature = true),
            ),
        )
        assertTrue(
            NveTvEnvironmentDetector.isTvEnvironment(
                NveTvEnvironmentSignals(hasLeanbackOnlyFeature = true),
            ),
        )
        // UI_MODE_TYPE_TELEVISION (0x04) combined with UI_MODE_NIGHT_YES (0x20)
        assertTrue(NveTvEnvironmentDetector.isTelevisionUiMode(0x20 or 0x04))
        assertFalse(NveTvEnvironmentDetector.isTelevisionUiMode(0x20 or 0x01)) // UI_MODE_TYPE_NORMAL
        assertTrue(
            NveTvEnvironmentDetector.isTvEnvironment(
                NveTvEnvironmentSignals(uiMode = 0x24),
            ),
        )
        // Amazon Fire TV Stick 4K Max (AFTKA)
        assertTrue(
            NveTvEnvironmentDetector.isTvEnvironment(
                NveTvEnvironmentSignals(manufacturer = "Amazon", model = "AFTKA"),
            ),
        )
        // Amazon Fire tablet (non-AFT model) must not misclassify as TV
        assertFalse(
            NveTvEnvironmentDetector.isTvEnvironment(
                NveTvEnvironmentSignals(manufacturer = "Amazon", model = "KFTRWI"),
            ),
        )
    }
}
