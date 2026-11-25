package dev.nve.compose

import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/**
 * Direct `ByteBuffer` slabs backing a single `NveSceneView` frame from JNI (`NewDirectByteBuffer`).
 */
data class NveSceneSlabs(
    val nodes: ByteBuffer? = null,
    val textSlab: ByteBuffer? = null,
    val childrenSlab: ByteBuffer? = null,
    val dirtyIndices: ByteBuffer? = null,
    val removedIds: ByteBuffer? = null,
    val metadataSlab: ByteBuffer? = null,
)

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
 * `NveAndroidSurfaceJni.cpp`.
 */
object NveNodeMetadataCodec {
    const val HEADER_SIZE: Int = 32
    const val RECORD_HEADER_SIZE: Int = 28
    const val FORMAT_VERSION: Int = 1

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
     * Decodes all dirty-node metadata records from [buffer] into [target], returning the list
     * of updated node IDs (empty if the buffer is empty, invalid, or contains 0 records).
     */
    fun decodeInto(
        buffer: ByteBuffer?,
        target: MutableMap<Int, NveNodeMetadata>,
    ): IntArray {
        if (buffer == null || buffer.limit() < HEADER_SIZE) return IntArray(0)
        for (i in MAGIC.indices) {
            if (buffer.get(i) != MAGIC[i]) return IntArray(0)
        }
        val version = readU16Le(buffer, 8)
        if (version != FORMAT_VERSION) return IntArray(0)
        val recordCount = readI32Le(buffer, 12)
        val totalBytes = readI32Le(buffer, 24)
        if (recordCount <= 0 || totalBytes < HEADER_SIZE || totalBytes > buffer.limit()) {
            return IntArray(0)
        }
        if (recordCount.toLong() * RECORD_HEADER_SIZE.toLong() > (totalBytes - HEADER_SIZE).toLong()) {
            return IntArray(0)
        }

        val updatedIds = IntArray(recordCount)
        var written = 0
        var pos = HEADER_SIZE
        for (i in 0 until recordCount) {
            if (totalBytes - pos < RECORD_HEADER_SIZE) break
            val nodeId = readI32Le(buffer, pos)
            val flags = readI32Le(buffer, pos + 4)
            val labelLen = readU16Le(buffer, pos + 8)
            val roleLen = readU16Le(buffer, pos + 10)
            val hintLen = readU16Le(buffer, pos + 12)
            val stateDescLen = readU16Le(buffer, pos + 14)
            val placeholderLen = readU16Le(buffer, pos + 16)
            val testIdLen = readU16Le(buffer, pos + 18)
            val drawListLen = readI32Le(buffer, pos + 20)
            pos += RECORD_HEADER_SIZE

            if (drawListLen < 0) break
            val payloadBytes = labelLen.toLong() + roleLen + hintLen + stateDescLen +
                placeholderLen + testIdLen + drawListLen
            if (payloadBytes > (totalBytes - pos).toLong()) break

            val label = readUtf8(buffer, pos, labelLen); pos += labelLen
            val role = readUtf8(buffer, pos, roleLen); pos += roleLen
            val hint = readUtf8(buffer, pos, hintLen); pos += hintLen
            val stateDesc = readUtf8(buffer, pos, stateDescLen); pos += stateDescLen
            val placeholder = readUtf8(buffer, pos, placeholderLen); pos += placeholderLen
            val testId = readUtf8(buffer, pos, testIdLen); pos += testIdLen

            var drawBytes: ByteArray? = null
            var drawCorrupt = (flags and FLAG_DRAW_LIST_CORRUPT) != 0
            if (drawListLen > 0) {
                if (NveDrawListRenderer.validate(buffer, pos, drawListLen) == null) {
                    val copy = ByteArray(drawListLen)
                    for (b in 0 until drawListLen) {
                        copy[b] = buffer.get(pos + b)
                    }
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

    private fun readU16Le(buf: ByteBuffer, pos: Int): Int =
        (buf.get(pos).toInt() and 0xFF) or ((buf.get(pos + 1).toInt() and 0xFF) shl 8)

    private fun readI32Le(buf: ByteBuffer, pos: Int): Int =
        (buf.get(pos).toInt() and 0xFF) or
            ((buf.get(pos + 1).toInt() and 0xFF) shl 8) or
            ((buf.get(pos + 2).toInt() and 0xFF) shl 16) or
            ((buf.get(pos + 3).toInt() and 0xFF) shl 24)

    private fun readUtf8(buf: ByteBuffer, pos: Int, len: Int): String {
        if (len <= 0) return ""
        val bytes = ByteArray(len)
        for (i in 0 until len) {
            bytes[i] = buf.get(pos + i)
        }
        return String(bytes, StandardCharsets.UTF_8)
    }
}

/**
 * Strict validation errors thrown when attaching a malformed or incompatible `NveSceneView` frame.
 */
sealed class NveSceneValidationException(message: String) : IllegalArgumentException(message) {
    class SchemaHashMismatch(val expected: Long, val actual: Long) :
        NveSceneValidationException(
            "NveSceneView schema hash mismatch: expected 0x${java.lang.Long.toHexString(expected)}, " +
                "got 0x${java.lang.Long.toHexString(actual)}",
        )

    class NegativeCount(val field: String, val value: Int) :
        NveSceneValidationException("Negative $field count: $value")

    class TruncatedBuffer(val slab: String, val requiredBytes: Long, val availableBytes: Long) :
        NveSceneValidationException(
            "Truncated $slab buffer: need $requiredBytes bytes, available $availableBytes bytes",
        )

    class DirtyIndexOutOfBounds(val dirtySlot: Int, val index: Int, val nodeCount: Int) :
        NveSceneValidationException(
            "Dirty index $index at slot $dirtySlot out of bounds for nodeCount=$nodeCount",
        )

    class TextRangeOutOfBounds(val nodeId: Int, val offset: Long, val length: Int, val slabLen: Int) :
        NveSceneValidationException(
            "Text range [$offset..${offset + length}) for node $nodeId exceeds textSlabLen=$slabLen",
        )

    class ChildrenRangeOutOfBounds(val nodeId: Int, val offset: Long, val count: Int, val slabLen: Int) :
        NveSceneValidationException(
            "Children range [$offset..${offset + count}) for node $nodeId exceeds childrenSlabLen=$slabLen",
        )

    class InvalidUtf8(val nodeId: Int, cause: Throwable) :
        NveSceneValidationException("Invalid UTF-8 text payload for node $nodeId: ${cause.message}") {
        init {
            initCause(cause)
        }
    }
}

/**
 * Immutable snapshot of one 64-byte `NveRenderNode`, materialized incrementally only for dirty indices.
 */
data class NveRenderNodeSnapshot(
    val nodeId: Int,
    val kind: Int,
    val flags: Int,
    val opacityU8: Int,
    val x: Float,
    val y: Float,
    val width: Float,
    val height: Float,
    val bgRgba: Int,
    val textRgba: Int,
    val borderRgba: Int,
    val borderRadius: Float,
    val borderWidth: Float,
    val fontSize: Float,
    val text: String,
    val childIds: IntArray,
    val zIndex: Int,
    val metadata: NveNodeMetadata = NveNodeMetadata.EMPTY,
    val drawListBytes: ByteArray? = null,
) {
    val opacity: Float get() = opacityU8 / 255.0f
    val isLeafText: Boolean get() = (flags and NveSceneBuffer.FLAG_LEAF_TEXT) != 0
    val isInteractive: Boolean get() = (flags and NveSceneBuffer.FLAG_INTERACTIVE) != 0
    val clipsBounds: Boolean get() = (flags and NveSceneBuffer.FLAG_CLIP_BOUNDS) != 0

    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is NveRenderNodeSnapshot) return false
        return nodeId == other.nodeId &&
            kind == other.kind &&
            flags == other.flags &&
            opacityU8 == other.opacityU8 &&
            x.toRawBits() == other.x.toRawBits() &&
            y.toRawBits() == other.y.toRawBits() &&
            width.toRawBits() == other.width.toRawBits() &&
            height.toRawBits() == other.height.toRawBits() &&
            bgRgba == other.bgRgba &&
            textRgba == other.textRgba &&
            borderRgba == other.borderRgba &&
            borderRadius.toRawBits() == other.borderRadius.toRawBits() &&
            borderWidth.toRawBits() == other.borderWidth.toRawBits() &&
            fontSize.toRawBits() == other.fontSize.toRawBits() &&
            text == other.text &&
            childIds.contentEquals(other.childIds) &&
            zIndex == other.zIndex &&
            metadata == other.metadata &&
            ((drawListBytes == null && other.drawListBytes == null) ||
                (drawListBytes != null && other.drawListBytes != null &&
                    drawListBytes.contentEquals(other.drawListBytes)))
    }

    override fun hashCode(): Int {
        var result = nodeId
        result = 31 * result + kind
        result = 31 * result + flags
        result = 31 * result + opacityU8
        result = 31 * result + x.toRawBits()
        result = 31 * result + y.toRawBits()
        result = 31 * result + width.toRawBits()
        result = 31 * result + height.toRawBits()
        result = 31 * result + bgRgba
        result = 31 * result + textRgba
        result = 31 * result + borderRgba
        result = 31 * result + borderRadius.toRawBits()
        result = 31 * result + borderWidth.toRawBits()
        result = 31 * result + fontSize.toRawBits()
        result = 31 * result + text.hashCode()
        result = 31 * result + childIds.contentHashCode()
        result = 31 * result + zIndex
        result = 31 * result + metadata.hashCode()
        result = 31 * result + (drawListBytes?.contentHashCode() ?: 0)
        return result
    }
}

/**
 * Summary of an incremental dirty-index diff applied from an [NveSceneBuffer] frame.
 */
data class NveSceneDiff(
    val frameId: Long,
    val updatedNodeIds: IntArray,
    val removedNodeIds: IntArray,
    val totalNodeCount: Int,
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is NveSceneDiff) return false
        return frameId == other.frameId &&
            updatedNodeIds.contentEquals(other.updatedNodeIds) &&
            removedNodeIds.contentEquals(other.removedNodeIds) &&
            totalNodeCount == other.totalNodeCount
    }

