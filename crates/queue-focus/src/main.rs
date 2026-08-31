mod app;
mod dbus;
mod state;
mod ui;

use adw::prelude::*;
use gtk::{gio, glib};
use qf_core::Bucket;
use std::rc::Rc;
use ui::Page;

pub const APP_ID: &str = "org.queuefocus.QueueFocus";

const USAGE: &str = "\
usage: queue-focus [COMMAND]

  toggle          show/hide the queue panel (default)
  queue | show    show the queue panel
  board           show the board
  add [TEXT]      add a task (opens quick-add if TEXT omitted)
                  syntax: '!title' -> Now, '#w'/'#p' tag, '@later'/'@side'
  done            complete (delete) the current task
  status          print the current state as JSON
  hide            hide all windows
  service         start in the background only (used by D-Bus activation)
  quit            exit the background service
  version         print the version
";

fn main() -> glib::ExitCode {
    let first_arg = std::env::args().nth(1);
    if matches!(first_arg.as_deref(), Some("version" | "--version")) {
        println!(concat!("queue-focus ", env!("CARGO_PKG_VERSION")));
        return glib::ExitCode::from(0);
    }
    if first_arg.as_deref() != Some("service") {
        ensure_service();
    }
    let state = match state::State::load() {
        Ok(state) => state,
        Err(e) => {
            eprintln!("queue-focus: {e}; refusing to start to protect the task file");
            return glib::ExitCode::from(1);
        }
    };
    let app = app::QfApplication::new(APP_ID, state);

    app.connect_startup(|app| {
        ui::load_css();
        gtk::Window::set_default_icon_name(APP_ID);
        // Keep running with no windows so the top-bar extension always has a service.
        std::mem::forget(app.hold());
    });

    app.connect_activate(|app| app.ui().show(Page::Queue));

    app.connect_command_line(|app, cmd| {
        glib::ExitCode::from(run_command(app, cmd, &app.ui(), &app.state()) as u8)
    });

    app.run()
}

/// Make sure a background service owns the bus name so this invocation acts as
/// a short-lived remote rather than becoming the (terminal-blocking) primary.
fn ensure_service() {
    use std::os::unix::process::CommandExt;
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return;
    };
    let bus_call = |method: &str, args: glib::Variant| {
        conn.call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            method,
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            5000,
            gio::Cancellable::NONE,
        )
    };
    let has_owner = || {
        bus_call("NameHasOwner", (APP_ID,).to_variant())
            .ok()
            .and_then(|v| v.get::<(bool,)>())
            .map(|(b,)| b)
            .unwrap_or(false)
    };
    if has_owner() {
        return;
    }
    // Preferred: D-Bus activation (service file installed).
    if bus_call("StartServiceByName", (APP_ID, 0u32).to_variant()).is_ok() {
        return;
    }
    // Fallback: spawn ourselves detached.
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let spawned = std::process::Command::new(exe)
        .arg("service")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
    if spawned.is_err() {
        return;
    }
    for _ in 0..50 {
        if has_owner() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn run_command(
    app: &app::QfApplication,
    cmd: &gio::ApplicationCommandLine,
    ui: &Rc<ui::Ui>,
    state: &state::SharedState,
) -> i32 {
    let args: Vec<String> = cmd
        .arguments()
        .iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    match args.first().map(String::as_str) {
        None | Some("toggle") => ui.toggle(),
        Some("queue") | Some("show") => ui.show(Page::Queue),
        Some("board") => ui.show(Page::Board),
        Some("hide") => ui.hide(),
        Some("service") => {}
        Some("quit") => app.quit(),
        Some("version") | Some("--version") => {
            cmd.print_literal(concat!("queue-focus ", env!("CARGO_PKG_VERSION"), "\n"));
        }
        Some("done") => match command_update(cmd, state.update(|s| s.complete_current())) {
            Ok(Some(t)) => cmd.print_literal(&format!("done: {}\n", t.title)),
            Ok(None) => cmd.print_literal("nothing in Now\n"),
            Err(e) => {
                cmd.printerr_literal(&format!("queue-focus: {e}\n"));
                return 1;
            }
        },
        Some("status") => {
            let json = state.store().snapshot_json();
            cmd.print_literal(&format!("{json}\n"));
        }
        Some("add") => {
            let text = args[1..].join(" ");
            if text.trim().is_empty() {
                ui.quick_add_dialog();
            } else {
                match command_update(cmd, state.update(|s| s.quick_add(&text, Bucket::Next))) {
                    Ok(Some(id)) => cmd.print_literal(&format!("added #{id}\n")),
                    Ok(None) => {
                        cmd.printerr_literal("nothing to add\n");
                        return 1;
                    }
                    Err(e) => {
                        cmd.printerr_literal(&format!("queue-focus: {e}\n"));
                        return 1;
                    }
                }
            }
        }
        Some("-h") | Some("--help") | Some("help") => cmd.print_literal(USAGE),
        Some(other) => {
            cmd.printerr_literal(&format!("unknown command: {other}\n\n{USAGE}"));
            return 2;
        }
    }
    0
}

fn command_update<R>(
    cmd: &gio::ApplicationCommandLine,
    result: std::io::Result<state::UpdateOutcome<R>>,
) -> std::io::Result<R> {
    result.map(|outcome| {
        let (value, warning) = outcome.into_parts();
        if let Some(warning) = warning {
            cmd.printerr_literal(&format!("queue-focus: warning: {warning}\n"));
        }
        value
    })
}
