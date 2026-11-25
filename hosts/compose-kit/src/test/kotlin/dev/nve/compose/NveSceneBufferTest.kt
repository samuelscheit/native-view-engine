package dev.nve.compose

import org.junit.jupiter.api.Assertions.assertArrayEquals
import org.junit.jupiter.api.Assertions.assertEquals
import org.junit.jupiter.api.Assertions.assertFalse
import org.junit.jupiter.api.Assertions.assertSame
import org.junit.jupiter.api.Assertions.assertThrows
import org.junit.jupiter.api.Assertions.assertTrue
import org.junit.jupiter.api.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

class NveSceneBufferTest {

    @Test
    fun parsesAll64ByteRenderNodeFieldsDirectZeroCopy() {
        val frame = SceneSlabBuilder.buildFrame(
            frameId = 7L,
            nodes = listOf(
                SceneSlabBuilder.NodeSpec(
                    nodeId = 1,
                    kind = NveSceneBuffer.KIND_ROOT,
                    flags = NveSceneBuffer.FLAG_CLIP_BOUNDS,
                    opacityU8 = 255,
                    x = 0f,
                    y = 0f,
                    width = 412f,
                    height = 915f,
                    bgRgba = 0x101820FF,
                    borderRadius = 12.5f,
                    borderWidth = 1.5f,
                    childIds = intArrayOf(2, 3),
                    zIndex = -1,
                ),
                SceneSlabBuilder.NodeSpec(
                    nodeId = 2,
                    kind = NveSceneBuffer.KIND_TEXT,
                    flags = NveSceneBuffer.FLAG_LEAF_TEXT or NveSceneBuffer.FLAG_INTERACTIVE,
                    opacityU8 = 128,
                    x = 16f,
                    y = 24f,
                    width = 200f,
                    height = 48f,
                    bgRgba = 0x223344FF,
                    textRgba = 0xAABBCCFF.toInt(),
                    borderRgba = 0xFFEEDDCC.toInt(),
                    borderRadius = 6f,
                    borderWidth = 2f,
                    fontSize = 18.5f,
                    text = "Hello NVE 🚀 — 日本語",
                    zIndex = 5,
                ),
            ),
            dirtyIndices = intArrayOf(1),
            removedIds = intArrayOf(99),
        )

        val reader = NveSceneBuffer()
        reader.attachFromMeta(frame.meta, frame.slabs)

        assertEquals(7L, reader.frameId)
        assertEquals(2, reader.nodeCount)
        assertEquals(1, reader.dirtyCount)
        assertEquals(1, reader.removedCount)
        assertEquals(1, reader.dirtyIndex(0))
        assertEquals(99, reader.removedId(0))

        // Node 0: root with 2 children
        assertEquals(1, reader.nodeId(0))
        assertEquals(NveSceneBuffer.KIND_ROOT, reader.kind(0))
        assertTrue(reader.clipsBounds(0))
        assertFalse(reader.isLeafText(0))
        assertFalse(reader.isInteractive(0))
        assertEquals(1.0f, reader.opacity(0), 1e-6f)
        assertEquals(412f, reader.width(0))
        assertEquals(915f, reader.height(0))
        assertEquals(12.5f, reader.borderRadius(0))
        assertEquals(1.5f, reader.borderWidth(0))
        assertEquals(-1, reader.zIndex(0))
        assertEquals(2, reader.childCount(0))
        assertEquals(2, reader.childId(0, 0))
        assertEquals(3, reader.childId(0, 1))
        assertEquals("", reader.readText(0))

        // Node 1: styled interactive leaf text with multibyte UTF-8
        assertEquals(2, reader.nodeId(1))
        assertEquals(NveSceneBuffer.KIND_TEXT, reader.kind(1))
        assertTrue(reader.isLeafText(1))
        assertTrue(reader.isInteractive(1))
        assertFalse(reader.clipsBounds(1))
        assertEquals(128, reader.opacityU8(1))
        assertEquals(128f / 255f, reader.opacity(1), 1e-6f)
        assertEquals(16f, reader.x(1))
        assertEquals(24f, reader.y(1))
        assertEquals(200f, reader.width(1))
        assertEquals(48f, reader.height(1))
        assertEquals(0x223344FF, reader.bgRgba(1))
        assertEquals(0xAABBCCFF.toInt(), reader.textRgba(1))
        assertEquals(0xFFEEDDCC.toInt(), reader.borderRgba(1))
        assertEquals(6f, reader.borderRadius(1))
        assertEquals(2f, reader.borderWidth(1))
        assertEquals(18.5f, reader.fontSize(1))
        assertEquals(5, reader.zIndex(1))
        assertEquals("Hello NVE 🚀 — 日本語", reader.readText(1))

        // Zero-copy verification: mutating the backing direct ByteBuffer is visible immediately.
        frame.slabs.nodes!!.putFloat(NveSceneBuffer.NODE_STRIDE + NveSceneBuffer.OFFSET_X, 77.25f)
        assertEquals(77.25f, reader.x(1))
    }

