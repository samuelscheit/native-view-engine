import 'bin_codec_suite.dart';
import 'check.dart';
import 'draw_list_suite.dart';
import 'dynamic_value_suite.dart';
import 'event_batch_suite.dart';
import 'generated_suite.dart';
import 'runtime_scene_suite.dart';
import 'table_scene_suite.dart';
import 'xxh64_suite.dart';

/// Runs every suite; identical on the Dart VM and when compiled to JavaScript.
Check runAll() {
  final c = Check();
  runXxh64(c);
  runBinCodec(c);
  runDynamicValue(c);
  runEventBatch(c);
  runTableScene(c);
  runGenerated(c);
  runRuntimeScene(c);
  runDrawList(c);
  return c;
}
