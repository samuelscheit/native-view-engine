import 'bin_codec_suite.dart';
import 'bin_dynamic_suite.dart';
import 'check.dart';
import 'runtime_scene_suite.dart';
import 'table_scene_suite.dart';
import 'xxh64_suite.dart';

/// Runs every suite; identical on the Dart VM and when compiled to JavaScript.
Check runAll() {
  final c = Check();
  runXxh64(c);
  runBinCodec(c);
  runBinDynamic(c);
  runTableScene(c);
  runRuntimeScene(c);
  return c;
}