    override fun hashCode(): Int {
        var result = frameId.hashCode()
        result = 31 * result + updatedNodeIds.contentHashCode()
        result = 31 * result + removedNodeIds.contentHashCode()
        result = 31 * result + totalNodeCount
        return result
    }
}

/**
 * Zero-copy, zero-JSON direct `ByteBuffer` reader over 64-byte `NveRenderNode` + `NveSceneView`
 * slabs matching `crates/nve-abi/src/scene.rs` and `cxx/nve-fabric-bridge/include/nve_scene_abi.generated.h`.
 */
class NveSceneBuffer {
    companion object {
        const val SCHEMA_HASH: Long = 0x00F2CF037DB50CC8L
        const val NODE_STRIDE: Int = 64
        const val META_FIELD_COUNT: Int = 7

        // NveRenderNode::flags bits
        const val FLAG_LEAF_TEXT: Int = 1 shl 0
        const val FLAG_INTERACTIVE: Int = 1 shl 1
        const val FLAG_CLIP_BOUNDS: Int = 1 shl 2

        // Compact uint8 NveRenderNode::kind discriminants
        const val KIND_ROOT: Int = 0
        const val KIND_VIEW: Int = 1
        const val KIND_TEXT: Int = 2
        const val KIND_SCROLL_VIEW: Int = 3
        const val KIND_IMAGE: Int = 4
        const val KIND_TEXT_INPUT: Int = 5
        const val KIND_PRESSABLE: Int = 6
        const val KIND_SKIA_CANVAS: Int = 7
        const val KIND_CUSTOM: Int = 255

        // Exact byte offsets within one 64-byte NveRenderNode
        const val OFFSET_NODE_ID: Int = 0
        const val OFFSET_KIND: Int = 4
        const val OFFSET_FLAGS: Int = 5
        const val OFFSET_OPACITY_U8: Int = 6
        const val OFFSET_PAD0: Int = 7
        const val OFFSET_X: Int = 8
        const val OFFSET_Y: Int = 12
        const val OFFSET_WIDTH: Int = 16
        const val OFFSET_HEIGHT: Int = 20
        const val OFFSET_BG_RGBA: Int = 24
        const val OFFSET_TEXT_RGBA: Int = 28
        const val OFFSET_BORDER_RGBA: Int = 32
        const val OFFSET_BORDER_RADIUS: Int = 36
        const val OFFSET_BORDER_WIDTH: Int = 40
        const val OFFSET_FONT_SIZE: Int = 44
        const val OFFSET_TEXT_OFFSET: Int = 48
        const val OFFSET_TEXT_LEN: Int = 52
        const val OFFSET_CHILD_COUNT: Int = 54
        const val OFFSET_CHILD_OFFSET: Int = 56
        const val OFFSET_Z_INDEX: Int = 60
    }

