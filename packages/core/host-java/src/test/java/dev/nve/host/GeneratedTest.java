package dev.nve.host;

import dev.nve.host.generated.NveAbi;
import dev.nve.host.generated.NveModuleTables;
import dev.nve.host.generated.NveRuntimeAbi;
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
        for (NveTables.Info t : NveTables.all()) table(c, t.key, t.name, t.words, t.fields());
        c.that(NveModuleTables.all().length > 0, "capability-module tables are generated");
        for (NveModuleTables.Info t : NveModuleTables.all()) table(c, t.key, t.name, t.words, t.fields());
        c.eq(NveRuntimeAbi.STATUS_OK, 0, "NveRuntimeStatus values are STATUS_*");
        c.eq(NveRuntimeAbi.SIZEOF_NODE_ROW, 32, "NveNodeRow stride");
        // ABI v4: 8 u32 header words (incl. the row-index lengths), then 5 pointers.
        c.eq(NveAbi.SIZEOF_TABLE_VIEW_64, 32 + 5 * 8, "NveTableView 64-bit size");
        c.eq(NveAbi.SIZEOF_TABLE_VIEW_32, 32 + 5 * 4, "NveTableView 32-bit size");
        c.eq(NveAbi.TABLE_VIEW_OFFSET_ROW_INDEX_DIR_LEN, 24, "NveTableView.row_index_dir_len @24");
        c.eq(NveAbi.TABLE_VIEW_OFFSET_ROW_INDEX_PAGE_COUNT, 28, "NveTableView.row_index_page_count @28");
        c.eq(NveAbi.TABLE_VIEW_OFFSET_ROWS, 32, "NveTableView.rows @32");
        c.eq(NveAbi.TABLE_VIEW_OFFSET_ROW_INDEX_PAGES_64, 32 + 4 * 8, "NveTableView.row_index_pages is last (64-bit)");
        c.eq(NveAbi.TABLE_VIEW_OFFSET_ROW_INDEX_PAGES_32, 32 + 4 * 4, "NveTableView.row_index_pages is last (32-bit)");

        final int[] defaults = NveTables.Layout.defaults();
        final ByteBuffer rows = TableSceneTest.words(defaults);
        final TableReader layout =
                new TableReader().attach(NveTables.Layout.KEY, 0, NveTables.Layout.WORDS, 1, rows, null, null, 0, null, 0, null, 0);
        c.that(layout.f32(0, NveTables.Layout.FLEX_SHRINK) == 1.0f, "Layout.flex_shrink default is 1.0");
        defaults[0] = 7;
        c.eq(NveTables.Layout.defaults()[0], 2, "defaults() returns a fresh copy");

        // A generated two-word STR spec reads its (offset, len) span.
        final FieldSpec label = NveTables.Accessibility.LABEL;
        final int[] row = new int[NveTables.Accessibility.WORDS];
        row[label.word] = 5;
        row[label.word + 1] = 7;
        final TableReader a11y = new TableReader()
                .attach(NveTables.Accessibility.KEY, 0, NveTables.Accessibility.WORDS, 1, TableSceneTest.words(row), null, null, 0, null, 0, null, 0);
        c.eq(a11y.strOffset(0, label), 5, "Accessibility.label offset");
        c.eq(a11y.strLen(0, label), 7, "Accessibility.label len");
    }

    private static void table(Check c, Key64 key, String name, int words, FieldSpec[] fields) {
        c.that(key.equals(Key64.of(name)), name + " key is xxh64(name)");
        c.that(words >= 1 && words <= TableReader.MAX_WORDS, name + " word count");
        for (FieldSpec f : fields) c.that(f.fits(words), name + " field fits its row");
    }
}
