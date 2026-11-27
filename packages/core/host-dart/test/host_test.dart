// Dart VM entry: `dart run test/host_test.dart` (exit code 1 on any failure).
import 'dart:io';

import 'src/suites.dart';

void main() {
  final c = runAll();
  c.failures.forEach(stderr.writeln);
  stdout.writeln('nve_host (vm): ${c.passed} checks passed, ${c.failed} failed');
  if (c.failed != 0) {
    exitCode = 1;
  }
}
