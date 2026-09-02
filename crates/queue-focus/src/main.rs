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
  restart         stop the background service and start it again
  quit            exit the background service until the next request
  version         print the version
";

fn main() -> glib::ExitCode {
    let first_arg = std::env::args().nth(1);
    let restart = first_arg.as_deref() == Some("restart");
    match first_arg.as_deref() {
        Some("version" | "--version") => {
            println!(concat!("queue-focus ", env!("CARGO_PKG_VERSION")));
            return glib::ExitCode::from(0);
        }
        Some("service") => {}
        // Nothing to stop: just start the installed binary.
        Some("restart") if !service_running() => return started(ensure_service()),
        _ => {
            ensure_service();
        }
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

    if !restart {
        return app.run();
    }
    // A restart is a `quit` sent to the running service, followed by starting
    // it again from here. The service announces that it is stopping on purpose,
    // so the top-bar extension leaves the restart to us.
    let argv0 = std::env::args()
        .next()
        .unwrap_or_else(|| "queue-focus".into());
    let code = app.run_with_args(&[argv0, "quit".into()]);
    if code != glib::ExitCode::from(0) {
        return code;
    }
    if !wait_for_release() {
        eprintln!("queue-focus: the running service did not stop; restart failed");
        return glib::ExitCode::from(1);
    }
    started(ensure_service())
}

fn started(running: bool) -> glib::ExitCode {
    if !running {
        eprintln!("queue-focus: the service could not be started");
    }
    glib::ExitCode::from(u8::from(!running))
}

fn session_bus() -> Option<gio::DBusConnection> {
    gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok()
}

fn bus_call(
    conn: &gio::DBusConnection,
    method: &str,
    args: glib::Variant,
) -> Result<glib::Variant, glib::Error> {
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
}

fn name_has_owner(conn: &gio::DBusConnection) -> bool {
    bus_call(conn, "NameHasOwner", (APP_ID,).to_variant())
        .ok()
        .and_then(|v| v.get::<(bool,)>())
        .map(|(b,)| b)
        .unwrap_or(false)
}

fn service_running() -> bool {
    session_bus().is_some_and(|conn| name_has_owner(&conn))
}

/// Make sure a background service owns the bus name so this invocation acts as
/// a short-lived remote rather than becoming the (terminal-blocking) primary.
/// Returns whether the name is owned afterwards.
fn ensure_service() -> bool {
    use std::os::unix::process::CommandExt;
    let Some(conn) = session_bus() else {
        return false;
    };
    if name_has_owner(&conn) {
        return true;
    }
    // Preferred: D-Bus activation (service file installed). The bus replies
    // once the activated service owns the name.
    if bus_call(&conn, "StartServiceByName", (APP_ID, 0u32).to_variant()).is_ok() {
        return true;
    }
    // Fallback: spawn ourselves detached.
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let spawned = std::process::Command::new(exe)
        .arg("service")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
    if spawned.is_err() {
        return false;
    }
    poll(|| name_has_owner(&conn))
}

/// Give a service that was just told to quit time to let go of the bus name,
/// so the next activation starts a fresh process.
fn wait_for_release() -> bool {
    let Some(conn) = session_bus() else {
        return false;
    };
    poll(|| !name_has_owner(&conn))
}

/// Check `done` every 100 ms for up to five seconds.
fn poll(done: impl Fn() -> bool) -> bool {
    for _ in 0..50 {
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
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
        // The remote that sends `restart` starts the service again itself.
        Some("quit") | Some("restart") => {
            if let Some(conn) = app.dbus_connection() {
                dbus::announce_stopping(&conn);
            }
            app.quit();
        }
        Some("version") | Some("--version") => {
            cmd.print_literal(concat!("queue-focus ", env!("CARGO_PKG_VERSION"), "\n"));
        }
        Some("done") => match command_update(cmd, state.complete_current()) {
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
