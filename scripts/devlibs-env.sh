# Source this to build WITHOUT the -dev packages / pkg-config, linking straight
# against the GTK/libadwaita runtime libraries already on the system.
#   source scripts/devlibs-env.sh && cargo build --release
# Not needed if libgtk-4-dev + libadwaita-1-dev are installed.
_qf_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
_qf_libs="$_qf_root/.devlibs"
mkdir -p "$_qf_libs"
_L=/usr/lib/$(uname -m)-linux-gnu
for p in gtk-4:1 adwaita-1:0 glib-2.0:0 gobject-2.0:0 gio-2.0:0 gmodule-2.0:0 pango-1.0:0 pangocairo-1.0:0 cairo:2 cairo-gobject:2 gdk_pixbuf-2.0:0 graphene-1.0:0; do
  n=${p%%:*}; v=${p##*:}
  [ -e "$_qf_libs/lib$n.so" ] || ln -sf "$_L/lib$n.so.$v" "$_qf_libs/lib$n.so"
done
# system-deps env overrides: NAME = crate's pkg name, upper-cased, non-alnum → _
for dep in GLIB_2_0:glib-2.0 GOBJECT_2_0:gobject-2.0 GIO_2_0:gio-2.0 GMODULE_2_0:gmodule-2.0 \
           PANGO:pango-1.0 PANGOCAIRO:pangocairo-1.0 CAIRO:cairo CAIRO_GOBJECT:cairo-gobject \
           GDK_PIXBUF_2_0:gdk_pixbuf-2.0 GRAPHENE_GOBJECT_1_0:graphene-1.0 GTK4:gtk-4 LIBADWAITA_1:adwaita-1; do
  name=${dep%%:*}; lib=${dep##*:}
  export SYSTEM_DEPS_${name}_NO_PKG_CONFIG=1
  export SYSTEM_DEPS_${name}_LIB="$lib"
  export SYSTEM_DEPS_${name}_SEARCH_NATIVE="$_qf_libs"
done
export PATH="$HOME/.cargo/bin:$PATH"
