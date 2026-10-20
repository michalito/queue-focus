use super::*;
use qf_core::sync::{ConnectionState, Id, RecoveryChange};

impl Ui {
    pub(super) fn sync_settings_card(self: &Rc<Self>, column: &gtk::Box) {
        let enabled = self.service.engine().sync_status().is_some();
        let card = self.settings_card(column, "Sync", Some(if enabled {
            "Now and the timer are shared. Queue edits can be saved offline."
        } else {
            "Not configured. Quit Queue Focus and use qf-sync to preview and join your private server. Each device needs its own enrollment."
        }));
        if !enabled {
            return;
        }
        card.append(&self.sync_label());
        let actions = gtk::Box::builder()
            .spacing(8)
            .margin_start(12)
            .margin_end(12)
            .margin_top(8)
            .margin_bottom(8)
            .build();
        let retry = gtk::Button::with_label("Retry connection");
        let this = Rc::downgrade(self);
        retry.connect_clicked(move |_| {
            if let Some(ui) = this.upgrade() {
                let _ = ui.report(ui.service.retry_sync());
            }
        });
        actions.append(&retry);
        let export = gtk::Button::with_label("Export recovery copy…");
        let this = Rc::downgrade(self);
        export.connect_clicked(move |_| {
            if let Some(ui) = this.upgrade() {
                ui.export_recovery();
            }
        });
        actions.append(&export);
        card.append(&actions);
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(12)
            .build();
        card.append(&list);
        let last: Rc<RefCell<Option<Vec<RecoveryChange>>>> = Rc::new(RefCell::new(None));
        let last_quarantine = Cell::new(false);
        let update = {
            let this = Rc::downgrade(self);
            let list = list.downgrade();
            let retry = retry.downgrade();
            move || {
                let (Some(ui), Some(list), Some(retry)) =
                    (this.upgrade(), list.upgrade(), retry.upgrade())
                else {
                    return;
                };
                let status = ui.service.engine().sync_status().cloned();
                retry.set_sensitive(status.as_ref().is_some_and(|s| {
                    !matches!(
                        s.state,
                        ConnectionState::Recovery | ConnectionState::Durability
                    )
                }));
                let changes = ui.service.engine().sync_recovery();
                let quarantined = status
                    .as_ref()
                    .is_some_and(|s| s.state == ConnectionState::Recovery);
                // Polling updates status often; do not replace focused controls
                // unless the recovery contents actually changed.
                if last.borrow().as_ref() == Some(&changes) && last_quarantine.get() == quarantined
                {
                    return;
                }
                while let Some(child) = list.first_child() {
                    list.remove(&child);
                }
                for item in &changes {
                    ui.recovery_row(&list, item, quarantined);
                }
                *last.borrow_mut() = Some(changes);
                last_quarantine.set(quarantined);
            }
        };
        update();
        self.service.on_status_change(update);
    }

    fn recovery_row(self: &Rc<Self>, list: &gtk::Box, item: &RecoveryChange, quarantined: bool) {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        let text = gtk::Label::builder()
            .label(format!(
                "{}\n{} · {}\nThis change was not applied.",
                item.title, item.attempt, item.reason
            ))
            .wrap(true)
            .xalign(0.0)
            .selectable(true)
            .build();
        row.append(&text);
        let actions = gtk::Box::builder().spacing(8).build();
        let copy = gtk::Button::with_label("Copy change");
        let copy_text = item.copy_text.clone();
        copy.connect_clicked(move |button| button.clipboard().set_text(&copy_text));
        actions.append(&copy);
        let dismiss = gtk::Button::with_label("Dismiss…");
        dismiss.set_sensitive(!quarantined);
        let this = Rc::downgrade(self);
        let command = item.command.clone();
        dismiss.connect_clicked(move |_| {
            let Some(ui) = this.upgrade() else { return; };
            let dialog = adw::AlertDialog::new(Some("Dismiss this rejected change?"),
                Some("This removes the local recovery record. Copy or export it first if you still need it. The shared queue and pending commands stay intact."));
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("dismiss", "Dismiss saved change");
            dialog.set_response_appearance("dismiss", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            let this = Rc::downgrade(&ui);
            let command = command.clone();
            dialog.connect_response(Some("dismiss"), move |_, _| {
                if let Some(ui) = this.upgrade() {
                    let _ = ui.update(|e| e.dismiss_sync_recovery(&command));
                }
            });
            dialog.present(ui.visible_window().as_ref());
        });
        actions.append(&dismiss);
        row.append(&actions);
        list.append(&row);
    }

    fn export_recovery(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Choose where to save a recovery copy")
            .accept_label("Export here")
            .build();
        let this = Rc::downgrade(self);
        dialog.select_folder(self.visible_window().as_ref(), None::<&gio::Cancellable>, move |result| {
            let Some(ui) = this.upgrade() else { return; };
            match result {
                Ok(folder) => {
                    let Some(parent) = folder.path() else {
                        ui.alert("Choose a local folder", "Recovery copies must be saved on the local filesystem.");
                        return;
                    };
                    let destination = parent.join(format!("Queue Focus Recovery {}", Id::new_v4()));
                    match ui.service.export_sync(&destination) {
                        Ok(()) => ui.show_alert("Recovery copy exported", &format!("Saved to {}. The active installation is unchanged. Pending and rejected changes are recorded separately.", destination.display())),
                        Err(error) => ui.alert("Could not export recovery copy", &error.to_string()),
                    }
                }
                Err(error) if error.matches(gtk::DialogError::Dismissed) || error.matches(gtk::DialogError::Cancelled) => {}
                Err(error) => ui.alert("Could not choose a folder", &error.to_string()),
            }
        });
    }
}
