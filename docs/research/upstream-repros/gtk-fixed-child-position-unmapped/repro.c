/* gtk_fixed_get_child_position ignores the position set by put/move until
 * the GtkFixed has been allocated, and goes stale after gtk_fixed_move(). */
#include <gtk/gtk.h>

static int failures;

static void check (const char *label, GtkFixed *fixed, GtkWidget *child, double ex, double ey)
{
  double x = -1, y = -1, tx = -1, ty = -1;
  gtk_fixed_get_child_position (fixed, child, &x, &y);
  float fx = 0, fy = 0;
  gsk_transform_to_translate (gtk_fixed_get_child_transform (fixed, child), &fx, &fy);
  tx = fx; ty = fy;
  gboolean ok = x == ex && y == ey;
  if (!ok) failures++;
  g_print ("%-38s get_child_position=(%g,%g) get_child_transform=(%g,%g) expected=(%g,%g) %s\n",
           label, x, y, tx, ty, ex, ey, ok ? "OK" : "MISMATCH");
}

int main (void)
{
  g_print ("GTK runtime %u.%u.%u\n", gtk_get_major_version (), gtk_get_minor_version (), gtk_get_micro_version ());
  if (!gtk_init_check ()) { g_printerr ("gtk_init_check failed (no display)\n"); return 2; }

  GtkWidget *fixed = gtk_fixed_new ();
  GtkWidget *child = gtk_label_new ("x");
  g_object_ref_sink (fixed);

  gtk_fixed_put (GTK_FIXED (fixed), child, 10, 20);
  check ("1. after put, unrooted/unallocated", GTK_FIXED (fixed), child, 10, 20);

  /* Allocate without a root/window (as GTK's own unit tests do). */
  gtk_widget_measure (fixed, GTK_ORIENTATION_HORIZONTAL, -1, NULL, NULL, NULL, NULL);
  gtk_widget_allocate (fixed, 200, 200, -1, NULL);
  check ("2. after gtk_widget_allocate", GTK_FIXED (fixed), child, 10, 20);

  gtk_fixed_move (GTK_FIXED (fixed), child, 30, 40);
  check ("3. after move, before re-allocate", GTK_FIXED (fixed), child, 30, 40);

  gtk_widget_allocate (fixed, 200, 200, -1, NULL);
  check ("4. after move + re-allocate", GTK_FIXED (fixed), child, 30, 40);

  g_object_unref (fixed);
  g_print ("%d mismatch(es)\n", failures);
  return failures ? 1 : 0;
}
