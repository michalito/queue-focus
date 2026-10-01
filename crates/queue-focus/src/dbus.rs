//! Session-bus API used by the GNOME Shell extension (and anything else).
//! Bus name is the application id; object path /org/queuefocus/QueueFocus.

use crate::service::SharedService;
use crate::ui::{Page, Ui};
use adw::prelude::*;
use gtk::{gio, glib};
use qf_core::{Bucket, EngineError, Outcome, Tag};
use std::rc::Rc;

pub const PATH: &str = "/org/queuefocus/QueueFocus";
pub const IFACE: &str = "org.queuefocus.QueueFocus1";

const XML: &str = r#"
<node>
  <interface name="org.queuefocus.QueueFocus1">
    <method name="GetState"><arg type="s" name="json" direction="out"/></method>
    <method name="Add">
      <arg type="s" name="text" direction="in"/>
      <arg type="s" name="bucket" direction="in"/>
      <arg type="t" name="id" direction="out"/>
    </method>
    <method name="CompleteCurrent">
      <arg type="t" name="id" direction="out"/>
      <arg type="s" name="title" direction="out"/>
    </method>
    <method name="Complete"><arg type="t" name="id" direction="in"/></method>
    <method name="UndoComplete">
      <arg type="t" name="id" direction="in"/>
      <arg type="b" name="undone" direction="out"/>
    </method>
    <method name="TogglePause"><arg type="b" name="toggled" direction="out"/></method>
    <method name="Promote"><arg type="t" name="id" direction="in"/></method>
    <method name="Remove"><arg type="t" name="id" direction="in"/></method>
    <method name="Move">
      <arg type="t" name="id" direction="in"/>
      <arg type="s" name="bucket" direction="in"/>
      <arg type="i" name="index" direction="in"/>
    </method>
    <method name="SetTag">
      <arg type="t" name="id" direction="in"/>
      <arg type="s" name="tag" direction="in"/>
    </method>
    <method name="Show"><arg type="s" name="view" direction="in"/></method>
    <method name="Hide"/>
    <method name="GetSettings"><arg type="s" name="json" direction="out"/></method>
    <method name="SetSettings">
      <arg type="s" name="json" direction="in"/>
      <arg type="s" name="settings" direction="out"/>
    </method>
    <signal name="Changed"><arg type="s" name="json"/></signal>
    <signal name="SettingsChanged"><arg type="s" name="json"/></signal>
    <signal name="Flash"><arg type="s" name="json"/></signal>
    <signal name="DurabilityWarning"><arg type="s" name="message"/></signal>
    <signal name="Stopping"/>
  </interface>
</node>
"#;

const ERR_INVALID_ARGS: &str = "org.queuefocus.Error.InvalidArgs";
const ERR_PERSISTENCE: &str = "org.queuefocus.Error.Persistence";

/// Export the object on `conn` and start broadcasting changes.
/// Called from `QfApplication::dbus_register`, i.e. before the bus name is owned.
pub fn export(
    conn: &gio::DBusConnection,
    service: &SharedService,
    ui: &Rc<Ui>,
) -> Result<gio::RegistrationId, glib::Error> {
    let node = gio::DBusNodeInfo::for_xml(XML)?;
    let iface = node.lookup_interface(IFACE).expect("interface in XML");

    let svc = service.clone();
    let ui = ui.clone();
    let id = conn
        .register_object(PATH, &iface)
        .method_call(move |conn, _sender, _path, _iface, method, params, inv| {
            handle(&conn, &svc, &ui, method, params, inv);
        })
        .build()?;

    let svc = Rc::downgrade(service);
    let changed_conn = conn.clone();
    service.on_change(move || {
        if let Some(svc) = svc.upgrade() {
            let json = svc.engine().state_json();
            emit(&changed_conn, "Changed", Some(&(json,).to_variant()));
        }
    });

    let svc = Rc::downgrade(service);
    let changed_conn = conn.clone();
    service.on_settings_change(move || {
        if let Some(svc) = svc.upgrade() {
            let json = svc.engine().settings_json();
            emit(
                &changed_conn,
                "SettingsChanged",
                Some(&(json,).to_variant()),
            );
        }
    });

    let flash_conn = conn.clone();
    service.set_flash_emitter(move |event| {
        emit(&flash_conn, "Flash", Some(&(event.to_json(),).to_variant()));
    });
    Ok(id)
}

/// Tell clients the service is exiting because it was asked to, so the
/// top-bar extension does not mistake the lost bus name for a crash and
/// start the service again.
pub fn announce_stopping(conn: &gio::DBusConnection) {
    emit(conn, "Stopping", None);
    // The process exits right after this; make sure the signal leaves the socket.
    if let Err(e) = conn.flush_sync(gio::Cancellable::NONE) {
        eprintln!("queue-focus: flush before exit failed: {e}");
    }
}

fn emit(conn: &gio::DBusConnection, signal: &str, args: Option<&glib::Variant>) {
    if conn.is_closed() {
        return;
    }
    if let Err(e) = conn.emit_signal(None, PATH, IFACE, signal, args) {
        eprintln!("queue-focus: emit {signal} failed: {e}");
    }
}