    @Test
    fun incrementalDirtyDiffOnlyMutatesDirtyNodesAndRemovesDeletedIds() {
        val reader = NveSceneBuffer()
        val mounted = LinkedHashMap<Int, NveRenderNodeSnapshot>()

        // Frame 1: initial mount of 3 nodes
        val frame1 = SceneSlabBuilder.buildFrame(
            frameId = 1L,
            nodes = listOf(
                SceneSlabBuilder.NodeSpec(nodeId = 1, kind = NveSceneBuffer.KIND_ROOT, childIds = intArrayOf(2, 3)),
                SceneSlabBuilder.NodeSpec(nodeId = 2, kind = NveSceneBuffer.KIND_TEXT, text = "Count: 0"),
                SceneSlabBuilder.NodeSpec(nodeId = 3, kind = NveSceneBuffer.KIND_PRESSABLE, text = "Old button"),
            ),
            dirtyIndices = intArrayOf(0, 1, 2),
        )
        reader.attachFromMeta(frame1.meta, frame1.slabs)
        val diff1 = reader.applyIncrementalDiff(mounted)

        assertEquals(1L, diff1.frameId)
        assertArrayEquals(intArrayOf(1, 2, 3), diff1.updatedNodeIds)
        assertArrayEquals(intArrayOf(), diff1.removedNodeIds)
        assertEquals(3, mounted.size)
        val untouchedRootRef = mounted[1]!!

        // Frame 2: node 1 clean, node 2 dirty with new text, node 3 removed
        val frame2 = SceneSlabBuilder.buildFrame(
            frameId = 2L,
            nodes = listOf(
                SceneSlabBuilder.NodeSpec(nodeId = 1, kind = NveSceneBuffer.KIND_ROOT, childIds = intArrayOf(2)),
                SceneSlabBuilder.NodeSpec(nodeId = 2, kind = NveSceneBuffer.KIND_TEXT, text = "Count: 1"),
            ),
            dirtyIndices = intArrayOf(1),
            removedIds = intArrayOf(3),
        )
        reader.attachFromMeta(frame2.meta, frame2.slabs)
        val diff2 = reader.applyIncrementalDiff(mounted)

        assertEquals(2L, diff2.frameId)
        assertArrayEquals(intArrayOf(2), diff2.updatedNodeIds)
        assertArrayEquals(intArrayOf(3), diff2.removedNodeIds)
        assertEquals(2, diff2.totalNodeCount)
        assertEquals(setOf(1, 2), mounted.keys)
        // Clean node 1 was never re-snapshotted
        assertSame(untouchedRootRef, mounted[1])
        assertEquals("Count: 1", mounted[2]!!.text)

        // Frame 3: clean no-op frame (0 dirty, 0 removed)
        val frame3 = SceneSlabBuilder.buildFrame(
            frameId = 3L,
            nodes = listOf(
                SceneSlabBuilder.NodeSpec(nodeId = 1, kind = NveSceneBuffer.KIND_ROOT),
                SceneSlabBuilder.NodeSpec(nodeId = 2, kind = NveSceneBuffer.KIND_TEXT, text = "Count: 1"),
            ),
            dirtyIndices = intArrayOf(),
            removedIds = intArrayOf(),
        )
        reader.attachFromMeta(frame3.meta, frame3.slabs)
        val diff3 = reader.applyIncrementalDiff(mounted)
        assertEquals(0, diff3.updatedNodeIds.size)
        assertEquals(0, diff3.removedNodeIds.size)
        assertEquals(2, mounted.size)
    }

