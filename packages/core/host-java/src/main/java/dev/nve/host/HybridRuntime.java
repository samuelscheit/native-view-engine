package dev.nve.host;

import java.util.Arrays;

/** Hybrid FFI prelude shared by every generated Java / Kotlin hybrid client. */
public final class HybridRuntime {
    private HybridRuntime() {}

    /** Status of a fallible hybrid FFI trampoline (Rust {@code NveHybridStatus}, {@code #[repr(i32)]}). */
    public enum HybridStatus {
        OK(0), ERR(1), INVALID_ARG(2), SCHEMA_MISMATCH(3), NULL_HANDLE(4);

        public final int code;

        HybridStatus(int code) {
            this.code = code;
        }

        /** Status for {@code code}, or {@code null} for an unknown code. */
        public static HybridStatus fromCode(int code) {
            for (HybridStatus s : values()) {
                if (s.code == code) return s;
            }
            return null;
        }
    }

    /** Status of {@code NveHostCallbacks::invoke} / {@code channel_publish} (Rust {@code NveHostStatus}). */
    public enum HostStatus {
        OK(0), PENDING(1), BUFFER_TOO_SMALL(-1), UNSUPPORTED(-2), PERMISSION_DENIED(-3), INVALID_ARG(-4),
        NOT_FOUND(-5), CANCELLED(-6), IO_ERROR(-7), INTERNAL_ERROR(-8);

        public final int code;

        HostStatus(int code) {
            this.code = code;
        }

        /** Status for {@code code}, or {@code null} for an unknown code. */
        public static HostStatus fromCode(int code) {
            for (HostStatus s : values()) {
                if (s.code == code) return s;
            }
            return null;
        }
    }

    /** A hybrid call failed: bad status, schema mismatch, or a remote error message. */
    public static final class HybridException extends RuntimeException {
        private static final long serialVersionUID = 1L;

        private final HybridStatus status;

        public HybridException(HybridStatus status, String detail) {
            super(status + ": " + detail);
            this.status = status;
        }

        public HybridStatus status() {
            return status;
        }
    }

    /** Throws {@code SCHEMA_MISMATCH} unless a peer's schema hash equals the generated one. */
    public static void checkSchema(Key64 expected, Key64 actual) {
        if (!expected.equals(actual)) {
            throw new HybridException(HybridStatus.SCHEMA_MISMATCH, "expected " + expected + ", got " + actual);
        }
    }

    /** Owned dynamic value in the compact {@code NveBinValue} encoding; always exactly one valid value. */
    public static final class DynamicValue {
        public static final DynamicValue NULL = new DynamicValue(new byte[] {0});

        private final byte[] bytes;

        private DynamicValue(byte[] bytes) {
            this.bytes = bytes;
        }

        /** Validated copy of {@code bytes}; throws {@link BinCodecException} when invalid. */
        public static DynamicValue of(byte[] bytes) {
            BinReader.validate(bytes);
            return new DynamicValue(bytes.clone());
        }

        /** Copy of the encoded bytes. */
        public byte[] bytes() {
            return bytes.clone();
        }

        public BinTag tag() {
            return BinTag.fromCode(bytes[0] & 0xFF);
        }

        public boolean isNull() {
            return tag() == BinTag.NULL;
        }

        public BinReader reader() {
            return new BinReader(bytes);
        }

        @Override
        public boolean equals(Object other) {
            return other instanceof DynamicValue && Arrays.equals(((DynamicValue) other).bytes, bytes);
        }

        @Override
        public int hashCode() {
            return Arrays.hashCode(bytes);
        }
    }
}
