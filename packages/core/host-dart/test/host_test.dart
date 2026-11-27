// Dart VM entry: `dart run test/host_test.dart` (exit code 1 on any failure).
import 'dart:io';

import 'src/ffi_suite.dart';
import 'src/suites.dart';

void main() {
  final c = runAll();
  runFfi(c); // VM only: the web entry never imports `dart:ffi`
  c.skips.forEach(stdout.writeln);
  c.failures.forEach(stderr.writeln);
  final skipSuffix = c.skipped == 0 ? '' : ', ${c.skipped} skipped';
  stdout.writeln('nve_host (vm): ${c.passed} checks passed, ${c.failed} failed$skipSuffix');
  if (c.failed != 0) {
    exitCode = 1;
  }
}
