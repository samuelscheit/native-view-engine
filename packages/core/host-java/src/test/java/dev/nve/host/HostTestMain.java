package dev.nve.host;

/** Runs every host-java suite; exits non-zero on any failure. */
public final class HostTestMain {
    private HostTestMain() {}

    public static void main(String[] args) {
        final Check c = new Check();
        Xxh64Test.run(c);
        BinCodecTest.run(c);
        TableSceneTest.run(c);
        RuntimeSceneTest.run(c);
        EventRingTest.run(c);
        GeneratedTest.run(c);
        System.out.println("host-java: " + c.passed + " checks passed, " + c.failed + " failed");
        if (c.failed != 0) System.exit(1);
    }
}
