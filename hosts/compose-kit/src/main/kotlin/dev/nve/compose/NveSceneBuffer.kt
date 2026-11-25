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
 * slabs matching `packages/core/abi/src/scene.rs` and `cxx/nve-fabric-bridge/include/nve_scene_abi.generated.h`.
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
    private val metadataScratch = NveNodeMetadataCodec.Utf8Scratch()

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
            NveNodeMetadataCodec.decodeInto(metadataSlab, metadataById, scratch = metadataScratch)
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
