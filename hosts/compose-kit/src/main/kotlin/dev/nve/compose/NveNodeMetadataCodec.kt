package dev.nve.compose

import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets

/**
 * Retained per-node accessibility, `TextInput`, `Image`, and `SkiaCanvas` metadata decoded from
 * one batched `NVE_MET1` binary slab per dirty frame (zero JSON, zero per-prop JNI chatter).
 */
data class NveNodeMetadata(
    val accessibilityLabel: String = "",
    val accessibilityRole: String = "",
    val accessibilityHint: String = "",
    val accessibilityStateDescription: String = "",
    val disabled: Boolean = false,
    val accessibilityHidden: Boolean = false,
    val placeholder: String = "",
    val secureTextEntry: Boolean = false,
    val multiline: Boolean = false,
    val resizeMode: Int = RESIZE_MODE_COVER,
    val testId: String = "",
    val drawListBytes: ByteArray? = null,
    val drawListCorrupt: Boolean = false,
) {
    companion object {
        const val RESIZE_MODE_COVER: Int = 0
        const val RESIZE_MODE_CONTAIN: Int = 1
        const val RESIZE_MODE_STRETCH: Int = 2
        const val RESIZE_MODE_CENTER: Int = 3

        val EMPTY: NveNodeMetadata = NveNodeMetadata()
    }

    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is NveNodeMetadata) return false
        return accessibilityLabel == other.accessibilityLabel &&
            accessibilityRole == other.accessibilityRole &&
            accessibilityHint == other.accessibilityHint &&
            accessibilityStateDescription == other.accessibilityStateDescription &&
            disabled == other.disabled &&
            accessibilityHidden == other.accessibilityHidden &&
            placeholder == other.placeholder &&
            secureTextEntry == other.secureTextEntry &&
            multiline == other.multiline &&
            resizeMode == other.resizeMode &&
            testId == other.testId &&
            drawListCorrupt == other.drawListCorrupt &&
            ((drawListBytes == null && other.drawListBytes == null) ||
                (drawListBytes != null && other.drawListBytes != null &&
                    drawListBytes.contentEquals(other.drawListBytes)))
    }

    override fun hashCode(): Int {
        var result = accessibilityLabel.hashCode()
        result = 31 * result + accessibilityRole.hashCode()
        result = 31 * result + accessibilityHint.hashCode()
        result = 31 * result + accessibilityStateDescription.hashCode()
        result = 31 * result + disabled.hashCode()
        result = 31 * result + accessibilityHidden.hashCode()
        result = 31 * result + placeholder.hashCode()
        result = 31 * result + secureTextEntry.hashCode()
        result = 31 * result + multiline.hashCode()
        result = 31 * result + resizeMode
        result = 31 * result + testId.hashCode()
        result = 31 * result + drawListCorrupt.hashCode()
        result = 31 * result + (drawListBytes?.contentHashCode() ?: 0)
        return result
    }
}

/**
 * Zero-JSON decoder for the batched dirty-node `NVE_MET1` binary metadata slab produced by
 * `NveAndroidMetadataPacker.cpp`.
 *
 * Uses `ByteOrder.LITTLE_ENDIAN` structured `ByteBuffer` reads and a reusable UTF-8 scratch
 * buffer instead of manual bit-shifting or per-string temporary byte array allocations.
 */
object NveNodeMetadataCodec {
    const val HEADER_SIZE: Int = 32
    const val RECORD_HEADER_SIZE: Int = 28
    const val FORMAT_VERSION: Int = 1

    const val OFFSET_VERSION: Int = 8
    const val OFFSET_RECORD_COUNT: Int = 12
    const val OFFSET_FRAME_ID: Int = 16
    const val OFFSET_TOTAL_BYTES: Int = 24

    const val REC_OFFSET_NODE_ID: Int = 0
    const val REC_OFFSET_FLAGS: Int = 4
    const val REC_OFFSET_LABEL_LEN: Int = 8
    const val REC_OFFSET_ROLE_LEN: Int = 10
    const val REC_OFFSET_HINT_LEN: Int = 12
    const val REC_OFFSET_STATE_LEN: Int = 14
    const val REC_OFFSET_PLACEHOLDER_LEN: Int = 16
    const val REC_OFFSET_TEST_ID_LEN: Int = 18
    const val REC_OFFSET_DRAW_LIST_LEN: Int = 20

    const val FLAG_DISABLED: Int = 1 shl 0
    const val FLAG_SECURE_TEXT_ENTRY: Int = 1 shl 1
    const val FLAG_MULTILINE: Int = 1 shl 2
    const val FLAG_HAS_DRAW_LIST: Int = 1 shl 3
    const val FLAG_DRAW_LIST_CORRUPT: Int = 1 shl 4
    const val FLAG_ACCESSIBILITY_HIDDEN: Int = 1 shl 5
    const val RESIZE_MODE_SHIFT: Int = 8
    const val RESIZE_MODE_MASK: Int = 0x3

    private val MAGIC = byteArrayOf(
        'N'.code.toByte(),
        'V'.code.toByte(),
        'E'.code.toByte(),
        '_'.code.toByte(),
        'M'.code.toByte(),
        'E'.code.toByte(),
        'T'.code.toByte(),
        '1'.code.toByte(),
    )

    /**
     * Reusable UTF-8 decode scratch buffer for streaming `NVE_MET1` parsing.
     */
    class Utf8Scratch(initialCapacity: Int = 128) {
        private var bytes = ByteArray(maxOf(32, initialCapacity))

        fun read(leBuf: ByteBuffer, offset: Int, len: Int): String {
            if (len <= 0) return ""
            if (bytes.size < len) {
                bytes = ByteArray(maxOf(len, bytes.size shl 1))
            }
            leBuf.position(offset)
            leBuf.get(bytes, 0, len)
            return String(bytes, 0, len, StandardCharsets.UTF_8)
        }
    }

