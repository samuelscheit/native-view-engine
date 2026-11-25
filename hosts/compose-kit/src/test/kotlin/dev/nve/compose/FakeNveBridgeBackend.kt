package dev.nve.compose

import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.StandardCharsets

/**
 * Helper for building exact 64-byte little-endian `NveRenderNode` direct `ByteBuffer` slabs
 * matching `packages/core/abi/src/scene.rs` and `cxx/nve-fabric-bridge/include/nve_scene_abi.generated.h`.
 */
class SceneSlabBuilder {
    data class NodeSpec(
        val nodeId: Int,
        val kind: Int = NveSceneBuffer.KIND_VIEW,
        val flags: Int = 0,
        val opacityU8: Int = 255,
        val x: Float = 0f,
        val y: Float = 0f,
        val width: Float = 0f,
        val height: Float = 0f,
        val bgRgba: Int = 0,
        val textRgba: Int = 0xFFFFFFFF.toInt(),
        val borderRgba: Int = 0,
        val borderRadius: Float = 0f,
        val borderWidth: Float = 0f,
        val fontSize: Float = 14f,
        val text: String = "",
        val childIds: IntArray = IntArray(0),
        val zIndex: Int = 0,
    )

    data class BuiltFrame(
        val meta: LongArray,
        val slabs: NveSceneSlabs,
    )

    companion object {
        fun buildFrame(
            frameId: Long,
            nodes: List<NodeSpec>,
            dirtyIndices: IntArray = IntArray(nodes.size) { it },
            removedIds: IntArray = IntArray(0),
            schemaHash: Long = NveSceneBuffer.SCHEMA_HASH,
        ): BuiltFrame {
            val textChunks = ArrayList<ByteArray>(nodes.size)
            val textOffsets = IntArray(nodes.size)
            val textLengths = IntArray(nodes.size)
            var totalTextBytes = 0
            for (i in nodes.indices) {
                val raw = nodes[i].text.toByteArray(StandardCharsets.UTF_8)
                textChunks.add(raw)
                textOffsets[i] = totalTextBytes
                textLengths[i] = raw.size
                totalTextBytes += raw.size
            }

            val childOffsets = IntArray(nodes.size)
            var totalChildren = 0
            for (i in nodes.indices) {
                childOffsets[i] = totalChildren
                totalChildren += nodes[i].childIds.size
            }

            val nodesBuf = ByteBuffer.allocateDirect(nodes.size * NveSceneBuffer.NODE_STRIDE)
                .order(ByteOrder.LITTLE_ENDIAN)
            for (i in nodes.indices) {
                val n = nodes[i]
                val base = i * NveSceneBuffer.NODE_STRIDE
                nodesBuf.putInt(base + NveSceneBuffer.OFFSET_NODE_ID, n.nodeId)
                nodesBuf.put(base + NveSceneBuffer.OFFSET_KIND, n.kind.toByte())
                nodesBuf.put(base + NveSceneBuffer.OFFSET_FLAGS, n.flags.toByte())
                nodesBuf.put(base + NveSceneBuffer.OFFSET_OPACITY_U8, n.opacityU8.toByte())
                nodesBuf.put(base + NveSceneBuffer.OFFSET_PAD0, 0)
                nodesBuf.putFloat(base + NveSceneBuffer.OFFSET_X, n.x)
                nodesBuf.putFloat(base + NveSceneBuffer.OFFSET_Y, n.y)
                nodesBuf.putFloat(base + NveSceneBuffer.OFFSET_WIDTH, n.width)
                nodesBuf.putFloat(base + NveSceneBuffer.OFFSET_HEIGHT, n.height)
                nodesBuf.putInt(base + NveSceneBuffer.OFFSET_BG_RGBA, n.bgRgba)
                nodesBuf.putInt(base + NveSceneBuffer.OFFSET_TEXT_RGBA, n.textRgba)
                nodesBuf.putInt(base + NveSceneBuffer.OFFSET_BORDER_RGBA, n.borderRgba)
                nodesBuf.putFloat(base + NveSceneBuffer.OFFSET_BORDER_RADIUS, n.borderRadius)
                nodesBuf.putFloat(base + NveSceneBuffer.OFFSET_BORDER_WIDTH, n.borderWidth)
                nodesBuf.putFloat(base + NveSceneBuffer.OFFSET_FONT_SIZE, n.fontSize)
                nodesBuf.putInt(base + NveSceneBuffer.OFFSET_TEXT_OFFSET, textOffsets[i])
                nodesBuf.putShort(base + NveSceneBuffer.OFFSET_TEXT_LEN, textLengths[i].toShort())
                nodesBuf.putShort(base + NveSceneBuffer.OFFSET_CHILD_COUNT, n.childIds.size.toShort())
                nodesBuf.putInt(base + NveSceneBuffer.OFFSET_CHILD_OFFSET, childOffsets[i])
                nodesBuf.putInt(base + NveSceneBuffer.OFFSET_Z_INDEX, n.zIndex)
            }

            val textBuf = ByteBuffer.allocateDirect(totalTextBytes).order(ByteOrder.LITTLE_ENDIAN)
            for (chunk in textChunks) {
                textBuf.put(chunk)
            }
            textBuf.flip()

            val childrenBuf = ByteBuffer.allocateDirect(totalChildren * 4).order(ByteOrder.LITTLE_ENDIAN)
            for (n in nodes) {
                for (cid in n.childIds) {
                    childrenBuf.putInt(cid)
                }
            }
            childrenBuf.flip()

            val dirtyBuf = ByteBuffer.allocateDirect(dirtyIndices.size * 4).order(ByteOrder.LITTLE_ENDIAN)
            for (idx in dirtyIndices) {
                dirtyBuf.putInt(idx)
            }
            dirtyBuf.flip()

            val removedBuf = ByteBuffer.allocateDirect(removedIds.size * 4).order(ByteOrder.LITTLE_ENDIAN)
            for (id in removedIds) {
                removedBuf.putInt(id)
            }
            removedBuf.flip()

            val meta = longArrayOf(
                schemaHash,
                frameId,
                nodes.size.toLong(),
                dirtyIndices.size.toLong(),
                removedIds.size.toLong(),
                totalTextBytes.toLong(),
                totalChildren.toLong(),
            )
            return BuiltFrame(
                meta = meta,
                slabs = NveSceneSlabs(
                    nodes = nodesBuf,
                    textSlab = textBuf,
                    childrenSlab = childrenBuf,
                    dirtyIndices = dirtyBuf,
                    removedIds = removedBuf,
                ),
            )
        }
    }
}

