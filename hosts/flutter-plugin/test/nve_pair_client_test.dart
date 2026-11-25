import 'dart:async';
import 'dart:convert' show utf8;
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

class _MockPairSocket implements NvePairSocket {
  final StreamController<Uint8List> _inbound = StreamController<Uint8List>.broadcast();
  final Completer<void> _done = Completer<void>();
  final List<Uint8List> sent = <Uint8List>[];
  bool closed = false;

  @override
  Stream<Uint8List> get binaryMessages => _inbound.stream;

  @override
  Future<void> get done => _done.future;

  @override
  void send(Uint8List bytes) {
    sent.add(Uint8List.fromList(bytes));
  }

  void pushServerBinary(Uint8List bytes) {
    if (!_inbound.isClosed) {
      _inbound.add(bytes);
    }
  }

  @override
  Future<void> close() async {
    closed = true;
    if (!_inbound.isClosed) {
      await _inbound.close();
    }
    if (!_done.isCompleted) {
      _done.complete();
    }
  }
}

void _writeVarint(int value, BytesBuilder out) {
  var v = value;
  while (true) {
    var b = v & 0x7F;
    v = v >>> 7;
    if (v != 0) {
      out.addByte(b | 0x80);
    } else {
      out.addByte(b);
      break;
    }
  }
}

Uint8List _encodeXorRleDelta(Uint8List base, Uint8List next) {
  final out = BytesBuilder(copy: false);
  _writeVarint(next.length, out);
  final len = next.length;
  var i = 0;
  while (i < len) {
    final z0 = i;
    while (i < len && ((i < base.length ? base[i] : 0) ^ next[i]) == 0) { i++; }
    final zr = i - z0;
    if (zr > 0) { if (zr <= 127) { out.addByte(zr); } else { out.addByte(0); _writeVarint(zr, out); } }
    if (i >= len) break;
    final l0 = i;
    while (i < len) {
      if (((i < base.length ? base[i] : 0) ^ next[i]) == 0) {
        final nz = (i + 1 < len) && (((i + 1 < base.length ? base[i + 1] : 0) ^ next[i + 1]) == 0);
        if (nz || i + 1 == len) break;
      }
      i++;
    }
    final ll = i - l0;
    if (ll <= 127) { out.addByte(0x80 | ll); } else { out.addByte(0x80); _writeVarint(ll, out); }
    for (var k = l0; k < i; k++) { out.addByte((k < base.length ? base[k] : 0) ^ next[k]); }
  }
  return out.toBytes();
}

/// Builds a valid `NVE1` `CommandBuffer` with `CreateNode`, `SetText`, `SetChildren`, `Commit`.
Uint8List _makeNveCommandBuffer({int surfaceId = 1, required int frameId, required String text}) {
  final ops = <Uint8List>[];

  // CreateNode(id=2, kind=2 (Text), name="")
  final op1 = Uint8List(9);
  op1[0] = 1;
  ByteData.sublistView(op1)
    ..setUint32(1, 2, Endian.little)
    ..setUint16(5, 2, Endian.little)
    ..setUint16(7, 0, Endian.little);
  ops.add(op1);

  // SetText(id=2, text)
  final textBytes = utf8.encode(text);
  final op2 = Uint8List(9 + textBytes.length);
  op2[0] = 4;
  ByteData.sublistView(op2)
    ..setUint32(1, 2, Endian.little)
    ..setUint32(5, textBytes.length, Endian.little);
  op2.setRange(9, op2.length, textBytes);
  ops.add(op2);

  // SetChildren(parent=1, [2])
  final op3 = Uint8List(13);
  op3[0] = 3;
  ByteData.sublistView(op3)
    ..setUint32(1, 1, Endian.little)
    ..setUint32(5, 1, Endian.little)
    ..setUint32(9, 2, Endian.little);
  ops.add(op3);

  // Commit(sync=1)
  ops.add(Uint8List.fromList(<int>[10, 1]));

  final builder = BytesBuilder(copy: false);
  final hdr = Uint8List(24);
  ByteData.sublistView(hdr)
    ..setUint32(0, nveAbiMagic, Endian.little)
    ..setUint16(4, nveAbiVersion, Endian.little)
    ..setUint16(6, 0, Endian.little)
    ..setUint32(8, surfaceId, Endian.little)
    ..setUint64(12, frameId, Endian.little)
    ..setUint32(20, ops.length, Endian.little);
  builder.add(hdr);
  for (final op in ops) {
    builder.add(op);
  }
  return builder.toBytes();
}

