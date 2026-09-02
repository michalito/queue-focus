//! Session-bus API used by the GNOME Shell extension (and anything else).
//! Bus name is the application id; object path /org/queuefocus/QueueFocus.

use crate::state::{DurabilityWarning, SharedState, UpdateOutcome};
use crate::ui::{Page, Ui};
use adw::prelude::*;
use gtk::{gio, glib};
use qf_core::{Bucket, Tag};
use std::io;
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
    <method name="UndoComplete">
      <arg type="t" name="id" direction="in"/>
      <arg type="b" name="undone" direction="out"/>
    </method>
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
    <signal name="Changed"><arg type="s" name="json"/></signal>
    <signal name="DurabilityWarning"><arg type="s" name="message"/></signal>
    <signal name="Stopping"/>
  </interface>
</node>
"#;

const ERR_INVALID_ARGS: &str = "org.queuefocus.Error.InvalidArgs";
const ERR_PERSISTENCE: &str = "org.queuefocus.Error.Persistence";

/// Export the object on `conn` and start broadcasting state changes.
/// Called from `QfApplication::dbus_register`, i.e. before the bus name is owned.
pub fn export(
    conn: &gio::DBusConnection,
    state: &SharedState,
    ui: &Rc<Ui>,
) -> Result<gio::RegistrationId, glib::Error> {
    let node = gio::DBusNodeInfo::for_xml(XML)?;
    let iface = node.lookup_interface(IFACE).expect("interface in XML");

    let st = state.clone();
    let ui = ui.clone();
    let id = conn
        .register_object(PATH, &iface)
        .method_call(move |conn, _sender, _path, _iface, method, params, inv| {
            handle(&conn, &st, &ui, method, params, inv);
        })
        .build()?;

    let st = state.clone();
    let changed_conn = conn.clone();
    state.on_change(move || {
        let json = st.store().snapshot_json();
        emit(&changed_conn, "Changed", Some(&(json,).to_variant()));
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

fn handle(
    conn: &gio::DBusConnection,
    state: &SharedState,
    ui: &Rc<Ui>,
    method: &str,
    params: glib::Variant,
    inv: gio::DBusMethodInvocation,
) {
    let bad = |inv: gio::DBusMethodInvocation, msg: &str| {
        inv.return_dbus_error(ERR_INVALID_ARGS, msg);
    };
    match method {
        "GetState" => inv.return_value(Some(&(state.store().snapshot_json(),).to_variant())),
        "Add" => {
            let Some((text, bucket)) = params.get::<(String, String)>() else {
                return bad(inv, "expected (ss)");
            };
            let default = Bucket::parse(&bucket).unwrap_or(Bucket::Next);
            let result = state.update(|s| s.quick_add(&text, default));
            reply(conn, inv, result, |id| {
                id.map(|id| Some((id,).to_variant())).ok_or("empty title")
            });
        }
        // Replies with id 0 and an empty title when Now was empty.
        "CompleteCurrent" => reply(conn, inv, state.complete_current(), |done| {
            let (id, title) = done.map(|t| (t.id, t.title)).unwrap_or_default();
            Ok(Some((id, title).to_variant()))
        }),
        "UndoComplete" => {
            let Some((id,)) = params.get::<(u64,)>() else {
                return bad(inv, "expected (t)");
            };
            reply(conn, inv, state.undo_complete(id), |undone| {
                Ok(Some((undone,).to_variant()))
            });
        }
        "Promote" | "Remove" => {
            let Some((id,)) = params.get::<(u64,)>() else {
                return bad(inv, "expected (t)");
            };
            let result = state.update(|s| {
                if method == "Promote" {
                    s.promote(id)
                } else {
                    s.remove(id)
                }
            });
            reply(conn, inv, result, found);
        }
        "Move" => {
            let Some((id, bucket, index)) = params.get::<(u64, String, i32)>() else {
                return bad(inv, "expected (tsi)");
            };
            let Some(bucket) = Bucket::parse(&bucket) else {
                return bad(inv, "bad bucket");
            };
            let index = usize::try_from(index).ok();
            let result = state.update(|s| s.move_to(id, bucket, index));
            reply(conn, inv, result, found);
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
            let result = state.update(|s| s.set_tag(id, tag));
            reply(conn, inv, result, found);
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
        _ => inv.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "unknown method"),
    }
}

/// Answer a mutating call. A failure to persist is a D-Bus error; a mutation
/// that committed with a durability warning still succeeds, and the warning
/// is broadcast because the caller is the one who should hear about it.
/// `to_reply` turns the mutation's value into the return value, or into an
/// InvalidArgs message when the arguments referred to nothing.
fn reply<R>(
    conn: &gio::DBusConnection,
    inv: gio::DBusMethodInvocation,
    result: io::Result<UpdateOutcome<R>>,
    to_reply: impl FnOnce(R) -> Result<Option<glib::Variant>, &'static str>,
) {
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(e) => return inv.return_dbus_error(ERR_PERSISTENCE, &e.to_string()),
    };
    let (value, warning) = outcome.into_parts();
    match to_reply(value) {
        Ok(value) => inv.return_value(value.as_ref()),
        Err(msg) => inv.return_dbus_error(ERR_INVALID_ARGS, msg),
    }
    if let Some(warning) = warning {
        emit_warning(conn, &warning);
    }
}

fn emit_warning(conn: &gio::DBusConnection, warning: &DurabilityWarning) {
    emit(
        conn,
        "DurabilityWarning",
        Some(&(warning.to_string(),).to_variant()),
    );
}

/// Reply for calls that name a task and return nothing.
fn found(found: bool) -> Result<Option<glib::Variant>, &'static str> {
    found.then_some(None).ok_or("no such task")
}
