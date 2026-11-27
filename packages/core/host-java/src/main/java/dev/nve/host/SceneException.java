package dev.nve.host;

/** A published {@code NveSceneView} frame failed validation at {@link SceneReader#attach}. */
public final class SceneException extends RuntimeException {
    private static final long serialVersionUID = 1L;

    public enum Kind {
        SCHEMA_MISMATCH, MISSING_SLAB, INVALID_UTF8, INVALID_TABLE
    }

    private final Kind kind;

    public SceneException(Kind kind, String detail) {
        super(kind + ": " + detail);
        this.kind = kind;
    }

    public Kind kind() {
        return kind;
    }
}
