import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

void main() {
  test('Generated NveHash golden XXH64 vectors match Rust', () {
    expect(nveHashVerifyTestVectors(), isTrue);
  });

  test('NveWireFrame v2 roundtrip and XXH64 corruption rejection', () {
    final payload = Uint8List.fromList(utf8.encode('flutter-command-delta'));
    final encoded = NveWireCodec.encodeFrame(
      channel: NveWireChannel.uiScene,
      kind: NveWireKind.commandDelta,
      sequence: 23,
      payload: payload,
    );
    expect(encoded.length, NveWireCodec.headerSize + payload.length + NveWireCodec.trailerSize);

    final decoded = NveWireCodec.decodeFrame(encoded);
    expect(decoded.channel, NveWireChannel.uiScene);
    expect(decoded.kind, NveWireKind.commandDelta);
    expect(decoded.sequence, 23);
    expect(decoded.payload, payload);

    final corrupted = Uint8List.fromList(encoded);
    corrupted[NveWireCodec.headerSize] ^= 0x01;
    expect(
      () => NveWireCodec.decodeFrame(corrupted),
      throwsA(isA<NvePairProtocolException>()),
    );
  });

  test('NveParsedPairUri parses quic/wss/cert_hash URIs', () {
    final parsed = NveParsedPairUri.parse(
      'nve://pair?host=127.0.0.1&quic=8091&wss=8092&token=nve_tok&cert_hash=deadbeef',
    );
    expect(parsed.webSocketUri.toString(), 'wss://127.0.0.1:8092/nve-wire');
    expect(parsed.token, 'nve_tok');
    expect(parsed.certHashHex, 'deadbeef');
  });

  test('NvePairClient handshake, CommandDelta, and EventBatch', () {
    final client = NvePairClient();
    expect(client.isPaired, isFalse);

    final ackPayload = Uint8List(12);
    ByteData.sublistView(ackPayload)
      ..setUint32(0, 88, Endian.little)
      ..setUint16(4, 2, Endian.little);
    client.handleInboundBinary(NveWireCodec.encodeFrame(
      channel: NveWireChannel.control,
      kind: NveWireKind.helloAck,
      sequence: 1,
      payload: ackPayload,
    ));
    expect(client.isPaired, isTrue);
    expect(client.telemetry.sessionId, 88);

    final cmd = Uint8List.fromList(<int>[0x4E, 0x56, 0x45, 0x31, 1, 2, 3, 4]);
    client.handleInboundBinary(NveWireCodec.encodeFrame(
      channel: NveWireChannel.uiScene,
      kind: NveWireKind.commandDelta,
      sequence: 4,
      payload: cmd,
    ));
    expect(client.latestFrame, cmd);
    expect(client.latestFrameSeq, 4);
    expect(client.telemetry.framesReceived, 1);

    final evFrame = client.sendInputEvents(<NveEvent>[
      const NveEvent(
        surfaceId: 1,
        nodeId: 2,
        timestampUs: 1000,
        kind: NveEventKind.press,
        payload: NvePointPayload(x: 10, y: 20),
      ),
    ]);
    final decodedEv = NveWireCodec.decodeFrame(evFrame);
    expect(decodedEv.channel, NveWireChannel.inputEvents);
    expect(decodedEv.kind, NveWireKind.eventBatch);
    expect(client.telemetry.eventsSent, 1);
  });
}