    var frameId: Long = 0L
        private set
    var nodeCount: Int = 0
        private set
    var dirtyCount: Int = 0
        private set
    var removedCount: Int = 0
        private set
    var textSlabLen: Int = 0
        private set
    var childrenSlabLen: Int = 0
        private set

    private var nodesBuf: ByteBuffer? = null
    private var textSlabBuf: ByteBuffer? = null
    private var childrenSlabBuf: ByteBuffer? = null
    private var dirtyIndicesBuf: ByteBuffer? = null
    private var removedIdsBuf: ByteBuffer? = null
    private var textScratch: ByteArray = ByteArray(256)

    /**
     * Rebinds this reusable reader from the 7-element metadata array returned by JNI
     * `nativeRenderFrame` (`[schema_hash, frame_id, node_count, dirty_count, removed_count,
     * text_slab_len, children_slab_len]`) and the 5 direct slab buffers.
     */
    fun attachFromMeta(meta: LongArray, slabs: NveSceneSlabs) {
        if (meta.size < META_FIELD_COUNT) {
            throw NveSceneValidationException.TruncatedBuffer(
                slab = "meta",
                requiredBytes = META_FIELD_COUNT.toLong() * 8L,
                availableBytes = meta.size.toLong() * 8L,
            )
        }
        attach(
            schemaHash = meta[0],
            frameId = meta[1],
            nodeCount = meta[2].toIntExact("nodeCount"),
            dirtyCount = meta[3].toIntExact("dirtyCount"),
            removedCount = meta[4].toIntExact("removedCount"),
            textSlabLen = meta[5].toIntExact("textSlabLen"),
            childrenSlabLen = meta[6].toIntExact("childrenSlabLen"),
            nodesBuf = slabs.nodes,
            textSlabBuf = slabs.textSlab,
            childrenSlabBuf = slabs.childrenSlab,
            dirtyIndicesBuf = slabs.dirtyIndices,
            removedIdsBuf = slabs.removedIds,
        )
    }

