package dev.nve.compose

import dev.nve.host.EventRingReader
import dev.nve.host.FieldSpec
import dev.nve.host.Key64
import dev.nve.host.PackedSceneBuilder
import dev.nve.host.SceneReader
import dev.nve.host.generated.NveModuleTables
import dev.nve.host.generated.NveTables
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Packed runtime frames for JVM tests over [PackedSceneBuilder] (the JNI layout, incl. the sparse
 * row index): tables keep their real storage (dense core tables have no owners). Node index `i`
 * has `NodeId = 1 << 24 | i`; index 1 is the root.
 */
class TestScene(capacity: Int = 16, var frameId: Long = 1L, var epoch: Int = 0) {
    private val builder = PackedSceneBuilder(capacity, frameId, epoch)
    private val tables = LinkedHashMap<Key64, PackedSceneBuilder.Table>()
    val removed = ArrayList<Int>()
    /** Resource ids (re)defined on this frame (`dirty_resources`, kept ascending). */
    val dirtyResources = ArrayList<Int>()
    /** Content-dirty node indexes (`null`: every live non-root node). */
    var dirty: IntArray? = null
    /** Geometry-only node indexes (`moved_indices`). */
    var moved = IntArray(0)
    private val scrolls = ArrayList<Triple<Int, Float, Float>>()

    fun id(index: Int): Int = builder.nodeId(index)

    fun node(index: Int, parent: Int, x: Float, y: Float, w: Float, h: Float): TestScene {
        builder.node(index, parent, x, y, w, h)
        return this
    }

    /** The table with [key], with the storage its generated descriptor declares. */
    fun table(key: Key64, words: Int): PackedSceneBuilder.Table = tables.getOrPut(key) {
        val dense = NveTables.all().firstOrNull { it.key == key }?.dense
            ?: NveModuleTables.all().firstOrNull { it.key == key }?.dense
            ?: error("no generated table $key")
        builder.table(key, tables.size, words, dense)
    }

    fun set(t: PackedSceneBuilder.Table, index: Int, f: FieldSpec, bits: Int): TestScene {
        t.set(index, f, bits)
        return this
    }

    fun set(t: PackedSceneBuilder.Table, index: Int, f: FieldSpec, value: Float): TestScene {
        t.set(index, f, value)
        return this
    }

    fun set(t: PackedSceneBuilder.Table, index: Int, f: FieldSpec, value: String): TestScene {
        t.set(index, f, value)
        return this
    }

    fun setEvents(t: PackedSceneBuilder.Table, index: Int, eventsWord: Int, eventMask: Int): TestScene {
        t.setEvents(index, eventsWord, eventMask)
        return this
    }

    /** Runtime scroll offset of container [index] (call in ascending index order). */
    fun scroll(index: Int, x: Float, y: Float): TestScene {
        scrolls.add(Triple(index, x, y))
        return this
    }

    fun build(): Pair<LongArray, Array<ByteBuffer?>> {
        builder.frame(frameId, epoch)
        dirty?.let { builder.dirty(*it) }
        builder.moved(*moved)
        builder.removed(*removed.toIntArray())
        builder.dirtyResources(*dirtyResources.sorted().toIntArray())
        builder.clearScrolls()
        for ((index, x, y) in scrolls) builder.scroll(id(index), x, y)
        @Suppress("UNCHECKED_CAST")
        return builder.header() to (builder.slabs() as Array<ByteBuffer?>)
    }

    fun reader(): SceneReader {
        val (h, s) = build()
        return SceneReader().attach(h, s)
    }
}

/** Node [id] of the captured state (fails the test when it is absent). */
fun NveSceneState.at(id: Int): NveNode = this[id] ?: error("no node ${Integer.toHexString(id)}")

/** Applies [scene]'s frame to [into] (a fresh state by default) and returns it. */
fun capture(scene: TestScene, resources: NveResourceLookup, into: NveSceneState = NveSceneState()): NveSceneState =
    into.also { NveSceneCapture.capture(scene.reader(), resources, it) }

/** True when this `NVEV` v2 batch holds `(table, event)` at [index]. */
fun EventRingReader.isEvent(table: Key64, event: Int, index: Int = 0): Boolean =
    count() > index && table(index) == table && event(index) == event

/** `ImageSource` `FlatResource` wire bytes (`u32 tag | u32 length | payload zero-padded to 4 B`). */
fun imageSourceBytes(tag: Int, payload: ByteArray): ByteBuffer {
    val padded = (payload.size + 3) and 3.inv()
    return ByteBuffer.allocateDirect(8 + padded).order(ByteOrder.LITTLE_ENDIAN).apply {
        putInt(tag).putInt(payload.size).put(payload)
        repeat(padded - payload.size) { put(0.toByte()) }
        flip()
    }
}
