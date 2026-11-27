/// Minimal assertion harness (no `package:test`, so the suite runs offline and compiles to JS).
final class Check {
  int passed = 0;
  int failed = 0;
  int skipped = 0;
  final List<String> failures = <String>[];
  final List<String> skips = <String>[];

  void skip(String what) {
    skipped++;
    skips.add(what);
  }

  void that(bool cond, String what) {
    if (cond) {
      passed++;
    } else {
      failed++;
      failures.add(what);
    }
  }

  void eq(Object? actual, Object? expected, String what) => that(actual == expected, '$what: expected $expected, got $actual');

  /// Asserts [body] throws a [T] (returned for further checks).
  T? throwsA<T extends Object>(void Function() body, String what) {
    try {
      body();
    } on T catch (e) {
      passed++;
      return e;
    } on Object catch (e) {
      failed++;
      failures.add('$what: threw ${e.runtimeType}: $e');
      return null;
    }
    failed++;
    failures.add('$what: did not throw');
    return null;
  }
}