    /**
     * Rebinds this reusable reader to a new frame's direct buffers without copying any node records.
     * Validates schema hash, slab capacities, dirty indices, node slab ranges, and UTF-8 text.
     */
    fun attach(
        schemaHash: Long,
        frameId: Long,
        nodeCount: Int,
        dirtyCount: Int,
        removedCount: Int,
        textSlabLen: Int,
        childrenSlabLen: Int,
        nodesBuf: ByteBuffer?,
        textSlabBuf: ByteBuffer?,
        childrenSlabBuf: ByteBuffer?,
        dirtyIndicesBuf: ByteBuffer?,
        removedIdsBuf: ByteBuffer?,
    ) {
        if (schemaHash != SCHEMA_HASH) {
            throw NveSceneValidationException.SchemaHashMismatch(SCHEMA_HASH, schemaHash)
        }
        if (nodeCount < 0) throw NveSceneValidationException.NegativeCount("nodeCount", nodeCount)
        if (dirtyCount < 0) throw NveSceneValidationException.NegativeCount("dirtyCount", dirtyCount)
        if (removedCount < 0) throw NveSceneValidationException.NegativeCount("removedCount", removedCount)
        if (textSlabLen < 0) throw NveSceneValidationException.NegativeCount("textSlabLen", textSlabLen)
        if (childrenSlabLen < 0) throw NveSceneValidationException.NegativeCount("childrenSlabLen", childrenSlabLen)

        val nodes = normalizeSlab("nodes", nodesBuf, nodeCount.toLong() * NODE_STRIDE)
        val text = normalizeSlab("textSlab", textSlabBuf, textSlabLen.toLong())
        val children = normalizeSlab("childrenSlab", childrenSlabBuf, childrenSlabLen.toLong() * 4L)
        val dirty = normalizeSlab("dirtyIndices", dirtyIndicesBuf, dirtyCount.toLong() * 4L)
        val removed = normalizeSlab("removedIds", removedIdsBuf, removedCount.toLong() * 4L)

        // Validate dirty indices before mutating reader state so failed attach leaves previous state intact.
        if (dirtyCount > 0 && dirty != null) {
            for (slot in 0 until dirtyCount) {
                val idx = dirty.getInt(slot shl 2)
                if (idx < 0 || idx >= nodeCount) {
                    throw NveSceneValidationException.DirtyIndexOutOfBounds(slot, idx, nodeCount)
                }
            }
        }

        // Validate every node's text/children slab ranges and strict UTF-8 text slices.
        if (nodeCount > 0 && nodes != null) {
            val utf8Decoder = StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
            for (idx in 0 until nodeCount) {
                val base = idx * NODE_STRIDE
                val nid = nodes.getInt(base + OFFSET_NODE_ID)
                val nodeKind = nodes.get(base + OFFSET_KIND).toInt() and 0xFF

                val tLen = nodes.getShort(base + OFFSET_TEXT_LEN).toInt() and 0xFFFF
                if (tLen > 0) {
                    val tOff = nodes.getInt(base + OFFSET_TEXT_OFFSET).toLong() and 0xFFFF_FFFFL
                    if (tOff + tLen > textSlabLen.toLong() || text == null) {
                        throw NveSceneValidationException.TextRangeOutOfBounds(nid, tOff, tLen, textSlabLen)
                    }
                    // `KIND_SKIA_CANVAS` may carry binary `NVE_DRAW1` bytes directly in `text_slab`.
                    val isBinaryDrawList = nodeKind == KIND_SKIA_CANVAS &&
                        NveDrawListRenderer.hasMagic(text, tOff.toInt(), tLen)
                    if (!isBinaryDrawList) {
                        val slice = text.duplicate()
                        slice.position(tOff.toInt())
                        slice.limit(tOff.toInt() + tLen)
                        utf8Decoder.reset()
                        try {
                            utf8Decoder.decode(slice)
                        } catch (e: CharacterCodingException) {
                            throw NveSceneValidationException.InvalidUtf8(nid, e)
                        }
                    }
                }

                val cCount = nodes.getShort(base + OFFSET_CHILD_COUNT).toInt() and 0xFFFF
                if (cCount > 0) {
                    val cOff = nodes.getInt(base + OFFSET_CHILD_OFFSET).toLong() and 0xFFFF_FFFFL
                    if (cOff + cCount > childrenSlabLen.toLong() || children == null) {
                        throw NveSceneValidationException.ChildrenRangeOutOfBounds(
                            nid,
                            cOff,
                            cCount,
                            childrenSlabLen,
                        )
                    }
                }
            }
        }

        this.frameId = frameId
        this.nodeCount = nodeCount
        this.dirtyCount = dirtyCount
        this.removedCount = removedCount
        this.textSlabLen = textSlabLen
        this.childrenSlabLen = childrenSlabLen
        this.nodesBuf = nodes
        this.textSlabBuf = text
        this.childrenSlabBuf = children
        this.dirtyIndicesBuf = dirty
        this.removedIdsBuf = removed
    }

