package dev.nve.compose

import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets

/**
 * Retained per-node accessibility, `TextInput`, `Image`, and `SkiaCanvas` metadata decoded from
 * fixed-stride 64-byte `NveNodeMeta` records + a flat companion byte blob per dirty frame
 * (binary FFI, zero per-prop JNI chatter).
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
 * Direct-slab reader for fixed-stride 64-byte `NveNodeMeta` records + companion byte blob
 * produced by `NveAndroidMetadataPacker.cpp` (layout locked by `packages/core/abi/src/scene.rs`
 * and `nve_scene_abi.generated.h`).
 */
object NveNodeMetadataCodec {
    const val META_STRIDE: Int = 64

    const val META_OFFSET_NODE_ID: Int = 0
    const val META_OFFSET_FLAGS: Int = 4
    const val META_OFFSET_LABEL_OFFSET: Int = 8
    const val META_OFFSET_LABEL_LEN: Int = 12
    const val META_OFFSET_ROLE_LEN: Int = 14
    const val META_OFFSET_ROLE_OFFSET: Int = 16
    const val META_OFFSET_HINT_OFFSET: Int = 20
    const val META_OFFSET_HINT_LEN: Int = 24
    const val META_OFFSET_STATE_DESC_LEN: Int = 26
    const val META_OFFSET_STATE_DESC_OFFSET: Int = 28
    const val META_OFFSET_PLACEHOLDER_OFFSET: Int = 32
    const val META_OFFSET_PLACEHOLDER_LEN: Int = 36
    const val META_OFFSET_TEST_ID_LEN: Int = 38
    const val META_OFFSET_TEST_ID_OFFSET: Int = 40
    const val META_OFFSET_DRAW_LIST_OFFSET: Int = 44
    const val META_OFFSET_DRAW_LIST_LEN: Int = 48

    const val FLAG_DISABLED: Int = 1 shl 0
    const val FLAG_SECURE_TEXT_ENTRY: Int = 1 shl 1
    const val FLAG_MULTILINE: Int = 1 shl 2
    const val FLAG_HAS_DRAW_LIST: Int = 1 shl 3
    const val FLAG_DRAW_LIST_CORRUPT: Int = 1 shl 4
    const val FLAG_ACCESSIBILITY_HIDDEN: Int = 1 shl 5
    const val RESIZE_MODE_SHIFT: Int = 8
    const val RESIZE_MODE_MASK: Int = 0x3

    /**
     * Reusable UTF-8 decode scratch buffer for reading slices from the companion metadata blob.
     */
    class Utf8Scratch(initialCapacity: Int = 128) {
        private var bytes = ByteArray(maxOf(32, initialCapacity))

        fun read(leBlob: ByteBuffer?, blobBound: Int, offset: Int, len: Int): String {
            if (len <= 0 || leBlob == null || offset < 0) return ""
            if (offset.toLong() + len.toLong() > blobBound.toLong()) return ""
            if (bytes.size < len) {
                bytes = ByteArray(maxOf(len, bytes.size shl 1))
            }
            leBlob.position(offset)
            leBlob.get(bytes, 0, len)
            return String(bytes, 0, len, StandardCharsets.UTF_8)
        }
    }