Uint8List _makeCommandOpDeltaPayload({
  required Uint8List base,
  required Uint8List next,
  required int frameId,
  required bool useXorOp,
}) {
  final baseRanges = NveDeltaCodec.scanNveCommandSlices(base)!;
  final nextRanges = NveDeltaCodec.scanNveCommandSlices(next)!;
  final out = BytesBuilder(copy: false);
  out.add(<int>[0, 0]); // flags
  _writeVarint(1, out); // surfaceId
  _writeVarint(frameId, out);
  _writeVarint(4, out); // opCount

  // CopyRun(start=0, count=1)
  out.addByte(1);
  _writeVarint(0, out);
  _writeVarint(1, out);

  final (bs, be) = baseRanges[1];
  final (ns, ne) = nextRanges[1];
  final baseOp1 = Uint8List.sublistView(base, bs, be);
  final nextOp1 = Uint8List.sublistView(next, ns, ne);
  if (useXorOp) {
    final xorPayload = _encodeXorRleDelta(baseOp1, nextOp1);
    out.addByte(3);
    _writeVarint(1, out);
    _writeVarint(xorPayload.length, out);
    out.add(xorPayload);
  } else {
    out.addByte(2);
    _writeVarint(nextOp1.length, out);
    out.add(nextOp1);
  }

  // CopyRun(start=2, count=2)
  out.addByte(1);
  _writeVarint(2, out);
  _writeVarint(2, out);
  return out.toBytes();
}