/// Each method is one engine request. Only the argument parsing and the reply
/// are D-Bus's.
fn handle(
    conn: &gio::DBusConnection,
    service: &SharedService,
    ui: &Rc<Ui>,
    method: &str,
    params: glib::Variant,
    inv: gio::DBusMethodInvocation,
) {
    let bad = |inv: gio::DBusMethodInvocation, msg: &str| {
        inv.return_dbus_error(ERR_INVALID_ARGS, msg);
    };
    match method {
        "GetState" => {
            let json = service.engine().state_json();
            inv.return_value(Some(&(json,).to_variant()))
        }
        "Add" => {
            let Some((text, bucket)) = params.get::<(String, String)>() else {
                return bad(inv, "expected (ss)");
            };
            // An unnamed bucket is the one the user chose in Settings.
            let result = service.request(|e| e.add(&text, Bucket::parse(&bucket)));
            reply(conn, inv, result, |id| Some((id,).to_variant()));
        }
        // Replies with id 0 and an empty title when Now was empty.
        "CompleteCurrent" => {
            let result = service.request(|e| e.complete_current());
            reply(conn, inv, result, |done| {
                let (id, title) = done.map(|t| (t.id, t.title)).unwrap_or_default();
                Some((id, title).to_variant())
            });
        }
        "Complete" | "Promote" | "Remove" => {
            let Some((id,)) = params.get::<(u64,)>() else {
                return bad(inv, "expected (t)");
            };
            let result = service.request(|e| match method {
                "Complete" => e.complete(id),
                "Promote" => e.promote(id),
                _ => e.remove(id),
            });
            reply(conn, inv, result, |()| None);
        }
        "UndoComplete" => {
            let Some((id,)) = params.get::<(u64,)>() else {
                return bad(inv, "expected (t)");
            };
            let result = service.request(|e| e.undo_complete(id));
            reply(conn, inv, result, |undone| Some((undone,).to_variant()));
        }
        // Replies false when there is no running or paused clock to toggle.
        "TogglePause" => {
            let result = service.request(|e| e.toggle_pause());
            reply(conn, inv, result, |toggled| Some((toggled,).to_variant()));
        }
        "Move" => {
            let Some((id, bucket, index)) = params.get::<(u64, String, i32)>() else {
                return bad(inv, "expected (tsi)");
            };
            let Some(bucket) = Bucket::parse(&bucket) else {
                return bad(inv, "bad bucket");
            };
            // A negative index is the end of the bucket.
            let index = usize::try_from(index).ok();
            let result = service.request(|e| e.move_to(id, bucket, index));
            reply(conn, inv, result, |()| None);
        }
        "SetTag" => {
            let Some((id, tag)) = params.get::<(u64, String)>() else {
                return bad(inv, "expected (ts)");
            };
            let tag = if tag.is_empty() {
                None
            } else {
                match Tag::parse(&tag) {
                    Some(t) => Some(t),
                    None => return bad(inv, "bad tag"),
                }
            };
            let result = service.request(|e| e.set_tag(id, tag));
            reply(conn, inv, result, |()| None);
        }
        "Show" => {
            let Some((view,)) = params.get::<(String,)>() else {
                return bad(inv, "expected (s)");
            };
            match view.as_str() {
                "add" => ui.quick_add_dialog(),
                "toggle" => ui.toggle(),
                v => ui.show(Page::parse(v)),
            }
            inv.return_value(None);
        }
        "Hide" => {
            ui.hide();
            inv.return_value(None);
        }
        "GetSettings" => {
            let json = service.engine().settings_json();
            inv.return_value(Some(&(json,).to_variant()))
        }
        "SetSettings" => {
            // GDBus validates this against the introspection signature before
            // dispatch. Borrow the string so an oversized call is rejected
            // without first copying its entire body into a Rust String.
            let Some(json_arg) = params.try_child_value(0) else {
                return bad(inv, "expected (s)");
            };
            let Some(json) = json_arg.str() else {
                return bad(inv, "expected (s)");
            };
            match service.request(|e| e.set_settings(json).map(|_| e.settings_json())) {
                Ok(settings) => inv.return_value(Some(&(settings,).to_variant())),
                Err(e) => reply_error(inv, e),
            }
        }
        _ => inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "unknown method"),
    }
}

/// Answer a task request. A refusal is a D-Bus error; a change that committed
/// with a durability warning still succeeds, and the warning is broadcast
/// because the caller is the one who should hear about it.
fn reply<R>(
    conn: &gio::DBusConnection,
    inv: gio::DBusMethodInvocation,
    result: Result<Outcome<R>, EngineError>,
    to_reply: impl FnOnce(R) -> Option<glib::Variant>,
) {
    match result {
        Ok(outcome) => {
            let (value, warning) = outcome.into_parts();
            inv.return_value(to_reply(value).as_ref());
            if let Some(warning) = warning {
                emit(
                    conn,
                    "DurabilityWarning",
                    Some(&(warning.to_string(),).to_variant()),
                );
            }
        }
        Err(e) => reply_error(inv, e),
    }
}

fn reply_error(inv: gio::DBusMethodInvocation, error: EngineError) {
    let name = match error {
        EngineError::InvalidArgument(_) => ERR_INVALID_ARGS,
        EngineError::Persistence(_) => ERR_PERSISTENCE,
    };
    inv.return_dbus_error(name, &error.to_string());
}