    fun clear() {
        frameId = 0L
        nodeCount = 0
        dirtyCount = 0
        removedCount = 0
        textSlabLen = 0
        childrenSlabLen = 0
        nodesBuf = null
        textSlabBuf = null
        childrenSlabBuf = null
        dirtyIndicesBuf = null
        removedIdsBuf = null
    }

    fun dirtyIndex(i: Int): Int {
        checkRange(i, dirtyCount, "dirtyIndex")
        return dirtyIndicesBuf!!.getInt(i shl 2)
    }

    fun removedId(i: Int): Int {
        checkRange(i, removedCount, "removedId")
        return removedIdsBuf!!.getInt(i shl 2)
    }

    fun nodeId(idx: Int): Int = nodesBufAt(idx).getInt(idx * NODE_STRIDE + OFFSET_NODE_ID)
    fun kind(idx: Int): Int = nodesBufAt(idx).get(idx * NODE_STRIDE + OFFSET_KIND).toInt() and 0xFF
    fun flags(idx: Int): Int = nodesBufAt(idx).get(idx * NODE_STRIDE + OFFSET_FLAGS).toInt() and 0xFF
    fun opacityU8(idx: Int): Int = nodesBufAt(idx).get(idx * NODE_STRIDE + OFFSET_OPACITY_U8).toInt() and 0xFF
    fun opacity(idx: Int): Float = opacityU8(idx) / 255.0f
    fun x(idx: Int): Float = nodesBufAt(idx).getFloat(idx * NODE_STRIDE + OFFSET_X)
    fun y(idx: Int): Float = nodesBufAt(idx).getFloat(idx * NODE_STRIDE + OFFSET_Y)
    fun width(idx: Int): Float = nodesBufAt(idx).getFloat(idx * NODE_STRIDE + OFFSET_WIDTH)
    fun height(idx: Int): Float = nodesBufAt(idx).getFloat(idx * NODE_STRIDE + OFFSET_HEIGHT)
    fun bgRgba(idx: Int): Int = nodesBufAt(idx).getInt(idx * NODE_STRIDE + OFFSET_BG_RGBA)
    fun textRgba(idx: Int): Int = nodesBufAt(idx).getInt(idx * NODE_STRIDE + OFFSET_TEXT_RGBA)
    fun borderRgba(idx: Int): Int = nodesBufAt(idx).getInt(idx * NODE_STRIDE + OFFSET_BORDER_RGBA)
    fun borderRadius(idx: Int): Float = nodesBufAt(idx).getFloat(idx * NODE_STRIDE + OFFSET_BORDER_RADIUS)
    fun borderWidth(idx: Int): Float = nodesBufAt(idx).getFloat(idx * NODE_STRIDE + OFFSET_BORDER_WIDTH)
    fun fontSize(idx: Int): Float = nodesBufAt(idx).getFloat(idx * NODE_STRIDE + OFFSET_FONT_SIZE)
    fun textOffset(idx: Int): Int = nodesBufAt(idx).getInt(idx * NODE_STRIDE + OFFSET_TEXT_OFFSET)
    fun textLen(idx: Int): Int = nodesBufAt(idx).getShort(idx * NODE_STRIDE + OFFSET_TEXT_LEN).toInt() and 0xFFFF
    fun childCount(idx: Int): Int = nodesBufAt(idx).getShort(idx * NODE_STRIDE + OFFSET_CHILD_COUNT).toInt() and 0xFFFF
    fun childOffset(idx: Int): Int = nodesBufAt(idx).getInt(idx * NODE_STRIDE + OFFSET_CHILD_OFFSET)
    fun zIndex(idx: Int): Int = nodesBufAt(idx).getInt(idx * NODE_STRIDE + OFFSET_Z_INDEX)

