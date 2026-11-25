import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nve_flutter/nve_flutter.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('NveChannelMessage', () {
    test('decodeJson parses valid JSON objects and ignores non-object/binary payloads', () {
      final valid = NveChannelMessage(
        channelId: 1,
        topic: 'theme',
        seq: 1,
        payload: Uint8List.fromList(utf8.encode('{"mode":"dark"}')),
      );
      expect(valid.decodeJson(), <String, dynamic>{'mode': 'dark'});

      final arrayPayload = NveChannelMessage(
        channelId: 1,
        topic: 'theme',
        seq: 2,
        payload: Uint8List.fromList(utf8.encode('[1,2,3]')),
      );
      expect(arrayPayload.decodeJson(), isNull);

      final binary = NveChannelMessage(
        channelId: 1,
        topic: 'bin',
        seq: 3,
        payload: Uint8List.fromList(<int>[0xFF, 0x00]),
        isJson: false,
      );
      expect(binary.decodeJson(), isNull);
    });
  });

  group('NveChannelClient pure-Dart pub/sub and retained state', () {
    test('delivers topic-isolated messages with built-in and custom channel IDs', () async {
      final client = NveChannelClient();
      addTearDown(client.dispose);

      final themeEvents = <NveChannelMessage>[];
      final customEvents = <NveChannelMessage>[];
      final sub1 = client.subscribe(NveChannelClient.topicTheme).listen(themeEvents.add);
      final sub2 = client.subscribe('custom.analytics').listen(customEvents.add);
      addTearDown(() async {
        await sub1.cancel();
        await sub2.cancel();
      });

      await client.publishJson(
        NveChannelClient.topicTheme,
        <String, dynamic>{'mode': 'dark'},
        senderSurfaceId: 1001,
      );
      await client.publish(
        'custom.analytics',
        Uint8List.fromList(<int>[1, 2, 3]),
        isJson: false,
      );
      await pumpEventQueue();

      expect(themeEvents, hasLength(1));
      expect(themeEvents.single.channelId, 1);
      expect(themeEvents.single.seq, 1);
      expect(themeEvents.single.senderSurfaceId, 1001);
      expect(themeEvents.single.isJson, isTrue);
      expect(themeEvents.single.decodeJson(), <String, dynamic>{'mode': 'dark'});

      expect(customEvents, hasLength(1));
      expect(customEvents.single.channelId, 100);
      expect(customEvents.single.seq, 2);
      expect(customEvents.single.isJson, isFalse);
      expect(customEvents.single.payload, Uint8List.fromList(<int>[1, 2, 3]));
    });

    test('replays retained message when requested and honors replayRetained=false', () async {
      final client = NveChannelClient();
      addTearDown(client.dispose);

      await client.publishJson(
        NveChannelClient.topicLocale,
        <String, dynamic>{'lang': 'de-DE'},
        retain: true,
      );
      await client.publishJson(
        NveChannelClient.topicLocale,
        <String, dynamic>{'lang': 'transient'},
        retain: false,
      );

      expect(client.retained(NveChannelClient.topicLocale)?.decodeJson(), <String, dynamic>{
        'lang': 'de-DE',
      });

      final replayed = <NveChannelMessage>[];
      final nonReplayed = <NveChannelMessage>[];
      final sub1 = client
          .subscribe(NveChannelClient.topicLocale, replayRetained: true)
          .listen(replayed.add);
      final sub2 = client
          .subscribe(NveChannelClient.topicLocale, replayRetained: false)
          .listen(nonReplayed.add);
      addTearDown(() async {
        await sub1.cancel();
        await sub2.cancel();
      });

      await pumpEventQueue();
      expect(replayed, hasLength(1));
      expect(replayed.single.decodeJson(), <String, dynamic>{'lang': 'de-DE'});
      expect(nonReplayed, isEmpty);

      await client.publishJson(
        NveChannelClient.topicLocale,
        <String, dynamic>{'lang': 'en-US'},
        retain: true,
      );
      await pumpEventQueue();

      expect(replayed, hasLength(2));
      expect(replayed.last.decodeJson(), <String, dynamic>{'lang': 'en-US'});
      expect(nonReplayed, hasLength(1));
      expect(nonReplayed.single.decodeJson(), <String, dynamic>{'lang': 'en-US'});
    });

    test('unsubscribe closes topic stream and dispose cleans up all topics', () async {
      final client = NveChannelClient();
      var doneCalled = false;
      final sub = client.subscribe('auth').listen(null, onDone: () => doneCalled = true);
      addTearDown(sub.cancel);

      await client.unsubscribe('auth');
      await pumpEventQueue();
      expect(doneCalled, isTrue);

      var secondDone = false;
      final sub2 = client.subscribe('navigation').listen(null, onDone: () => secondDone = true);
      addTearDown(sub2.cancel);
      await client.dispose();
      await pumpEventQueue();
      expect(secondDone, isTrue);
    });
  });

  group('NveChannelClient platform channel bridging', () {
    test('forwards MethodChannel calls and receives EventChannel native messages', () async {
      const methodChannel = MethodChannel('dev.nve/test_channel_bus');
      const eventChannel = EventChannel('dev.nve/test_channel_bus/events');
      final messenger = TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;

      final methodCalls = <MethodCall>[];
      messenger.setMockMethodCallHandler(methodChannel, (MethodCall call) async {
        methodCalls.add(call);
        return null;
      });
      messenger.setMockMethodCallHandler(
        MethodChannel(eventChannel.name),
        (MethodCall call) async => null,
      );
      addTearDown(() {
        messenger.setMockMethodCallHandler(methodChannel, null);
        messenger.setMockMethodCallHandler(MethodChannel(eventChannel.name), null);
      });

      final client = NveChannelClient(
        methodChannel: methodChannel,
        eventChannel: eventChannel,
      );
      addTearDown(client.dispose);

      final received = <NveChannelMessage>[];
      final sub = client.subscribe('theme').listen(received.add);
      addTearDown(sub.cancel);

      await client.publishJson('theme', <String, dynamic>{'mode': 'light'}, retain: true);

      // Simulate native EventChannel pushing a retained message from Rust NveChannelBus.
      const codec = StandardMethodCodec();
      final nativePayload = Uint8List.fromList(utf8.encode('{"mode":"system"}'));
      await messenger.handlePlatformMessage(
        eventChannel.name,
        codec.encodeSuccessEnvelope(<String, dynamic>{
          'channelId': 1,
          'topic': 'theme',
          'seq': 99,
          'payload': nativePayload,
          'senderSurfaceId': 7,
          'isJson': true,
          'retained': true,
        }),
        (_) {},
      );
      await pumpEventQueue();

      expect(methodCalls.map((c) => c.method), containsAll(<String>['subscribe', 'publish']));
      expect(received, hasLength(2));
      expect(received.last.seq, 99);
      expect(received.last.senderSurfaceId, 7);
      expect(received.last.decodeJson(), <String, dynamic>{'mode': 'system'});
      expect(client.retained('theme')?.seq, 99);

      await client.unsubscribe('theme');
      expect(methodCalls.last.method, 'unsubscribe');
      expect(methodCalls.last.arguments, <String, dynamic>{'topic': 'theme'});
    });
  });
}
