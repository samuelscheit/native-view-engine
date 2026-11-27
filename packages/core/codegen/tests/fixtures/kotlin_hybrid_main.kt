// Drives the generated Kotlin hybrid clients (`CryptoHybridSpec` + `OverlayViewHybridSpec`) on a
// real JVM, built against the static `dev.nve.host` runtime:
//   `codec`        — codec round trips, golden bytes, decode errors, defaults; prints the schema
//                    constants and every `native` method for the Rust test to check.
//   `jni <dylib>`  — loads the JNI glue + live Rust provider and runs OverlayView end to end.
package dev.nve.hybrid

import dev.nve.host.BinCodecException
import dev.nve.host.BinReader
import dev.nve.host.BinWriter
import dev.nve.host.HybridRuntime.DynamicValue
import java.lang.reflect.Modifier

private var failures = 0

private fun check(cond: Boolean, what: String) {
    if (!cond) {
        failures++
        System.err.println("FAIL: $what")
    }
}

private inline fun <reified T : Throwable> expectThrows(what: String, body: () -> Unit): T? {
    try {
        body()
    } catch (t: Throwable) {
        if (t is T) return t
        check(false, "$what: threw $t")
        return null
    }
    check(false, "$what: did not throw")
    return null
}

private fun encode(block: (BinWriter) -> Unit): ByteArray = BinWriter().also(block).toByteArray()

private fun hex(bytes: ByteArray): String = bytes.joinToString("") { "%02x".format(it) }

/** `{"a": [1, "s", true, null, 2.5, b"\x00\xff"], "b": {"nested": "😀"}}` */
private fun sampleExtra(): DynamicValue = DynamicValue.of(encode { w ->
    val m = w.beginMap(2)
    w.writeMapKey("a")
    val a = w.beginArray(6)
    w.writeI64(1)
    w.writeStr("s")
    w.writeBool(true)
    w.writeNull()
    w.writeF64(2.5)
    w.writeBytes(byteArrayOf(0, -1))
    w.end(a)
    w.writeMapKey("b")
    val b = w.beginMap(1)
    w.writeMapKey("nested")
    w.writeStr("\uD83D\uDE00")
    w.end(b)
    w.end(m)
})

private fun sampleOverlay(caption: String?): Overlay = Overlay(
    kind = OverlayKind.Ad,
    cues = listOf(1, -2, Int.MAX_VALUE, Int.MIN_VALUE),
    caption = caption,
    extra = sampleExtra(),
    labels = listOf("", "é", "\uD83D\uDE00"),
)

private fun codec() {
    // Golden bytes: FramePoint {x: 7, y: 0} = map(2) {"x": f64 7.0, "y": f64 0.0}.
    val point = encode { FramePoint(7.0f, 0.0f).encodeBin(it) }
    val golden = listOf("07", "20000000", "02000000", "01000000", "78", "03", "0000000000001c40", "01000000", "79", "03", "0000000000000000")
    check(hex(point) == golden.joinToString(""), "FramePoint golden: ${hex(point)}")
    check(FramePoint.decodeBin(BinReader(point)) == FramePoint(7.0f, 0.0f), "FramePoint round trip")

    // Non-POD struct with Vec, Option, NveValue, and enum: decode(encode(x)) == x, both options.
    for (caption in listOf(null, "", "cap \uD83D\uDE00")) {
        val overlay = sampleOverlay(caption)
        val bytes = encode { overlay.encodeBin(it) }
        BinReader.validate(bytes)
        val r = BinReader(bytes)
        check(Overlay.decodeBin(r) == overlay, "Overlay round trip ($caption)")
        r.expectEnd()
    }

    // Field names shadowing the codec's parameters / locals (`w`, `r`, `mark`, `count`), two
    // options, and a nested sequence.
    val cursor = Cursor(mark = 1, w = byteArrayOf(1, 2, 3), r = "ab", count = 4, tag = null, grid = listOf(listOf(1, 2), emptyList(), listOf(3)))
    val cursorBytes = encode { cursor.encodeBin(it) }
    val cursorBack = Cursor.decodeBin(BinReader(cursorBytes))
    check(cursorBack.copy(w = cursor.w) == cursor && cursorBack.w.contentEquals(cursor.w), "Cursor round trip")
    check(encode { cursorBack.encodeBin(it) }.contentEquals(cursorBytes), "Cursor re-encodes identically")

    val payload = SignedPayload(
        header = DigestHeader(HashAlgorithm.Sha512, 64, true, FramePoint(-1.5f, 2.25f)),
        label = "label",
        digest = byteArrayOf(-34, -83),
        tags = listOf("a", "b"),
        nonce = Long.MIN_VALUE,
    )
    val payloadBytes = encode { payload.encodeBin(it) }
    check(encode { SignedPayload.decodeBin(BinReader(payloadBytes)).encodeBin(it) }.contentEquals(payloadBytes), "SignedPayload re-encodes identically")

    // Unknown keys are skipped, missing keys keep their defaults.
    val sparse = encode { w ->
        val m = w.beginMap(2)
        w.writeMapKey("zzz")
        val a = w.beginArray(1)
        w.writeStr("ignored")
        w.end(a)
        w.writeMapKey("kind")
        w.writeI64(7)
        w.end(m)
    }
    check(Overlay.decodeBin(BinReader(sparse)) == Overlay(kind = OverlayKind.Ad), "sparse Overlay")

    // Decode errors are exceptions, never silent garbage.
    val badEnum = encode { w ->
        val m = w.beginMap(1)
        w.writeMapKey("kind")
        w.writeI64(5)
        w.end(m)
    }
    val e = expectThrows<IllegalArgumentException>("invalid enum") { Overlay.decodeBin(BinReader(badEnum)) }
    check(e?.message == "invalid OverlayKind discriminant: 5", "invalid enum message: ${e?.message}")
    val overlayBytes = encode { sampleOverlay("x").encodeBin(it) }
    expectThrows<BinCodecException>("truncated Overlay") {
        Overlay.decodeBin(BinReader(overlayBytes.copyOf(overlayBytes.size - 3)))
    }
    val wrongTag = encode { w ->
        val m = w.beginMap(1)
        w.writeMapKey("cues")
        w.writeStr("not an array")
        w.end(m)
    }
    expectThrows<BinCodecException>("wrong tag") { Overlay.decodeBin(BinReader(wrongTag)) }

    // Prop defaults from the spec.
    check(VideoSurfaceProps().playbackRate == 1.0f && VideoSurfaceProps().quality == VideoQuality.Auto, "VideoSurface defaults")
    check(!VideoSurfaceProps().muted && VideoSurfaceProps().posterUrl == null, "VideoSurface option defaults")
    check(OverlayViewProps().kind == OverlayKind.Chapter && OverlayViewProps().extra.isNull, "OverlayView defaults")

    println("schema Crypto ${CryptoHybridClient.INTERFACE_ID} ${CryptoHybridClient.SCHEMA_HASH}")
    println("schema VideoSurface ${VideoSurfaceHybridClient.INTERFACE_ID} ${VideoSurfaceHybridClient.SCHEMA_HASH}")
    println("schema OverlayView ${OverlayViewHybridClient.INTERFACE_ID} ${OverlayViewHybridClient.SCHEMA_HASH}")
    println("file Crypto ${CryptoHybridSchema.ABI_VERSION} ${CryptoHybridSchema.FILE_SCHEMA_HASH}")
    println("file OverlayView ${OverlayViewHybridSchema.ABI_VERSION} ${OverlayViewHybridSchema.FILE_SCHEMA_HASH}")
    for (cls in listOf(CryptoHybridClient::class.java, VideoSurfaceHybridClient::class.java, OverlayViewHybridClient::class.java)) {
        for (m in cls.declaredMethods.filter { Modifier.isNative(it.modifiers) }.sortedBy { it.name }) {
            check(Modifier.isStatic(m.modifiers), "${m.name} is static (JNI receives a jclass)")
            println("native ${cls.simpleName} ${m.name} ${m.parameterCount}")
        }
        // The companion only forwards: a native companion method would bind `..._00024Companion_...`.
        val companion = Class.forName(cls.name + "\$Companion")
        check(companion.declaredMethods.none { Modifier.isNative(it.modifiers) }, "${cls.simpleName}.Companion has no natives")
    }
}