/**
 * Deterministic in-memory [NveBridgeBackend] recording all ABI crossings for unit & Robolectric tests.
 */
class FakeNveBridgeBackend : NveBridgeBackend {
    data class CreateCall(
        val handle: Long,
        val surfaceId: Int,
        val componentName: String,
        val vtableSymbol: String,
        val backendCode: Int,
    )

    data class ResizeCall(
        val handle: Long,
        val width: Float,
        val height: Float,
        val scaleFactor: Float,
    )

    data class CommandCall(
        val handle: Long,
        val name: String,
        val argsBin: ByteArray,
    )

    data class PointerCall(
        val handle: Long,
        val pointerId: Int,
        val phase: Int,
        val x: Float,
        val y: Float,
    )

    override var isAvailable: Boolean = true
    var nextHandle: Long = 100L
    var reloadShouldSucceed: Boolean = true
    var reloadPredicate: (String) -> Boolean = { reloadShouldSucceed }
    var measuredIntrinsic: FloatArray? = floatArrayOf(240f, 120f)
    var pointerDisposition: NveGestureDisposition = NveGestureDisposition.CLAIM
    var nextFrameProvider: (Long) -> SceneSlabBuilder.BuiltFrame? = { frameId ->
        SceneSlabBuilder.buildFrame(
            frameId = frameId,
            nodes = listOf(
                SceneSlabBuilder.NodeSpec(
                    nodeId = 1,
                    kind = NveSceneBuffer.KIND_ROOT,
                    width = 360f,
                    height = 640f,
                ),
            ),
        )
    }