    @Test
    fun rejectsSchemaHashMismatchAndPreservesPreviousState() {
        val reader = NveSceneBuffer()
        val valid = SceneSlabBuilder.buildFrame(
            frameId = 4L,
            nodes = listOf(SceneSlabBuilder.NodeSpec(nodeId = 10, text = "ok")),
        )
        reader.attachFromMeta(valid.meta, valid.slabs)
        assertEquals(4L, reader.frameId)

        val bad = SceneSlabBuilder.buildFrame(
            frameId = 5L,
            nodes = listOf(SceneSlabBuilder.NodeSpec(nodeId = 11)),
            schemaHash = NveSceneBuffer.SCHEMA_HASH xor 0xFFL,
        )
        val err = assertThrows(NveSceneValidationException.SchemaHashMismatch::class.java) {
            reader.attachFromMeta(bad.meta, bad.slabs)
        }
        assertEquals(NveSceneBuffer.SCHEMA_HASH, err.expected)
        assertEquals(NveSceneBuffer.SCHEMA_HASH xor 0xFFL, err.actual)

        // Previous valid frame remains intact after failed attach.
        assertEquals(4L, reader.frameId)
        assertEquals(10, reader.nodeId(0))
        assertEquals("ok", reader.readText(0))
    }

    @Test
    fun rejectsTruncatedNegativeOutOfBoundsAndInvalidUtf8Slabs() {
        val reader = NveSceneBuffer()

        // Truncated meta array
        assertThrows(NveSceneValidationException.TruncatedBuffer::class.java) {
            reader.attachFromMeta(longArrayOf(NveSceneBuffer.SCHEMA_HASH, 1L), NveSceneSlabs())
        }

        // Negative counts
        assertThrows(NveSceneValidationException.NegativeCount::class.java) {
            reader.attach(NveSceneBuffer.SCHEMA_HASH, 1L, -1, 0, 0, 0, 0, null, null, null, null, null)
        }
        assertThrows(NveSceneValidationException.NegativeCount::class.java) {
            reader.attach(NveSceneBuffer.SCHEMA_HASH, 1L, 0, -1, 0, 0, 0, null, null, null, null, null)
        }

        // Null nodes buffer when nodeCount > 0
        assertThrows(NveSceneValidationException.TruncatedBuffer::class.java) {
            reader.attach(NveSceneBuffer.SCHEMA_HASH, 1L, 1, 0, 0, 0, 0, null, null, null, null, null)
        }

        // Undersized nodes buffer (63 bytes < 64 bytes stride)
        val shortNodes = ByteBuffer.allocateDirect(63).order(ByteOrder.LITTLE_ENDIAN)
        assertThrows(NveSceneValidationException.TruncatedBuffer::class.java) {
            reader.attach(NveSceneBuffer.SCHEMA_HASH, 1L, 1, 0, 0, 0, 0, shortNodes, null, null, null, null)
        }

        // Out-of-bounds dirty index
        val oneNode = ByteBuffer.allocateDirect(64).order(ByteOrder.LITTLE_ENDIAN)
        val badDirty = ByteBuffer.allocateDirect(4).order(ByteOrder.LITTLE_ENDIAN).putInt(1).apply { flip() }
        assertThrows(NveSceneValidationException.DirtyIndexOutOfBounds::class.java) {
            reader.attach(NveSceneBuffer.SCHEMA_HASH, 1L, 1, 1, 0, 0, 0, oneNode, null, null, badDirty, null)
        }

        // Node text range out of bounds
        val badTextNode = ByteBuffer.allocateDirect(64).order(ByteOrder.LITTLE_ENDIAN)
        badTextNode.putInt(NveSceneBuffer.OFFSET_NODE_ID, 42)
        badTextNode.putInt(NveSceneBuffer.OFFSET_TEXT_OFFSET, 4)
        badTextNode.putShort(NveSceneBuffer.OFFSET_TEXT_LEN, 8)
        val tinyText = ByteBuffer.allocateDirect(6)
        assertThrows(NveSceneValidationException.TextRangeOutOfBounds::class.java) {
            reader.attach(NveSceneBuffer.SCHEMA_HASH, 1L, 1, 0, 0, 6, 0, badTextNode, tinyText, null, null, null)
        }

        // Node children range out of bounds
        val badChildrenNode = ByteBuffer.allocateDirect(64).order(ByteOrder.LITTLE_ENDIAN)
        badChildrenNode.putInt(NveSceneBuffer.OFFSET_NODE_ID, 43)
        badChildrenNode.putShort(NveSceneBuffer.OFFSET_CHILD_COUNT, 3)
        badChildrenNode.putInt(NveSceneBuffer.OFFSET_CHILD_OFFSET, 1)
        val twoChildren = ByteBuffer.allocateDirect(8)
        assertThrows(NveSceneValidationException.ChildrenRangeOutOfBounds::class.java) {
            reader.attach(NveSceneBuffer.SCHEMA_HASH, 1L, 1, 0, 0, 0, 2, badChildrenNode, null, twoChildren, null, null)
        }

        // Invalid UTF-8 bytes in text slab
        val badUtf8Node = ByteBuffer.allocateDirect(64).order(ByteOrder.LITTLE_ENDIAN)
        badUtf8Node.putInt(NveSceneBuffer.OFFSET_NODE_ID, 5)
        badUtf8Node.putInt(NveSceneBuffer.OFFSET_TEXT_OFFSET, 0)
        badUtf8Node.putShort(NveSceneBuffer.OFFSET_TEXT_LEN, 2)
        val badUtf8Slab = ByteBuffer.allocateDirect(2).put(0xFF.toByte()).put(0xFE.toByte()).apply { flip() }
        val utf8Err = assertThrows(NveSceneValidationException.InvalidUtf8::class.java) {
            reader.attach(NveSceneBuffer.SCHEMA_HASH, 1L, 1, 0, 0, 2, 0, badUtf8Node, badUtf8Slab, null, null, null)
        }
        assertEquals(5, utf8Err.nodeId)
    }

    @Test
    fun accessorOutOfBoundsThrowsIndexOutOfBounds() {
        val frame = SceneSlabBuilder.buildFrame(
            frameId = 1L,
            nodes = listOf(SceneSlabBuilder.NodeSpec(nodeId = 1, childIds = intArrayOf(2))),
        )
        val reader = NveSceneBuffer()
        reader.attachFromMeta(frame.meta, frame.slabs)

        assertThrows(IndexOutOfBoundsException::class.java) { reader.nodeId(-1) }
        assertThrows(IndexOutOfBoundsException::class.java) { reader.nodeId(1) }
        assertThrows(IndexOutOfBoundsException::class.java) { reader.childId(0, 1) }
        assertThrows(IndexOutOfBoundsException::class.java) { reader.dirtyIndex(1) }
        assertThrows(IndexOutOfBoundsException::class.java) { reader.removedId(0) }
    }
}
