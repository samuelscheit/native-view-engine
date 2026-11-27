package dev.nve.host;

/** Minimal assertion harness (no external test framework, so it runs offline with plain javac). */
final class Check {
    int passed;
    int failed;

    void that(boolean cond, String what) {
        if (cond) {
            passed++;
        } else {
            failed++;
            System.err.println("FAIL: " + what);
        }
    }

    void eq(long actual, long expected, String what) {
        that(actual == expected, what + ": expected 0x" + Long.toHexString(expected) + ", got 0x" + Long.toHexString(actual));
    }

    void eq(String actual, String expected, String what) {
        that(expected.equals(actual), what + ": expected \"" + expected + "\", got \"" + actual + "\"");
    }

    /** Asserts {@code body} throws {@code type} (and returns it for further checks). */
    <T extends Throwable> T throwsA(Class<T> type, Runnable body, String what) {
        try {
            body.run();
        } catch (Throwable t) {
            if (type.isInstance(t)) {
                passed++;
                return type.cast(t);
            }
            failed++;
            System.err.println("FAIL: " + what + ": threw " + t);
            return null;
        }
        failed++;
        System.err.println("FAIL: " + what + ": did not throw");
        return null;
    }

    void binError(BinCodecException.Kind kind, Runnable body, String what) {
        final BinCodecException e = throwsA(BinCodecException.class, body, what);
        if (e != null) that(e.kind() == kind, what + ": kind " + e.kind() + " != " + kind);
    }
}