    fun isLeafText(idx: Int): Boolean = (flags(idx) and FLAG_LEAF_TEXT) != 0
    fun isInteractive(idx: Int): Boolean = (flags(idx) and FLAG_INTERACTIVE) != 0
    fun clipsBounds(idx: Int): Boolean = (flags(idx) and FLAG_CLIP_BOUNDS) != 0

    fun childId(nodeIdx: Int, childPos: Int): Int {
        val count = childCount(nodeIdx)
        checkRange(childPos, count, "childPos")
        val offset = childOffset(nodeIdx) + childPos
        return childrenSlabBuf!!.getInt(offset shl 2)
    }

    fun readText(nodeIdx: Int): String {
        val len = textLen(nodeIdx)
        val slab = textSlabBuf
        if (len == 0 || slab == null) return ""
        val off = textOffset(nodeIdx)
        if (off < 0 || off + len > textSlabLen) return ""
        if (textScratch.size < len) {
            textScratch = ByteArray(maxOf(len, textScratch.size shl 1))
        }
        val dup = slab.duplicate()
        dup.position(off)
        dup.get(textScratch, 0, len)
        return String(textScratch, 0, len, StandardCharsets.UTF_8)
    }

    fun hasDrawListInTextSlab(nodeIdx: Int): Boolean {
        val len = textLen(nodeIdx)
        val slab = textSlabBuf ?: return false
        if (len < NveDrawListRenderer.HEADER_SIZE) return false
        val off = textOffset(nodeIdx)
        if (off < 0 || off + len > textSlabLen) return false
        return NveDrawListRenderer.hasMagic(slab, off, len)
    }