private fun jni(library: String) {
    System.load(library)
    val overlay = sampleOverlay("cap")
    val overlays = listOf(sampleOverlay(null), Overlay(), sampleOverlay("second"))
    val manyCues = List(10_000) { it * 7 - 35_000 }
    val props = OverlayViewProps(
        title = "Live \u2713 \uD83D\uDE00",
        kind = OverlayKind.Ad,
        overlay = overlay,
        overlays = overlays,
        cues = manyCues,
        extra = sampleExtra(),
    )
    OverlayViewHybridClient.create().use { client ->
        client.setProps(props)
        check(client.currentOverlay() == overlay, "live currentOverlay")
        check(client.currentOverlays() == overlays, "live currentOverlays")
        check(client.currentExtra() == props.extra, "live currentExtra (lossless NveValue prop)")
        check(client.summary() == "Live \u2713 \uD83D\uDE00|Ad|true|3|10000", "live summary: ${client.summary()}")

        val echoed = client.echo(overlay)
        check(echoed == overlay.copy(kind = OverlayKind.Caption, cues = overlay.cues.reversed(), caption = "CAP", labels = overlay.labels.map { it.uppercase() }), "live echo: $echoed")
        val err = expectThrows<IllegalStateException>("echo Err") { client.echo(sampleOverlay("fail")) }
        check(err?.message == "bad \uD83D\uDE00", "echo Err message is exact UTF-16: ${err?.message}")

        // Absent options and a scalar dynamic value cross as NVE_VAL_NULL / NVE_VAL_STR.
        val scalar = DynamicValue.of(encode { it.writeStr("solo") })
        client.setProps(OverlayViewProps(title = "t", overlay = null, extra = scalar))
        check(client.currentOverlay() == null && client.currentOverlays().isEmpty(), "live cleared props")
        check(client.currentExtra() == scalar, "live scalar extra")
        check(client.summary() == "t|Chapter|false|0|0", "live summary after clear: ${client.summary()}")
        client.close()
        val closed = expectThrows<IllegalStateException>("closed client") { client.summary() }
        check(closed?.message == "null hybrid handle", "closed client message: ${closed?.message}")
    }
}

fun main(args: Array<String>) {
    when (args.getOrNull(0)) {
        "codec" -> codec()
        "jni" -> jni(args[1])
        else -> check(false, "usage: codec | jni <library>")
    }
    if (failures != 0) {
        System.err.println("kotlin hybrid: $failures failure(s)")
        kotlin.system.exitProcess(1)
    }
    println("kotlin hybrid: ok")
}