    /**
     * Decodes [recordCount] fixed-stride 64-byte `NveNodeMeta` records from [recordsBuf] indexing
     * strings and `NVE_DRAW1` draw-list slices in [blobBuf], writing into [target] and returning
     * the array of updated node IDs.
     */
    fun decodeInto(
        recordsBuf: ByteBuffer?,
        blobBuf: ByteBuffer?,
        target: MutableMap<Int, NveNodeMetadata>,
        recordCount: Int = -1,
        blobUsedBytes: Int = -1,
        scratch: Utf8Scratch = Utf8Scratch(),
    ): IntArray {
        if (recordsBuf == null) return IntArray(0)
        val effectiveCount = if (recordCount >= 0) recordCount else recordsBuf.limit() / META_STRIDE
        if (effectiveCount <= 0) return IntArray(0)
        val requiredBytes = effectiveCount.toLong() * META_STRIDE.toLong()
        if (recordsBuf.limit().toLong() < requiredBytes) return IntArray(0)

        val leRecs = recordsBuf.duplicate().order(ByteOrder.LITTLE_ENDIAN)
        val leBlob = blobBuf?.duplicate()?.order(ByteOrder.LITTLE_ENDIAN)
        val blobBound = when {
            leBlob == null -> 0
            blobUsedBytes >= 0 -> if (blobUsedBytes > leBlob.limit()) return IntArray(0) else blobUsedBytes
            else -> leBlob.limit()
        }

        val updatedIds = IntArray(effectiveCount)
        for (i in 0 until effectiveCount) {
            val base = i * META_STRIDE
            val nodeId = leRecs.getInt(base + META_OFFSET_NODE_ID)
            val flags = leRecs.getInt(base + META_OFFSET_FLAGS)
            val labelOff = leRecs.getInt(base + META_OFFSET_LABEL_OFFSET)
            val labelLen = leRecs.getShort(base + META_OFFSET_LABEL_LEN).toInt() and 0xFFFF
            val roleLen = leRecs.getShort(base + META_OFFSET_ROLE_LEN).toInt() and 0xFFFF
            val roleOff = leRecs.getInt(base + META_OFFSET_ROLE_OFFSET)
            val hintOff = leRecs.getInt(base + META_OFFSET_HINT_OFFSET)
            val hintLen = leRecs.getShort(base + META_OFFSET_HINT_LEN).toInt() and 0xFFFF
            val stateDescLen = leRecs.getShort(base + META_OFFSET_STATE_DESC_LEN).toInt() and 0xFFFF
            val stateDescOff = leRecs.getInt(base + META_OFFSET_STATE_DESC_OFFSET)
            val placeholderOff = leRecs.getInt(base + META_OFFSET_PLACEHOLDER_OFFSET)
            val placeholderLen = leRecs.getShort(base + META_OFFSET_PLACEHOLDER_LEN).toInt() and 0xFFFF
            val testIdLen = leRecs.getShort(base + META_OFFSET_TEST_ID_LEN).toInt() and 0xFFFF
            val testIdOff = leRecs.getInt(base + META_OFFSET_TEST_ID_OFFSET)
            val drawListOff = leRecs.getInt(base + META_OFFSET_DRAW_LIST_OFFSET)
            val drawListLen = leRecs.getInt(base + META_OFFSET_DRAW_LIST_LEN)

            val label = scratch.read(leBlob, blobBound, labelOff, labelLen)
            val role = scratch.read(leBlob, blobBound, roleOff, roleLen)
            val hint = scratch.read(leBlob, blobBound, hintOff, hintLen)
            val stateDesc = scratch.read(leBlob, blobBound, stateDescOff, stateDescLen)
            val placeholder = scratch.read(leBlob, blobBound, placeholderOff, placeholderLen)
            val testId = scratch.read(leBlob, blobBound, testIdOff, testIdLen)

            var drawBytes: ByteArray? = null
            var drawCorrupt = (flags and FLAG_DRAW_LIST_CORRUPT) != 0
            if (drawListLen > 0) {
                if (leBlob != null &&
                    drawListOff >= 0 &&
                    drawListOff.toLong() + drawListLen.toLong() <= blobBound.toLong() &&
                    NveDrawListRenderer.validate(leBlob, drawListOff, drawListLen) == null
                ) {
                    val copy = ByteArray(drawListLen)
                    leBlob.position(drawListOff)
                    leBlob.get(copy, 0, drawListLen)
                    drawBytes = copy
                } else {
                    drawCorrupt = true
                }
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
            updatedIds[i] = nodeId
        }
        return updatedIds
    }
}