    /**
     * Decodes all dirty-node metadata records from [buffer] into [target], returning the array
     * of updated node IDs (empty if the buffer is empty, invalid, or contains 0 records).
     */
    fun decodeInto(
        buffer: ByteBuffer?,
        target: MutableMap<Int, NveNodeMetadata>,
        declaredBytes: Int = -1,
        expectedFrameId: Long? = null,
        scratch: Utf8Scratch = Utf8Scratch(),
    ): IntArray {
        val leBuf = validateHeader(buffer, declaredBytes, expectedFrameId) ?: return IntArray(0)
        val recordCount = leBuf.getInt(OFFSET_RECORD_COUNT)
        val totalBytes = leBuf.getInt(OFFSET_TOTAL_BYTES)

        val updatedIds = IntArray(recordCount)
        var written = 0
        var pos = HEADER_SIZE
        for (i in 0 until recordCount) {
            if (totalBytes - pos < RECORD_HEADER_SIZE) break
            val nodeId = leBuf.getInt(pos + REC_OFFSET_NODE_ID)
            val flags = leBuf.getInt(pos + REC_OFFSET_FLAGS)
            val labelLen = leBuf.getShort(pos + REC_OFFSET_LABEL_LEN).toInt() and 0xFFFF
            val roleLen = leBuf.getShort(pos + REC_OFFSET_ROLE_LEN).toInt() and 0xFFFF
            val hintLen = leBuf.getShort(pos + REC_OFFSET_HINT_LEN).toInt() and 0xFFFF
            val stateDescLen = leBuf.getShort(pos + REC_OFFSET_STATE_LEN).toInt() and 0xFFFF
            val placeholderLen = leBuf.getShort(pos + REC_OFFSET_PLACEHOLDER_LEN).toInt() and 0xFFFF
            val testIdLen = leBuf.getShort(pos + REC_OFFSET_TEST_ID_LEN).toInt() and 0xFFFF
            val drawListLen = leBuf.getInt(pos + REC_OFFSET_DRAW_LIST_LEN)
            pos += RECORD_HEADER_SIZE

            if (drawListLen < 0) break
            val payloadBytes = labelLen.toLong() + roleLen + hintLen + stateDescLen +
                placeholderLen + testIdLen + drawListLen
            if (payloadBytes > (totalBytes - pos).toLong()) break

            val label = scratch.read(leBuf, pos, labelLen); pos += labelLen
            val role = scratch.read(leBuf, pos, roleLen); pos += roleLen
            val hint = scratch.read(leBuf, pos, hintLen); pos += hintLen
            val stateDesc = scratch.read(leBuf, pos, stateDescLen); pos += stateDescLen
            val placeholder = scratch.read(leBuf, pos, placeholderLen); pos += placeholderLen
            val testId = scratch.read(leBuf, pos, testIdLen); pos += testIdLen

            var drawBytes: ByteArray? = null
            var drawCorrupt = (flags and FLAG_DRAW_LIST_CORRUPT) != 0
            if (drawListLen > 0) {
                if (NveDrawListRenderer.validate(leBuf, pos, drawListLen) == null) {
                    val copy = ByteArray(drawListLen)
                    leBuf.position(pos)
                    leBuf.get(copy, 0, drawListLen)
                    drawBytes = copy
                } else {
                    drawCorrupt = true
                }
                pos += drawListLen
            }

            target[nodeId] = NveNodeMetadata(
                accessibilityLabel = label,
                accessibilityRole = role,
                accessibilityHint = hint,
                accessibilityStateDescription = stateDesc,
                disabled = (flags and FLAG_DISABLED) != 0,
                accessibilityHidden = (flags and FLAG_ACCESSIBILITY_HIDDEN) != 0,
                placeholder = placeholder,
                secureTextEntry = (flags and FLAG_SECURE_TEXT_ENTRY) != 0,
                multiline = (flags and FLAG_MULTILINE) != 0,
                resizeMode = (flags ushr RESIZE_MODE_SHIFT) and RESIZE_MODE_MASK,
                testId = testId,
                drawListBytes = drawBytes,
                drawListCorrupt = drawCorrupt,
            )
            updatedIds[written++] = nodeId
        }
        return if (written == updatedIds.size) updatedIds else updatedIds.copyOf(written)
    }

    private fun validateHeader(
        buffer: ByteBuffer?,
        declaredBytes: Int,
        expectedFrameId: Long?,
    ): ByteBuffer? {
        if (buffer == null || buffer.limit() < HEADER_SIZE) return null
        for (i in MAGIC.indices) {
            if (buffer.get(i) != MAGIC[i]) return null
        }
        val leBuf = buffer.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val version = leBuf.getShort(OFFSET_VERSION).toInt() and 0xFFFF
        if (version != FORMAT_VERSION) return null
        val recordCount = leBuf.getInt(OFFSET_RECORD_COUNT)
        val slabFrameId = leBuf.getLong(OFFSET_FRAME_ID)
        val totalBytes = leBuf.getInt(OFFSET_TOTAL_BYTES)
        if (expectedFrameId != null && slabFrameId != expectedFrameId) return null
        val bound = if (declaredBytes > 0) minOf(declaredBytes, leBuf.limit()) else leBuf.limit()
        if (recordCount <= 0 || totalBytes < HEADER_SIZE || totalBytes > bound) return null
        if (recordCount.toLong() * RECORD_HEADER_SIZE.toLong() > (totalBytes - HEADER_SIZE).toLong()) {
            return null
        }
        return leBuf
    }
}
