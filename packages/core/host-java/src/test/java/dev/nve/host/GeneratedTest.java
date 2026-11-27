package dev.nve.host;

import dev.nve.host.generated.NveAbi;
import dev.nve.host.generated.NveTables;
import java.nio.ByteBuffer;

/**
 * {@code nve codegen abi} output against this package's static runtime: every generated table key
 * is {@code xxh64(name)}, every field spec fits its row, and a {@code TableReader} reads a
 * generated default row with a generated field spec.
 */
final class GeneratedTest {
    private GeneratedTest() {}

    static void run(Check c) {
        for (NveTables.Info t : NveTables.all()) {
            c.that(t.key.equals(Key64.of(t.name)), t.name + " key is xxh64(name)");
            c.that(t.words >= 1 && t.words <= TableReader.MAX_WORDS, t.name + " word count");
            for (FieldSpec f : t.fields()) c.that(f.fits(t.words), t.name + " field fits its row");
        }
        c.eq(NveAbi.SIZEOF_RENDER_NODE, 64, "NveRenderNode stride");
        c.eq(NveAbi.SIZEOF_TABLE_VIEW_64, 48, "NveTableView 64-bit size");
        c.eq(NveAbi.SIZEOF_TABLE_VIEW_32, 36, "NveTableView 32-bit size");

        final int[] defaults = NveTables.Layout.defaults();
        final ByteBuffer rows = TableSceneTest.words(defaults);
        final TableReader layout =
                new TableReader().attach(NveTables.Layout.KEY, 0, NveTables.Layout.WORDS, 1, rows, null, null, 0);
        c.that(layout.f32(0, NveTables.Layout.FLEX_SHRINK) == 1.0f, "Layout.flex_shrink default is 1.0");
        defaults[0] = 7;
        c.eq(NveTables.Layout.defaults()[0], 2, "defaults() returns a fresh copy");
    }
}