    fun readRawBytes(nodeIdx: Int): ByteArray? {
        val len = textLen(nodeIdx)
        val slab = textSlabBuf
        if (len <= 0 || slab == null) return null
        val off = textOffset(nodeIdx)
        if (off < 0 || off + len > textSlabLen) return null
        val out = ByteArray(len)
        val dup = slab.duplicate()
        dup.position(off)
        dup.get(out, 0, len)
        return out
    }

    fun snapshotNode(
        idx: Int,
        metadataById: Map<Int, NveNodeMetadata> = emptyMap(),
    ): NveRenderNodeSnapshot {
        val count = childCount(idx)
        val children = if (count == 0) {
            IntArray(0)
        } else {
            val baseOffset = childOffset(idx)
            val slab = childrenSlabBuf!!
            IntArray(count) { pos -> slab.getInt((baseOffset + pos) shl 2) }
        }
        val nid = nodeId(idx)
        val nodeKind = kind(idx)
        val meta = metadataById[nid] ?: NveNodeMetadata.EMPTY
        val isCanvasDrawBytes = nodeKind == KIND_SKIA_CANVAS && hasDrawListInTextSlab(idx)
        val textVal = if (isCanvasDrawBytes) "" else readText(idx)
        val drawBytes = if (isCanvasDrawBytes) {
            readRawBytes(idx)
        } else {
            meta.drawListBytes
        }
        return NveRenderNodeSnapshot(
            nodeId = nid,
            kind = nodeKind,
            flags = flags(idx),
            opacityU8 = opacityU8(idx),
            x = x(idx),
            y = y(idx),
            width = width(idx),
            height = height(idx),
            bgRgba = bgRgba(idx),
            textRgba = textRgba(idx),
            borderRgba = borderRgba(idx),
            borderRadius = borderRadius(idx),
            borderWidth = borderWidth(idx),
            fontSize = fontSize(idx),
            text = textVal,
            childIds = children,
            zIndex = zIndex(idx),
            metadata = meta,
            drawListBytes = drawBytes,
        )
    }

