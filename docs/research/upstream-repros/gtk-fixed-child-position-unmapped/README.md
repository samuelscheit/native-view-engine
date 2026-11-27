# GTK4: `gtk_fixed_get_child_position` reports the last *allocated* offset, not the put/move position

**Verdict:** genuine (minor) upstream bug: the implementation contradicts its documentation. Still present on GTK `main` (checked 2026-11, last gtkfixed.c commit `1be10b01`).

## Versions
- gtk4 4.24.2 (Homebrew, `pkg-config gtk4`), macOS arm64, native macOS backend.

## Contract
Docs (`gtk/gtkfixed.c`, https://docs.gtk.org/gtk4/method.Fixed.get_child_position.html):
> Retrieves the translation transformation of the given child `GtkWidget` in the `GtkFixed`. See also: `get_child_transform`.

`gtk_fixed_put()`/`gtk_fixed_move()` store exactly that translation on the `GtkFixedLayoutChild` synchronously.

## Implementation (main)
```c
if (!gtk_widget_compute_point (widget, GTK_WIDGET (fixed),
                               &GRAPHENE_POINT_INIT (0, 0), &p))
  graphene_point_init (&p, 0, 0);
```
`gtk_widget_compute_point` uses the child's *allocation* transform, which only exists after a size-allocate pass. So before the first allocation the result is `(0,0)`, and after `gtk_fixed_move()` it is stale until the next allocation. (A failed compute is also silently turned into `(0,0)`.) Introduced when GTK4 moved GtkFixed onto GtkFixedLayout; `b23ac5c8` (2023) only swapped `gtk_widget_translate_coordinates` for `compute_point`.

Correction to our `docs/UPSTREAM.md` wording: mapping/rooting is irrelevant — an unrooted fixed that has been `gtk_widget_allocate()`d returns correct values (case 2). The trigger is "not (re)allocated since put/move".

## Repro
`./run.sh` (needs a display; builds into `out/`, gitignored).

Expected: every line `(10,20)` / `(30,40)` as set. Actual (4.24.2):
```
GTK runtime 4.24.2
1. after put, unrooted/unallocated     get_child_position=(0,0) get_child_transform=(10,20) expected=(10,20) MISMATCH
2. after gtk_widget_allocate           get_child_position=(10,20) get_child_transform=(10,20) expected=(10,20) OK
3. after move, before re-allocate      get_child_position=(10,20) get_child_transform=(30,40) expected=(30,40) MISMATCH
4. after move + re-allocate            get_child_position=(30,40) get_child_transform=(30,40) expected=(30,40) OK
2 mismatch(es)
```

## Suggested fix
Read the layout child's transform, matching the docs and `get_child_transform`:
```c
GskTransform *t = gtk_fixed_layout_child_get_transform (child_info);
float fx = 0, fy = 0;
gsk_transform_to_translate (t, &fx, &fy);   /* or handle non-2D-translate category */
```
(For a non-pure-translate transform, transforming the origin point via `gsk_transform_transform_point` keeps today's semantics for rotated/scaled children.)

## Existing reports
None found (GitLab issue/MR API search `gtk_fixed_get_child_position`, web search). File new at https://gitlab.gnome.org/GNOME/gtk/-/issues/new.

## Our mitigation
`hosts/gtk-shell/tests/gtk_shell_test.cpp` reads `gtk_fixed_get_child_transform` + `gsk_transform_to_translate`.
