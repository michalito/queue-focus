//! `adw::Application` subclass. The only reason for the subclass: exporting our
//! D-Bus object in `dbus_register`, which GApplication calls *before* it owns the
//! bus name — so a method call that D-Bus-activates the service is never lost.

use crate::dbus;
use crate::flash::{FlashClock, SharedFlash};
use crate::settings::SharedSettings;
use crate::state::SharedState;
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
        pub state: OnceCell<SharedState>,
        pub settings: OnceCell<SharedSettings>,
        pub flash: OnceCell<SharedFlash>,
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
            let (Some(state), Some(settings), Some(flash), Some(ui)) = (
                self.state.get(),
                self.settings.get(),
                self.flash.get(),
                self.ui.get(),
            ) else {
                return Ok(());
            };
            let id = dbus::export(connection, state, settings, flash, ui)?;
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
    pub fn new(app_id: &str, state: SharedState, settings: SharedSettings) -> Self {
        let app: Self = glib::Object::builder()
            .property("application-id", app_id)
            .property("flags", gio::ApplicationFlags::HANDLES_COMMAND_LINE)
            .build();
        let flash = FlashClock::new(state.clone(), settings.clone());
        let ui = Ui::new(
            app.clone().upcast(),
            state.clone(),
            settings.clone(),
            flash.clone(),
        );
        app.imp().state.set(state).ok().expect("state set once");
        app.imp()
            .settings
            .set(settings)
            .ok()
            .expect("settings set once");
        app.imp().flash.set(flash).ok().expect("flash set once");
        app.imp().ui.set(ui).ok().expect("ui set once");
        app
    }

    pub fn ui(&self) -> Rc<Ui> {
        self.imp().ui.get().expect("ui").clone()
    }

    pub fn state(&self) -> SharedState {
        self.imp().state.get().expect("state").clone()
    }

    pub fn settings(&self) -> SharedSettings {
        self.imp().settings.get().expect("settings").clone()
    }
}