    /**
     * Applies only `removedIds`, `dirtyIndices`, and optional batched `metadataSlab` updates onto
     * [mountedNodes] without iterating clean nodes, returning the frame's incremental [NveSceneDiff].
     */
    fun applyIncrementalDiff(
        mountedNodes: MutableMap<Int, NveRenderNodeSnapshot>,
        metadataById: MutableMap<Int, NveNodeMetadata>? = null,
        metadataSlab: ByteBuffer? = null,
    ): NveSceneDiff {
        val removed = IntArray(removedCount) { i ->
            val id = removedId(i)
            mountedNodes.remove(id)
            metadataById?.remove(id)
            id
        }
        val metaUpdatedIds = if (metadataSlab != null && metadataById != null) {
            NveNodeMetadataCodec.decodeInto(metadataSlab, metadataById)
        } else {
            IntArray(0)
        }
        val effectiveMeta = metadataById ?: emptyMap()
        val updated = IntArray(dirtyCount) { i ->
            val idx = dirtyIndex(i)
            val snap = snapshotNode(idx, effectiveMeta)
            mountedNodes[snap.nodeId] = snap
            snap.nodeId
        }
        // Apply raw-prop-only metadata changes (e.g., accessibilityLabel or drawList updated
        // without changing layout/style) onto already-mounted nodes without touching clean nodes.
        if (metaUpdatedIds.isNotEmpty()) {
            for (nid in metaUpdatedIds) {
                val existing = mountedNodes[nid] ?: continue
                val nextMeta = effectiveMeta[nid] ?: NveNodeMetadata.EMPTY
                val nextDrawBytes = if (existing.kind == KIND_SKIA_CANVAS && nextMeta.drawListBytes != null) {
                    nextMeta.drawListBytes
                } else if (existing.kind == KIND_SKIA_CANVAS && nextMeta.drawListCorrupt) {
                    null
                } else {
                    existing.drawListBytes
                }
                if (existing.metadata != nextMeta ||
                    !((existing.drawListBytes == null && nextDrawBytes == null) ||
                        (existing.drawListBytes != null && nextDrawBytes != null &&
                            existing.drawListBytes.contentEquals(nextDrawBytes)))
                ) {
                    mountedNodes[nid] = existing.copy(
                        metadata = nextMeta,
                        drawListBytes = nextDrawBytes,
                    )
                }
            }
        }
        return NveSceneDiff(
            frameId = frameId,
            updatedNodeIds = updated,
            removedNodeIds = removed,
            totalNodeCount = nodeCount,
        )
    }

    private fun nodesBufAt(idx: Int): ByteBuffer {
        checkRange(idx, nodeCount, "nodeIndex")
        return nodesBuf!!
    }

    private fun checkRange(index: Int, limit: Int, label: String) {
        if (index < 0 || index >= limit) {
            throw IndexOutOfBoundsException("$label=$index out of bounds for count=$limit")
        }
    }

    private fun normalizeSlab(name: String, buffer: ByteBuffer?, requiredBytes: Long): ByteBuffer? {
        if (requiredBytes == 0L) {
            return buffer?.duplicate()?.order(ByteOrder.LITTLE_ENDIAN)
        }
        val available = buffer?.limit()?.toLong() ?: 0L
        if (buffer == null || available < requiredBytes) {
            throw NveSceneValidationException.TruncatedBuffer(name, requiredBytes, available)
        }
        return buffer.duplicate().order(ByteOrder.LITTLE_ENDIAN)
    }

    private fun Long.toIntExact(field: String): Int {
        if (this < 0L) throw NveSceneValidationException.NegativeCount(field, this.toInt())
        if (this > Int.MAX_VALUE.toLong()) {
            throw NveSceneValidationException.TruncatedBuffer(field, this, Int.MAX_VALUE.toLong())
        }
        return this.toInt()
    }
}
