//! Session-bus API used by the GNOME Shell extension (and anything else).
//! Bus name is the application id; object path /org/queuefocus/QueueFocus.

use crate::state::SharedState;
use crate::ui::{Page, Ui};
use adw::prelude::*;
use gtk::{gio, glib};
use qf_core::{Bucket, Tag};
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
    <method name="CompleteCurrent"><arg type="b" name="completed" direction="out"/></method>
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
  </interface>
</node>
"#;

/// Export the object on `conn` and start broadcasting state changes and
/// durability warnings.
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
        .method_call(move |_conn, _sender, _path, _iface, method, params, inv| {
            handle(&st, &ui, method, params, inv);
        })
        .build()?;

    let st = state.clone();
    let changed_conn = conn.clone();
    state.on_change(move || {
        if changed_conn.is_closed() {
            return;
        }
        let json = st.store().snapshot_json();
        if let Err(e) =
            changed_conn.emit_signal(None, PATH, IFACE, "Changed", Some(&(json,).to_variant()))
        {
            eprintln!("queue-focus: emit Changed failed: {e}");
        }
    });
    let warning_conn = conn.clone();
    state.on_durability_warning(move |warning| {
        if warning_conn.is_closed() {
            return;
        }
        if let Err(e) = warning_conn.emit_signal(
            None,
            PATH,
            IFACE,
            "DurabilityWarning",
            Some(&(warning.to_string(),).to_variant()),
        ) {
            eprintln!("queue-focus: emit DurabilityWarning failed: {e}");
        }
    });
    Ok(id)
}

fn handle(
    state: &SharedState,
    ui: &Rc<Ui>,
    method: &str,
    params: glib::Variant,
    inv: gio::DBusMethodInvocation,
) {
    let bad = |inv: gio::DBusMethodInvocation, msg: &str| {
        inv.return_dbus_error("org.queuefocus.Error.InvalidArgs", msg);
    };
    let persistence = |inv: gio::DBusMethodInvocation, error: &std::io::Error| {
        inv.return_dbus_error("org.queuefocus.Error.Persistence", &error.to_string());
    };
    match method {
        "GetState" => inv.return_value(Some(&(state.store().snapshot_json(),).to_variant())),
        "Add" => {
            let Some((text, bucket)) = params.get::<(String, String)>() else {
                return bad(inv, "expected (ss)");
            };
            let default = Bucket::parse(&bucket).unwrap_or(Bucket::Next);
            match state
                .update(|s| s.quick_add(&text, default))
                .map(|o| o.into_value())
            {
                Ok(Some(id)) => inv.return_value(Some(&(id,).to_variant())),
                Ok(None) => bad(inv, "empty title"),
                Err(e) => persistence(inv, &e),
            }
        }
        "CompleteCurrent" => match state
            .update(|s| s.complete_current())
            .map(|o| o.into_value())
        {
            Ok(done) => inv.return_value(Some(&(done.is_some(),).to_variant())),
            Err(e) => persistence(inv, &e),
        },
        "Promote" | "Remove" => {
            let Some((id,)) = params.get::<(u64,)>() else {
                return bad(inv, "expected (t)");
            };
            let ok = state
                .update(|s| {
                    if method == "Promote" {
                        s.promote(id)
                    } else {
                        s.remove(id)
                    }
                })
                .map(|o| o.into_value());
            match ok {
                Ok(true) => inv.return_value(None),
                Ok(false) => bad(inv, "no such task"),
                Err(e) => persistence(inv, &e),
            }
        }
        "Move" => {
            let Some((id, bucket, index)) = params.get::<(u64, String, i32)>() else {
                return bad(inv, "expected (tsi)");
            };
            let Some(bucket) = Bucket::parse(&bucket) else {
                return bad(inv, "bad bucket");
            };
            let index = usize::try_from(index).ok();
            let ok = state
                .update(|s| s.move_to(id, bucket, index))
                .map(|o| o.into_value());
            match ok {
                Ok(true) => inv.return_value(None),
                Ok(false) => bad(inv, "no such task"),
                Err(e) => persistence(inv, &e),
            }
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
            let ok = state.update(|s| s.set_tag(id, tag)).map(|o| o.into_value());
            match ok {
                Ok(true) => inv.return_value(None),
                Ok(false) => bad(inv, "no such task"),
                Err(e) => persistence(inv, &e),
            }
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