void main() {
  group('NvePairUri parsing', () {
    test('parses nve://pair full, embedded ws/wss, IPv6, and direct ws URIs', () {
      final full = NvePairUri.parse(
        'nve://pair?host=127.0.0.1&udp=8091&ws=8092&token=tok123&w=393&h=852',
      );
      expect(full.wsUri.toString(), 'ws://127.0.0.1:8092/ws/nvd');
      expect(full.token, 'tok123');
      expect(full.wsPort, 8092);
      expect(full.udpPort, 8091);
      expect(full.width, 393);
      expect(full.height, 852);
      expect(full.schemaVersion, 1);

      final embedded = NvePairUri.parse(
        'nve://pair?ws=wss://studio.local:9443/ws/nvd&token=abc&schema=2',
      );
      expect(embedded.wsUri.toString(), 'wss://studio.local:9443/ws/nvd');
      expect(embedded.token, 'abc');
      expect(embedded.schemaVersion, 2);

      final ipv6 = NvePairUri.parse('nve://pair?host=::1&ws=8092&token=v6');
      expect(ipv6.wsUri.toString(), 'ws://[::1]:8092/ws/nvd');

      final direct = NvePairUri.parse('ws://10.0.0.2:8092/ws/nvd', fallbackToken: 'override');
      expect(direct.wsUri.toString(), 'ws://10.0.0.2:8092/ws/nvd');
      expect(direct.token, 'override');
    });

    test('rejects invalid pairing URIs', () {
      expect(() => NvePairUri.parse(''), throwsA(isA<NvePairProtocolException>()));
      expect(() => NvePairUri.parse('nve://bad?ws=8092&token=t'), throwsA(isA<NvePairProtocolException>()));
      expect(() => NvePairUri.parse('nve://pair?host=127.0.0.1&token=t'), throwsA(isA<NvePairProtocolException>()));
      expect(() => NvePairUri.parse('nve://pair?host=127.0.0.1&ws=8092'), throwsA(isA<NvePairProtocolException>()));
      expect(() => NvePairUri.parse('nve://pair?ws=70000&token=t'), throwsA(isA<NvePairProtocolException>()));
    });
  });

  group('NVT1 framing & NVD1 delta codecs', () {
    test('FNV-1a 64-bit, token proof, session ID, and NVT1 envelopes match Rust wire spec', () {
      // Standard FNV-1a 64-bit test vectors: "" -> 0xcbf29ce484222325, "foobar" -> 0x85944171f73967e8
      expect(NvePairCrypto.fnv1a64(const <int>[]).toHex(), '0xcbf29ce484222325');
      expect(NvePairCrypto.fnv1a64(utf8.encode('foobar')).toHex(), '0x85944171f73967e8');

      final proof = NvePairCrypto.deriveTokenProof('secret-pair', schemaVersion: 1);
      expect(proof.length, 16);

      final hello = NveHelloPayload(
        schemaVersion: 1,
        maxWindow: 64,
        clientLastSeq: 5,
        clientNonce: const NveU64(0x05060708, 0x01020304),
        tokenProof: proof,
      );
      final encodedHello = NveTransportEnvelope.encode(
        kind: NveTransportMsgKind.hello,
        sessionId: 0,
        payload: hello.encode(),
      );
      expect(encodedHello.length, 40);
      expect(encodedHello.sublist(0, 4), <int>[0x4E, 0x54, 1, 1]);

      final parsedEnv = NveTransportEnvelope.parse(encodedHello);
      expect(parsedEnv.kind, NveTransportMsgKind.hello);
      final decodedHello = NveHelloPayload.decode(parsedEnv.payload);
      expect(decodedHello.clientLastSeq, 5);
      expect(decodedHello.clientNonce, hello.clientNonce);
      expect(decodedHello.tokenProof, proof);

      final sid = NvePairCrypto.deriveSessionId(
        pairingToken: 'secret-pair',
        clientNonce: hello.clientNonce,
        serverNonce: const NveU64(0x2222, 0x3333),
      );
      expect(sid, isNot(0));
    });

    test('NVD1 Keyframe, CommandOpDelta (LiteralOp & XorOp), and XorRleDelta reconstruct verbatim', () {
      final receiver = NveDeltaReceiver();
      final f1 = _makeNveCommandBuffer(frameId: 1, text: 'Todo');
      final f2 = _makeNveCommandBuffer(frameId: 2, text: 'In Progress');
      final f3 = _makeNveCommandBuffer(frameId: 3, text: 'In Progres!');

      final kf = NveDeltaPacket.encode(kind: NveDeltaPacketKind.keyframe, seq: 1, payload: f1);
      final out1 = receiver.receivePacket(kf);
      expect(out1.appliedFrames.length, 1);
      expect(out1.appliedFrames.first.frame, f1);
      expect(out1.gapDetected, isFalse);

      final delta2 = _makeCommandOpDeltaPayload(base: f1, next: f2, frameId: 2, useXorOp: false);
      final p2 = NveDeltaPacket.encode(
        kind: NveDeltaPacketKind.commandOpDelta,
        seq: 2,
        baseSeq: 1,
        payload: delta2,
      );
      final out2 = receiver.receivePacket(p2);
      expect(out2.appliedFrames.first.frame, f2);

      final delta3 = _makeCommandOpDeltaPayload(base: f2, next: f3, frameId: 3, useXorOp: true);
      final p3 = NveDeltaPacket.encode(
        kind: NveDeltaPacketKind.commandOpDelta,
        seq: 3,
        baseSeq: 2,
        payload: delta3,
      );
      final out3 = receiver.receivePacket(p3);
      expect(out3.appliedFrames.first.frame, f3);

      // Extended (>127 byte) zero run + length growth XOR+RLE round-trip
      final largeBase = Uint8List.fromList(List<int>.filled(400, 0x55));
      final largeNext = Uint8List.fromList(List<int>.filled(520, 0x55));
      largeNext[210] = 0xAA;
      largeNext[519] = 0x12;
      final xorBytes = _encodeXorRleDelta(largeBase, largeNext);
      expect(xorBytes.length, lessThan(160));
      expect(NveDeltaCodec.decodeXorRleDelta(largeBase, xorBytes), largeNext);
    });

    test('rejects truncated, bad-magic, bad-checksum, and oversized packets', () {
      expect(
        () => NveTransportEnvelope.parse(Uint8List.fromList(<int>[0x4E, 0x54, 1])),
        throwsA(isA<NvePairProtocolException>()),
      );
      expect(
        () => NveTransportEnvelope.parse(Uint8List.fromList(<int>[0, 0, 1, 1, 0, 0, 0, 0])),
        throwsA(isA<NvePairProtocolException>()),
      );
      expect(
        () => NveTransportEnvelope.parse(Uint8List.fromList(<int>[0x4E, 0x54, 1, 99, 0, 0, 0, 0])),
        throwsA(isA<NvePairProtocolException>()),
      );

      final badChecksum = NveDeltaPacket.encode(
        kind: NveDeltaPacketKind.keyframe,
        seq: 1,
        payload: Uint8List.fromList(<int>[1, 2, 3, 4]),
      );
      badChecksum[badChecksum.length - 1] ^= 0xFF;
      expect(
        () => NveDeltaPacket.parse(badChecksum),
        throwsA(
          predicate<Object>(
            (e) =>
                e is NvePairProtocolException &&
                e.code == NvePairProtocolErrorCode.checksumMismatch,
          ),
        ),
      );

      final truncated = badChecksum.sublist(0, badChecksum.length - 1);
      expect(
        () => NveDeltaPacket.parse(truncated),
        throwsA(
          predicate<Object>(
            (e) =>
                e is NvePairProtocolException &&
                e.code == NvePairProtocolErrorCode.lengthMismatch,
          ),
        ),
      );
    });

    test('InputEvents = 11 round-trips binary EventRing and enforces 64 KiB cap', () {
      final events = <NveEvent>[
        const NveEvent(
          surfaceId: 1,
          nodeId: 9,
          timestampUs: 50000,
          kind: NveEventKind.press,
          payload: NvePointPayload(x: 10.5, y: 20.5),
        ),
        const NveEvent(
          surfaceId: 1,
          nodeId: 9,
          timestampUs: 50010,
          kind: NveEventKind.textChange,
          payload: NveTextPayload('flutter-pair'),
        ),
      ];
      final frame = NvePairCodec.encodeInputEvents(events, 0x12345678);
      final env = NveTransportEnvelope.parse(frame);
      expect(env.kind, NveTransportMsgKind.inputEvents);
      expect(env.kind.code, 11);
      expect(env.sessionId, 0x12345678);

      final decoded = NveEventRingCodec.decode(env.payload);
      expect(decoded.length, 2);
      expect(decoded[0].kind, NveEventKind.press);
      expect(decoded[1].payload, const NveTextPayload('flutter-pair'));

      expect(
        () => NvePairCodec.encodeRawInputEvents(
          Uint8List(NvePairProtocol.maxInputEventBytes + 1),
          1,
        ),
        throwsA(isA<NvePairProtocolException>()),
      );
    });
  });

  group('NvePairClient state machine & NvePairedSurface widget', () {
    test('handshake, gap recovery KeyframeRequest, Ping/Pong RTT, InputEvents, and Bye', () async {
      var nowUs = 200000;
      late _MockPairSocket socket;
      Uri? dialedUri;
      final client = NvePairClient(
        connector: (Uri uri) async {
          dialedUri = uri;
          socket = _MockPairSocket();
          return socket;
        },
        autoReconnect: false,
        heartbeatInterval: Duration.zero,
        nowMicros: () => nowUs,
      );
      addTearDown(client.dispose);

      await client.connect('nve://pair?host=127.0.0.1&ws=8092&token=tok_flutter');
      expect(dialedUri.toString(), 'ws://127.0.0.1:8092/ws/nvd');
      expect(client.state, NvePairConnectionState.connecting);
      expect(socket.sent.length, 1);

      final helloEnv = NveTransportEnvelope.parse(socket.sent.first);
      expect(helloEnv.kind, NveTransportMsgKind.hello);
      final hello = NveHelloPayload.decode(helloEnv.payload);

      const serverNonce = NveU64(0x11223344, 0x55667788);
      final sid = NvePairCrypto.deriveSessionId(
        pairingToken: 'tok_flutter',
        clientNonce: hello.clientNonce,
        serverNonce: serverNonce,
      );
      final ackEnv = NveTransportEnvelope.encode(
        kind: NveTransportMsgKind.helloAck,
        sessionId: sid,
        payload: const NveHelloAckPayload(
          schemaVersion: 1,
          serverMaxWindow: 64,
          serverSeq: 0,
          serverNonce: serverNonce,
          resyncRequired: false,
        ).encode(),
      );
      client.handleInboundBinary(ackEnv);
      expect(client.state, NvePairConnectionState.paired);
      expect(client.sessionId, sid);

      // Keyframe seq=1
      final f1 = _makeNveCommandBuffer(frameId: 1, text: 'Frame 1');
      final nvd1 = NveDeltaPacket.encode(kind: NveDeltaPacketKind.keyframe, seq: 1, payload: f1);
      client.handleInboundBinary(
        NveTransportEnvelope.encode(
          kind: NveTransportMsgKind.deltaSingle,
          sessionId: sid,
          payload: nvd1,
        ),
      );
      expect(client.latestFrameSeq, 1);
      expect(client.latestFrame, f1);

      // Sequence gap: seq=3 arrives while seq=2 missing -> triggers KeyframeRequest
      final f3 = _makeNveCommandBuffer(frameId: 3, text: 'Frame 3');
      final nvd3 = NveDeltaPacket.encode(
        kind: NveDeltaPacketKind.xorRleDelta,
        seq: 3,
        baseSeq: 2,
        payload: _encodeXorRleDelta(f1, f3),
      );
      final gapOut = client.handleInboundBinary(
        NveTransportEnvelope.encode(
          kind: NveTransportMsgKind.deltaSingle,
          sessionId: sid,
          payload: nvd3,
        ),
      );
      final gapKinds = gapOut.map((b) => NveTransportEnvelope.parse(b).kind).toList();
      expect(gapKinds, contains(NveTransportMsgKind.keyframeRequest));
      expect(client.telemetry.keyframeRequestsSent, 1);

      // Ping / Pong RTT
      final pingOut = client.sendPing()!;
      expect(NveTransportEnvelope.parse(pingOut).kind, NveTransportMsgKind.ping);
      nowUs += 3500;
      final pongPayload = Uint8List(8);
      NveU64.fromInt(200000).writeLE(ByteData.sublistView(pongPayload), 0);
      client.handleInboundBinary(NvePairCodec.encodePong(sid, pongPayload));
      expect(client.telemetry.lastRttUs, 3500);
      expect(client.telemetry.smoothedRttUs, 3500);

      // InputEvents = 11
      client.sendInputEvents(<NveEvent>[
        const NveEvent(
          surfaceId: 1,
          nodeId: 2,
          timestampUs: 203500,
          kind: NveEventKind.press,
          payload: NvePointPayload(x: 4, y: 8),
        ),
      ]);
      expect(NveTransportEnvelope.parse(socket.sent.last).kind, NveTransportMsgKind.inputEvents);
      expect(client.telemetry.eventsSent, 1);

      await client.disconnect();
      expect(client.state, NvePairConnectionState.disconnected);
      expect(NveTransportEnvelope.parse(socket.sent.last).kind, NveTransportMsgKind.bye);
    });

    testWidgets('NvePairedSurface connects, displays telemetry & frame, and forwards tap events', (
      WidgetTester tester,
    ) async {
      late _MockPairSocket socket;
      final receivedSeqs = <int>[];
      final client = NvePairClient(
        connector: (Uri _) async {
          socket = _MockPairSocket();
          return socket;
        },
        autoReconnect: false,
        heartbeatInterval: Duration.zero,
      );
      addTearDown(client.dispose);

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: NvePairedSurface(
              client: client,
              initialUri: 'nve://pair?host=127.0.0.1&ws=8092&token=ui_tok',
              onFrame: (int seq, Uint8List _) => receivedSeqs.add(seq),
            ),
          ),
        ),
      );

      expect(find.text('Disconnected'), findsOneWidget);
      await tester.tap(find.byKey(const Key('nve_pair_connect_button')));
      await tester.pump();

      final hello = NveHelloPayload.decode(NveTransportEnvelope.parse(socket.sent.first).payload);
      const serverNonce = NveU64(0x9999, 0xAAAA);
      final sid = NvePairCrypto.deriveSessionId(
        pairingToken: 'ui_tok',
        clientNonce: hello.clientNonce,
        serverNonce: serverNonce,
      );

      socket.pushServerBinary(
        NveTransportEnvelope.encode(
          kind: NveTransportMsgKind.helloAck,
          sessionId: sid,
          payload: const NveHelloAckPayload(
            schemaVersion: 1,
            serverMaxWindow: 64,
            serverSeq: 0,
            serverNonce: serverNonce,
            resyncRequired: false,
          ).encode(),
        ),
      );
      await tester.pump();
      expect(find.text('Disconnect'), findsOneWidget);
      expect(find.byKey(const Key('nve_pair_keyframe_button')), findsOneWidget);

      final f1 = _makeNveCommandBuffer(frameId: 1, text: 'Paired Hello');
      socket.pushServerBinary(
        NveTransportEnvelope.encode(
          kind: NveTransportMsgKind.deltaSingle,
          sessionId: sid,
          payload: NveDeltaPacket.encode(kind: NveDeltaPacketKind.keyframe, seq: 1, payload: f1),
        ),
      );
      await tester.pump();

      expect(receivedSeqs, <int>[1]);
      expect(find.textContaining('Frames 1'), findsOneWidget);
      expect(find.byKey(const Key('nve_pair_surface_canvas')), findsOneWidget);

      await tester.tap(find.byKey(const Key('nve_pair_surface_canvas')));
      await tester.pump();
      expect(NveTransportEnvelope.parse(socket.sent.last).kind, NveTransportMsgKind.inputEvents);
    });
  });
}
