package dev.nve.compose

import dev.nve.host.FieldKind
import dev.nve.host.FieldSpec
import dev.nve.host.Key64
import dev.nve.host.RuntimeSceneReader
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets

/**
 * Builds a packed `NveRuntimeSceneView` frame in direct buffers (the same layout
 * `libnve_android_shell` hands to [RuntimeSceneReader]) so scene capture runs on the JVM.
 * Node index `i` has `NodeId = 1 << 24 | i`; index 1 is the root.
 */
class TestScene(private val capacity: Int = 16, private val frameId: Long = 1L, private val epoch: Int = 0) {
    class Table(val key: Key64, val slot: Int, val words: Int) {
        val rows = LinkedHashMap<Int, IntArray>()
    }

    private val parents = IntArray(capacity)
    private val children = HashMap<Int, MutableList<Int>>()
    private val rects = HashMap<Int, FloatArray>()
    private val live = BooleanArray(capacity)
    private val tables = ArrayList<Table>()
    private val text = java.io.ByteArrayOutputStream()
    val removed = ArrayList<Int>()
    /** Resource ids (re)defined on this frame (`dirty_resources`). */
    val dirtyResources = ArrayList<Int>()

    init {
        live[1] = true
    }

    fun id(index: Int): Int = (1 shl 24) or index

    fun node(index: Int, parent: Int, x: Float, y: Float, w: Float, h: Float): TestScene {
        live[index] = true
        parents[index] = parent
        rects[index] = floatArrayOf(x, y, w, h)
        children.getOrPut(parent) { ArrayList() }.add(index)
        return this
    }

    fun table(key: Key64, words: Int): Table = tables.firstOrNull { it.key == key } ?: Table(key, tables.size, words).also { tables.add(it) }

    /** Sets field [f] of node [index]'s row in [t] to raw [bits] (`STR` takes a string). */
    fun set(t: Table, index: Int, f: FieldSpec, bits: Int): TestScene {
        val row = t.rows.getOrPut(index) { IntArray(t.words) }
        val mask = if (f.bits >= 32) -1 else (1 shl f.bits) - 1
        row[f.word] = (row[f.word] and (mask shl f.shift).inv()) or ((bits and mask) shl f.shift)
        return this
    }

    fun set(t: Table, index: Int, f: FieldSpec, value: Float): TestScene = set(t, index, f, value.toRawBits())

    fun set(t: Table, index: Int, f: FieldSpec, value: String): TestScene {
        require(f.kind == FieldKind.STR)
        val bytes = value.toByteArray(StandardCharsets.UTF_8)
        val row = t.rows.getOrPut(index) { IntArray(t.words) }
        row[f.word] = text.size()
        row[f.word + 1] = bytes.size
        text.write(bytes)
        return this
    }

    fun build(dirty: List<Int> = (2 until capacity).filter { live[it] }): Pair<LongArray, Array<ByteBuffer?>> {
        val header = LongArray(RuntimeSceneReader.HEADER_LEN)
        val slabs = arrayOfNulls<ByteBuffer>(RuntimeSceneReader.SLABS_LEN)
        val childSlab = ArrayList<Int>()
        val rows = buf(capacity * RuntimeSceneReader.NODE_ROW_SIZE)
        val rectBuf = buf(capacity * RuntimeSceneReader.LAYOUT_RECT_SIZE)
        for (i in 1 until capacity) {
            if (!live[i]) continue
            val at = i * RuntimeSceneReader.NODE_ROW_SIZE
            for (t in tables) {
                if (t.rows.containsKey(i)) {
                    val w = at + (t.slot ushr 5) * 4
                    rows.putInt(w, rows.getInt(w) or (1 shl (t.slot and 31)))
                }
            }
            rows.putInt(at + 16, if (i == 1) 0 else id(parents[i]))
            val kids = children[i].orEmpty()
            rows.putInt(at + 20, childSlab.size).putInt(at + 24, kids.size)
            kids.forEach { childSlab.add(id(it)) }
            rows.put(at + 28, 1).put(at + 29, 0xFF.toByte()).put(at + 31, 1)
            rects[i]?.forEachIndexed { k, v -> rectBuf.putFloat(i * 16 + k * 4, v) }
        }
        val textBytes = text.toByteArray()
        header[RuntimeSceneReader.H_SCHEMA] = RuntimeSceneReader.SCHEMA.value()
        header[RuntimeSceneReader.H_FRAME_ID] = frameId
        header[RuntimeSceneReader.H_NODE_CAPACITY] = capacity.toLong()
        header[RuntimeSceneReader.H_DIRTY_COUNT] = dirty.size.toLong()
        header[RuntimeSceneReader.H_REMOVED_COUNT] = removed.size.toLong()
        header[RuntimeSceneReader.H_TEXT_LEN] = textBytes.size.toLong()
        header[RuntimeSceneReader.H_CHILDREN_LEN] = childSlab.size.toLong()
        header[RuntimeSceneReader.H_TABLE_COUNT] = tables.size.toLong()
        header[RuntimeSceneReader.H_SCENE_EPOCH] = epoch.toLong()
        header[RuntimeSceneReader.H_DIRTY_RESOURCE_COUNT] = dirtyResources.size.toLong()
        slabs[RuntimeSceneReader.S_ROWS] = rows
        slabs[RuntimeSceneReader.S_RECTS] = rectBuf
        slabs[RuntimeSceneReader.S_TEXT] = buf(textBytes.size).also { it.put(0, textBytes) }
        slabs[RuntimeSceneReader.S_CHILDREN] = ints(childSlab)
        slabs[RuntimeSceneReader.S_DIRTY] = ints(dirty)
        slabs[RuntimeSceneReader.S_REMOVED] = ints(removed)
        slabs[RuntimeSceneReader.S_DIRTY_RESOURCES] = ints(dirtyResources)
        tables.forEachIndexed { n, t ->
            val h = RuntimeSceneReader.H_TABLES + n * RuntimeSceneReader.H_TABLE_STRIDE
            header[h] = t.key.value()
            header[h + 1] = t.slot.toLong()
            header[h + 2] = t.words.toLong()
            header[h + 3] = t.rows.size.toLong()
            val s = RuntimeSceneReader.S_TABLES + n * RuntimeSceneReader.S_TABLE_STRIDE
            slabs[s] = ints(t.rows.values.flatMap { it.toList() })
            slabs[s + 1] = ints(t.rows.keys.map { id(it) })
        }
        return header to slabs
    }

    fun reader(): RuntimeSceneReader {
        val (h, s) = build()
        return RuntimeSceneReader().attach(h, s)
    }

    private fun buf(n: Int): ByteBuffer = ByteBuffer.allocateDirect(n).order(ByteOrder.LITTLE_ENDIAN)

    private fun ints(v: List<Int>): ByteBuffer = buf(v.size * 4).also { b -> v.forEachIndexed { i, x -> b.putInt(i * 4, x) } }
}

/** `ImageSource` wire bytes (tag + u32 length + payload). */
fun imageSourceBytes(tag: Int, payload: ByteArray): ByteBuffer =
    ByteBuffer.allocateDirect(5 + payload.size).order(ByteOrder.LITTLE_ENDIAN).apply {
        put(tag.toByte()).putInt(payload.size).put(payload)
        flip()
    }
