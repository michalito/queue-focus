//! `adw::Application` subclass. The only reason for the subclass: exporting our
//! D-Bus object in `dbus_register`, which GApplication calls *before* it owns the
//! bus name — so a method call that D-Bus-activates the service is never lost.

use crate::dbus;
use crate::service::{Service, SharedService};
use crate::ui::Ui;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

struct PendingCall {
    conn: gio::DBusConnection,
    method: String,
    params: glib::Variant,
    invocation: gio::DBusMethodInvocation,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct QfApplication {
        pub service: OnceCell<SharedService>,
        pub ui: OnceCell<Rc<Ui>>,
        registration: RefCell<Option<gio::RegistrationId>>,
        pub(super) pending: RefCell<Vec<PendingCall>>,
        pub failure: OnceCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for QfApplication {
        const NAME: &'static str = "QfApplication";
        type Type = super::QfApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for QfApplication {}

    impl ApplicationImpl for QfApplication {
        fn dbus_register(
            &self,
            connection: &gio::DBusConnection,
            object_path: &str,
        ) -> Result<(), glib::Error> {
            self.parent_dbus_register(connection, object_path)?;
            let id = dbus::export(connection, &self.obj())?;
            *self.registration.borrow_mut() = Some(id);
            Ok(())
        }

        fn dbus_unregister(&self, connection: &gio::DBusConnection, object_path: &str) {
            if let Some(id) = self.registration.borrow_mut().take() {
                let _ = connection.unregister_object(id);
            }
            for pending in self.pending.borrow_mut().drain(..) {
                pending.invocation.return_dbus_error(
                    "org.queuefocus.Error.Persistence",
                    "Queue Focus stopped before startup completed.",
                );
            }
            self.parent_dbus_unregister(connection, object_path);
        }
    }

    impl GtkApplicationImpl for QfApplication {}
    impl AdwApplicationImpl for QfApplication {}
}

glib::wrapper! {
    pub struct QfApplication(ObjectSubclass<imp::QfApplication>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl QfApplication {
    pub fn new(app_id: &str) -> Self {
        glib::Object::builder()
            .property("application-id", app_id)
            .property("flags", gio::ApplicationFlags::HANDLES_COMMAND_LINE)
            .build()
    }

    /// Called only by startup, which GApplication emits only in the primary.
    pub fn initialize(&self) -> Result<(), String> {
        let (service, warning) = match Service::open() {
            Ok(opened) => opened,
            Err(error) => {
                let message = error.to_string();
                let _ = self.imp().failure.set(message.clone());
                for pending in self.imp().pending.borrow_mut().drain(..) {
                    pending
                        .invocation
                        .return_dbus_error("org.queuefocus.Error.Persistence", &message);
                }
                return Err(message);
            }
        };
        let ui = Ui::new(self.clone().upcast(), service.clone());
        self.imp()
            .service
            .set(service.clone())
            .ok()
            .expect("service set once");
        self.imp().ui.set(ui.clone()).ok().expect("ui set once");
        if let Some(conn) = self.dbus_connection() {
            dbus::attach(&conn, &service);
        }
        if let Some(warning) = warning {
            eprintln!("queue-focus: {warning}");
            ui.queue_settings_problem("Could not load your settings", &warning);
        }
        let pending = std::mem::take(&mut *self.imp().pending.borrow_mut());
        for call in pending {
            dbus::handle(
                &call.conn,
                &service,
                &ui,
                &call.method,
                call.params,
                call.invocation,
            );
        }
        Ok(())
    }

    pub fn ready(&self) -> bool {
        self.imp().service.get().is_some()
    }

    /// Export exists before bus-name acquisition, but calls that arrive during
    /// registration wait for primary startup. Remote CLI instances never open
    /// the engine or start a worker.
    pub fn dispatch(
        &self,
        conn: gio::DBusConnection,
        method: &str,
        params: glib::Variant,
        invocation: gio::DBusMethodInvocation,
    ) {
        if let Some(message) = self.imp().failure.get() {
            invocation.return_dbus_error("org.queuefocus.Error.Persistence", message);
        } else if self.ready() {
            dbus::handle(
                &conn,
                &self.service(),
                &self.ui(),
                method,
                params,
                invocation,
            );
        } else if self.imp().pending.borrow().len() < 64 {
            self.imp().pending.borrow_mut().push(PendingCall {
                conn,
                method: method.into(),
                params,
                invocation,
            });
        } else {
            invocation.return_dbus_error(
                "org.queuefocus.Error.Starting",
                "Queue Focus is starting; retry this request shortly.",
            );
        }
    }

    pub fn ui(&self) -> Rc<Ui> {
        self.imp().ui.get().expect("ui").clone()
    }

    pub fn service(&self) -> SharedService {
        self.imp().service.get().expect("service").clone()
    }
}
