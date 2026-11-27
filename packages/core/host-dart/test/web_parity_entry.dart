// Dart-web entry: `dart compile js -O2 -o out.js test/web_parity_entry.dart && node out.js`.
// Runs the exact VM suite on JS numbers; an uncaught error makes node exit non-zero.
import 'src/suites.dart';

void main() {
  final c = runAll();
  final web = identical(0, 0.0);
  print('nve_host (${web ? 'js' : 'vm'}): ${c.passed} checks passed, ${c.failed} failed');
  if (!web) {
    throw StateError('web_parity_entry must run compiled to JavaScript');
  }
  if (c.failed != 0) {
    throw StateError('failures:\n${c.failures.join('\n')}');
  }
}
