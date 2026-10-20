//! Shared geometry, with colors selected for the actual native runtime.
use gtk::gdk;

const SHARED: &str = include_str!("../style.css");
const MODERN: &str = include_str!("../style-modern.css");
const LEGACY: &str = include_str!("../style-legacy.css");

pub(super) fn legacy_runtime() -> bool {
    gtk::check_version(4, 16, 0).is_some() || (adw::major_version(), adw::minor_version()) < (1, 6)
}

pub(super) fn stylesheet(legacy: bool, dark: bool) -> String {
    // Active-toggle variables were only added in libadwaita 1.7. The named
    // fallback also serves 1.5, and must follow runtime System theme changes.
    let toggle = if dark {
        "rgba(255, 255, 255, 0.2)"
    } else {
        "white"
    };
    format!(
        "@define-color qf_toggle_bg {toggle};\n{SHARED}\n{}",
        if legacy { LEGACY } else { MODERN }
    )
}

pub(super) fn load() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    let manager = adw::StyleManager::default();
    let legacy = legacy_runtime();
    provider.load_from_string(&stylesheet(legacy, manager.is_dark()));
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    manager.connect_dark_notify(move |manager| {
        provider.load_from_string(&stylesheet(legacy, manager.is_dark()));
    });
}
