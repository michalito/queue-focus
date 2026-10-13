//! `adw::Application` subclass. The only reason for the subclass: exporting our
//! D-Bus object in `dbus_register`, which GApplication calls *before* it owns the
//! bus name — so a method call that D-Bus-activates the service is never lost.

use crate::dbus;
use crate::service::SharedService;
use crate::ui::Ui;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct QfApplication {
        pub service: OnceCell<SharedService>,
        pub ui: OnceCell<Rc<Ui>>,
        registration: RefCell<Option<gio::RegistrationId>>,
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
            let (Some(service), Some(ui)) = (self.service.get(), self.ui.get()) else {
                return Ok(());
            };
            let id = dbus::export(connection, service, ui)?;
            *self.registration.borrow_mut() = Some(id);
            Ok(())
        }

        fn dbus_unregister(&self, connection: &gio::DBusConnection, object_path: &str) {
            if let Some(id) = self.registration.borrow_mut().take() {
                let _ = connection.unregister_object(id);
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
    pub fn new(app_id: &str, service: SharedService) -> Self {
        let app: Self = glib::Object::builder()
            .property("application-id", app_id)
            .property("flags", gio::ApplicationFlags::HANDLES_COMMAND_LINE)
            .build();
        let ui = Ui::new(app.clone().upcast(), service.clone());
        app.imp()
            .service
            .set(service)
            .ok()
            .expect("service set once");
        app.imp().ui.set(ui).ok().expect("ui set once");
        app
    }

    pub fn ui(&self) -> Rc<Ui> {
        self.imp().ui.get().expect("ui").clone()
    }

    pub fn service(&self) -> SharedService {
        self.imp().service.get().expect("service").clone()
    }
}