    val createCalls = mutableListOf<CreateCall>()
    val destroyCalls = mutableListOf<Long>()
    val resizeCalls = mutableListOf<ResizeCall>()
    val propsBinCalls = mutableListOf<ByteArray>()
    val commandCalls = mutableListOf<CommandCall>()
    val submittedEventRings = mutableListOf<ByteArray>()
    val renderedFrames = mutableListOf<Long>()
    val reloadCalls = mutableListOf<String>()
    val pointerCalls = mutableListOf<PointerCall>()
    val outboundEventQueue = ArrayDeque<ByteArray>()

    private var lastBuiltFrame: SceneSlabBuilder.BuiltFrame? = null
    private val liveHandles = HashSet<Long>()

    fun activeHandleCount(): Int = liveHandles.size

    fun enqueueOutboundEvents(vararg events: NveEvent) {
        outboundEventQueue.addLast(NveEventRingCodec.encode(events.toList()))
    }

    override fun createSurface(
        surfaceId: Int,
        componentName: String,
        vtableSymbol: String,
        backendCode: Int,
    ): Long {
        if (!isAvailable) return 0L
        val h = nextHandle++
        liveHandles.add(h)
        createCalls.add(CreateCall(h, surfaceId, componentName, vtableSymbol, backendCode))
        return h
    }

    override fun destroySurface(handlePtr: Long) {
        liveHandles.remove(handlePtr)
        destroyCalls.add(handlePtr)
    }

    override fun resizeSurface(
        handlePtr: Long,
        width: Float,
        height: Float,
        scaleFactor: Float,
    ): Boolean {
        if (handlePtr !in liveHandles) return false
        resizeCalls.add(ResizeCall(handlePtr, width, height, scaleFactor))
        return true
    }

    override fun setPropsBinary(handlePtr: Long, propsBin: ByteArray): Boolean {
        if (handlePtr !in liveHandles) return false
        propsBinCalls.add(propsBin.copyOf())
        return true
    }

    override fun dispatchCommandBinary(handlePtr: Long, commandName: String, argsBin: ByteArray): Boolean {
        if (handlePtr !in liveHandles) return false
        commandCalls.add(CommandCall(handlePtr, commandName, argsBin.copyOf()))
        return true
    }

    override fun submitEvents(handlePtr: Long, eventRingBytes: ByteArray): Boolean {
        if (handlePtr !in liveHandles) return false
        submittedEventRings.add(eventRingBytes.copyOf())
        return true
    }

    override fun renderFrame(handlePtr: Long, frameId: Long, outMeta: LongArray): Int {
        if (handlePtr !in liveHandles) return -1
        renderedFrames.add(frameId)
        val built = nextFrameProvider(frameId) ?: return -1
        lastBuiltFrame = built
        built.meta.copyInto(outMeta, 0, 0, minOf(built.meta.size, outMeta.size))
        return built.meta[2].toInt()
    }

    override fun getSceneSlabs(handlePtr: Long): NveSceneSlabs? {
        if (handlePtr !in liveHandles) return null
        return lastBuiltFrame?.slabs
    }

    override fun drainEmittedEvents(handlePtr: Long): ByteArray? {
        if (handlePtr !in liveHandles) return null
        return outboundEventQueue.removeFirstOrNull()
    }

    override fun hotReloadApp(handlePtr: Long, soPath: String): Boolean {
        if (handlePtr !in liveHandles) return false
        reloadCalls.add(soPath)
        return reloadPredicate(soPath)
    }

    override fun measureSurface(
        handlePtr: Long,
        minWidth: Float,
        minHeight: Float,
        maxWidth: Float,
        maxHeight: Float,
    ): FloatArray? {
        if (handlePtr !in liveHandles) return null
        return measuredIntrinsic?.copyOf()
    }

    override fun hitTestPointer(
        handlePtr: Long,
        pointerId: Int,
        phase: Int,
        x: Float,
        y: Float,
    ): Int {
        pointerCalls.add(PointerCall(handlePtr, pointerId, phase, x, y))
        return pointerDisposition.code
    }
}
