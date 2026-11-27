package dev.nve.android.jni;

/** Minimal counting assertion harness for the JVM JNI suites (no framework, runs under ctest). */
final class Expect {
    private int passed;
    private int failed;

    void that(boolean cond, String what) {
        if (cond) {
            passed++;
        } else {
            failed++;
            System.err.println("FAIL: " + what);
        }
    }

    void eq(long actual, long expected, String what) {
        that(actual == expected, what + ": expected " + expected + ", got " + actual);
    }

    void eq(Object actual, Object expected, String what) {
        that(expected == null ? actual == null : expected.equals(actual), what + ": expected " + expected + ", got " + actual);
    }

    /** Prints the summary and exits non-zero on any failure. */
    void finish(String suite) {
        System.out.println(suite + ": " + passed + " checks passed, " + failed + " failed");
        System.exit(failed == 0 ? 0 : 1);
    }
}
