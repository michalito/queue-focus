//! Windows: the main window (Queue / Board pages) and the quick-add popup.
//! Both pages are views over the same store and are rebuilt on every change
//! (task counts are small; rebuilding is simpler and always correct).

use crate::state::SharedState;
use adw::prelude::*;
use gtk::{gdk, gio, glib};
use qf_core::{Bucket, Store, Tag, Task};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const CSS: &str = include_str!("style.css");

const QUEUE_SIZE: (i32, i32) = (400, 640);
const BOARD_SIZE: (i32, i32) = (1040, 640);

/// Bucket order on both pages.
const ORDER: [Bucket; 4] = [Bucket::Now, Bucket::Side, Bucket::Next, Bucket::Later];

const HINTS: &str = "j/k move · J/K reorder · ⏎ focus · d done · 1-4 bucket · t tag · r rename · \
                     l later · n add · b/q view · Esc hide";

pub fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Queue,
    Board,
}

impl Page {
    fn name(self) -> &'static str {
        match self {
            Page::Queue => "queue",
            Page::Board => "board",
        }
    }

    pub fn parse(s: &str) -> Page {
        if s == "board" {
            Page::Board
        } else {
            Page::Queue
        }
    }
}

/// One ListBox showing one bucket on one page.
struct BucketList {
    bucket: Bucket,
    page: Page,
    list: gtk::ListBox,
    header: gtk::Label,
}

pub struct Ui {
    app: adw::Application,
    state: SharedState,
    win: RefCell<Option<adw::ApplicationWindow>>,
    stack: RefCell<Option<adw::ViewStack>>,
    entry: RefCell<Option<gtk::Entry>>,
    lists: RefCell<Vec<BucketList>>,
    /// Rows showing the current task (one per page) and its `started_at`, for the timer.
    current_rows: RefCell<Vec<(adw::ActionRow, u64)>>,
    /// Later section on the queue page: collapsed by default.
    later_expander: RefCell<Option<gtk::Expander>>,
    /// Open rename popover (at most one), detached before any rebuild.
    rename_pop: RefCell<Option<gtk::Popover>>,
    pending_rename: Cell<Option<(u64, String)>>,
    quick: RefCell<Option<(gtk::Window, gtk::Entry)>>,
}

impl Ui {
    pub fn new(app: adw::Application, state: SharedState) -> Rc<Ui> {
        let ui = Rc::new(Ui {
            app,
            state: state.clone(),
            win: RefCell::new(None),
            stack: RefCell::new(None),
            entry: RefCell::new(None),
            lists: RefCell::new(Vec::new()),
            current_rows: RefCell::new(Vec::new()),
            later_expander: RefCell::new(None),
            rename_pop: RefCell::new(None),
            pending_rename: Cell::new(None),
            quick: RefCell::new(None),
        });
        let weak = Rc::downgrade(&ui);
        state.on_change(move || {
            if let Some(ui) = weak.upgrade() {
                ui.rebuild();
            }
        });
        let weak = Rc::downgrade(&ui);
        state.on_durability_warning(move |warning| {
            if let Some(ui) = weak.upgrade() {
                ui.show_durability_warning(&warning.to_string());
            }
        });
        let weak = Rc::downgrade(&ui);
        glib::timeout_add_seconds_local(1, move || {
            if let Some(ui) = weak.upgrade() {
                ui.tick();
            }
            glib::ControlFlow::Continue
        });
        ui
    }

    // ---- public surface ----------------------------------------------

    pub fn show(self: &Rc<Self>, page: Page) {
        let win = self.window();
        self.set_page(page);
        win.present();
    }

    /// Hide if focused, otherwise bring the queue to the front.
    pub fn toggle(self: &Rc<Self>) {
        let focused = self
            .win
            .borrow()
            .as_ref()
            .is_some_and(|w| w.is_visible() && w.is_active());
        if focused {
            self.hide();
        } else {
            self.show(Page::Queue);
        }
    }

    pub fn hide(&self) {
        self.close_rename();
        if let Some(w) = self.win.borrow().as_ref() {
            w.set_visible(false);
        }
        if let Some((w, _)) = self.quick.borrow().as_ref() {
            w.set_visible(false);
        }
    }

    /// Run a persisted mutation. The state listeners make warnings visible;
    /// this path handles failures that prevented the mutation from committing.
    fn update<R>(&self, f: impl FnOnce(&mut Store) -> R) -> Result<R, std::io::Error> {
        self.state
            .update(f)
            .map(|outcome| outcome.into_value())
            .inspect_err(|e| {
                eprintln!("queue-focus: {e}");
                let dialog = adw::AlertDialog::new(
                    Some("Could not safely save task changes"),
                    Some(&e.to_string()),
                );
                dialog.add_response("ok", "OK");
                if let Some(win) = self.win.borrow().as_ref() {
                    dialog.present(Some(win));
                } else if let Some((win, _)) = self.quick.borrow().as_ref() {
                    dialog.present(Some(win));
                }
            })
    }

    fn show_durability_warning(&self, message: &str) {
        let dialog = adw::AlertDialog::new(Some("Task change saved with a warning"), Some(message));
        dialog.add_response("ok", "OK");
        if let Some(win) = self.win.borrow().as_ref() {
            dialog.present(Some(win));
        } else if let Some((win, _)) = self.quick.borrow().as_ref() {
            dialog.present(Some(win));
        } else {
            eprintln!("queue-focus: warning: {message}");
        }
    }

    /// Tiny floating entry for capture from anywhere.
    pub fn quick_add_dialog(self: &Rc<Self>) {
        if let Some((w, e)) = self.quick.borrow().as_ref() {
            w.present();
            e.grab_focus();
            return;
        }
        let entry = gtk::Entry::builder()
            .placeholder_text("Add task…   !now  #w #p  @later @side")
            .hexpand(true)
            .width_chars(48)
            .css_classes(["quick-entry"])
            .build();
        let win = gtk::Window::builder()
            .application(&self.app)
            .title("Add task")
            .resizable(false)
            .child(&entry)
            .css_classes(["quick-add"])
            .build();
        let this = self.clone();
        entry.connect_activate(move |e| {
            if this.submit(e, Bucket::Next) {
                this.hide();
            }
        });
        let this = self.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |c, key, _, mods| {
            let Some(e) = c.widget().and_downcast::<gtk::Entry>() else {
                return glib::Propagation::Proceed;
            };
            match key {
                gdk::Key::Escape => this.hide(),
                gdk::Key::Return | gdk::Key::KP_Enter if is_ctrl(mods) => {
                    if this.submit(&e, Bucket::Now) {
                        this.hide();
                    }
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        entry.add_controller(keys);
        win.connect_close_request(|w| {
            w.set_visible(false);
            glib::Propagation::Stop
        });
        *self.quick.borrow_mut() = Some((win.clone(), entry.clone()));
        win.present();
        entry.grab_focus();
    }

    // ---- window construction ----------------------------------------

    fn window(self: &Rc<Self>) -> adw::ApplicationWindow {
        if let Some(w) = self.win.borrow().as_ref() {
            return w.clone();
        }
        let win = self.build_window();
        *self.win.borrow_mut() = Some(win.clone());
        self.rebuild();
        win
    }

    fn build_window(self: &Rc<Self>) -> adw::ApplicationWindow {
        let win = adw::ApplicationWindow::builder()
            .application(&self.app)
            .title("Queue Focus")
            .default_width(QUEUE_SIZE.0)
            .default_height(QUEUE_SIZE.1)
            .build();

        let stack = adw::ViewStack::new();
        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        let header = adw::HeaderBar::builder().title_widget(&switcher).build();

        // Shared quick-add entry under the header.
        let entry = gtk::Entry::builder()
            .placeholder_text("Add…   !now  #w #p  @later @side   (Ctrl+Enter → Now)")
            .hexpand(true)
            .primary_icon_name("list-add-symbolic")
            .css_classes(["main-entry"])
            .build();
        let entry_bar = gtk::Box::builder().css_classes(["entry-bar"]).build();
        entry_bar.append(&entry);
        let this = self.clone();
        entry.connect_activate(move |e| {
            this.submit(e, Bucket::Next);
        });
        let this = self.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |c, key, _, mods| {
            let Some(e) = c.widget().and_downcast::<gtk::Entry>() else {
                return glib::Propagation::Proceed;
            };
            match key {
                gdk::Key::Return | gdk::Key::KP_Enter if is_ctrl(mods) => {
                    this.submit(&e, Bucket::Now);
                }
                gdk::Key::Escape if !e.text().is_empty() => e.set_text(""),
                gdk::Key::Escape => this.focus_first_row(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        entry.add_controller(keys);

        let queue = stack.add_titled(&self.build_queue_page(), Some(Page::Queue.name()), "Queue");
        queue.set_icon_name(Some("view-list-symbolic"));
        let board = stack.add_titled(&self.build_board_page(), Some(Page::Board.name()), "Board");
        board.set_icon_name(Some("view-grid-symbolic"));

        let hints = gtk::Label::builder()
            .label(HINTS)
            .wrap(true)
            .justify(gtk::Justification::Center)
            .css_classes(["dim-label", "caption", "hints"])
            .build();

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&entry_bar);
        toolbar.set_content(Some(&stack));
        toolbar.add_bottom_bar(&hints);
        win.set_content(Some(&toolbar));

        // The board wants a wider window.
        let w = win.clone();
        stack.connect_visible_child_name_notify(move |s| {
            let (dw, dh) = match s.visible_child_name().as_deref().map(Page::parse) {
                Some(Page::Board) => BOARD_SIZE,
                _ => QUEUE_SIZE,
            };
            w.set_default_size(dw, dh);
        });

        let this = self.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, mods| this.on_key(key, mods));
        win.add_controller(keys);

        self.install_actions(&win);

        let this = self.clone();
        win.connect_close_request(move |_| {
            this.hide();
            glib::Propagation::Stop
        });

        *self.stack.borrow_mut() = Some(stack);
        *self.entry.borrow_mut() = Some(entry);
        win
    }

    /// Add the entry's text (quick-add syntax) to `default` bucket; clears on success.
    fn submit(&self, e: &gtk::Entry, default: Bucket) -> bool {
        let text = e.text();
        let added = matches!(self.update(|s| s.quick_add(&text, default)), Ok(Some(_)));
        if added {
            e.set_text("");
        }
        added
    }

    /// Queue page: one column; Later is collapsible.
    fn build_queue_page(self: &Rc<Self>) -> gtk::Widget {
        let col = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["queue-column"])
            .build();
        for b in ORDER {
            let (header, list) = self.make_list(b, Page::Queue);
            if b == Bucket::Later {
                let expander = gtk::Expander::builder()
                    .label_widget(&header)
                    .child(&list)
                    .expanded(false)
                    .css_classes(["later-expander"])
                    .build();
                col.append(&expander);
                *self.later_expander.borrow_mut() = Some(expander);
            } else {
                col.append(&header);
                col.append(&list);
            }
        }
        gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&col)
            .vexpand(true)
            .build()
            .upcast()
    }

    /// Board page: four columns side by side.
    fn build_board_page(self: &Rc<Self>) -> gtk::Widget {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .homogeneous(true)
            .css_classes(["board"])
            .build();
        for b in ORDER {
            let (header, list) = self.make_list(b, Page::Board);
            let col = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .css_classes(["board-column"])
                .build();
            col.append(&header);
            col.append(
                &gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .child(&list)
                    .vexpand(true)
                    .build(),
            );
            row.append(&col);
        }
        row.upcast()
    }

    fn make_list(self: &Rc<Self>, bucket: Bucket, page: Page) -> (gtk::Label, gtk::ListBox) {
        let header = gtk::Label::builder()
            .label(bucket.label())
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["bucket-header", "heading"])
            .build();
        let placeholder = gtk::Label::builder()
            .label("empty")
            .css_classes(["placeholder", "dim-label"])
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .activate_on_single_click(false) // double-click / Enter = make current
            .valign(gtk::Align::Start)
            .css_classes(["boxed-list", "bucket-list"])
            .build();
        list.set_placeholder(Some(&placeholder));

        let this = self.clone();
        list.connect_row_activated(move |_, row| {
            if let Some(id) = row_id(row) {
                let _ = this.update(|s| s.promote(id));
            }
        });

        // Drop on the list → before the row under the pointer (or at the end);
        // drop on the header → at the end (works for collapsed / empty buckets too).
        let this = self.clone();
        list.add_controller(drop_target(move |id, target, y| {
            let index = target
                .downcast_ref::<gtk::ListBox>()
                .and_then(|l| l.row_at_y(y as i32))
                .map(|r| r.index() as usize);
            this.update(|s| {
                let index = adjusted_drop_index(s, id, bucket, index);
                s.move_to(id, bucket, index)
            })
            .unwrap_or(false)
        }));
        let this = self.clone();
        header.add_controller(drop_target(move |id, _, _| {
            this.update(|s| s.move_to(id, bucket, None))
                .unwrap_or(false)
        }));

        self.lists.borrow_mut().push(BucketList {
            bucket,
            page,
            list: list.clone(),
            header: header.clone(),
        });
        (header, list)
    }

    /// `win.*` actions targeted by row menus/buttons.
    fn install_actions(self: &Rc<Self>, win: &adw::ApplicationWindow) {
        let action = |name: &str, ty: &str, f: Box<dyn Fn(&glib::Variant)>| {
            let a = gio::SimpleAction::new(name, Some(glib::VariantTy::new(ty).unwrap()));
            a.connect_activate(move |_, p| {
                if let Some(p) = p {
                    f(p);
                }
            });
            win.add_action(&a);
        };
        let this = self.clone();
        action(
            "move",
            "(ts)",
            Box::new(move |p| {
                if let Some((id, b)) = p.get::<(u64, String)>() {
                    if let Some(b) = Bucket::parse(&b) {
                        let _ = this.update(|s| s.move_to(id, b, None));
                    }
                }
            }),
        );
        for (name, op) in [
            (
                "remove",
                (|s: &mut qf_core::Store, id| s.remove(id)) as fn(&mut qf_core::Store, u64) -> bool,
            ),
            ("cycle-tag", |s, id| s.cycle_tag(id)),
            ("promote", |s, id| s.promote(id)),
        ] {
            let this = self.clone();
            action(
                name,
                "t",
                Box::new(move |p| {
                    if let Some(id) = p.get::<u64>() {
                        let _ = this.update(|s| op(s, id));
                    }
                }),
            );
        }
        let this = self.clone();
        action(
            "done",
            "t",
            Box::new(move |p| {
                if let Some(id) = p.get::<u64>() {
                    let _ = this.update(|s| complete_task(s, id));
                }
            }),
        );
        let this = self.clone();
        action(
            "rename",
            "t",
            Box::new(move |p| {
                if let Some(row) = p.get::<u64>().and_then(|id| this.row_for(id)) {
                    this.rename_popover(&row);
                }
            }),
        );
    }

    // ---- rebuilding ---------------------------------------------------

    fn rebuild(self: &Rc<Self>) {
        let Some(win) = self.win.borrow().clone() else {
            return;
        };
        self.close_rename();
        let focused = self
            .focused_row()
            .map(|r| (row_id(&r), self.visual_index(&r)));

        let store = self.state.store();
        let current = store.current().map(|t| t.id);
        self.current_rows.borrow_mut().clear();

        for bl in self.lists.borrow().iter() {
            bl.list.remove_all();
            let tasks: Vec<&Task> = store.in_bucket(bl.bucket).collect();
            bl.header.set_label(&match tasks.len() {
                0 => bl.bucket.label().to_string(),
                n => format!("{}  {n}", bl.bucket.label()),
            });
            for t in tasks {
                let is_current = Some(t.id) == current;
                let row = self.build_row(t, is_current, bl.bucket, bl.page == Page::Board);
                bl.list.append(&row);
                if let (true, Some(started)) = (is_current, t.started_at) {
                    self.current_rows.borrow_mut().push((row, started));
                }
            }
        }

        // Window tint follows the current task's tag.
        for tag in [Tag::Work, Tag::Personal] {
            win.remove_css_class(&format!("tag-{}", tag.as_str()));
        }
        if let Some(tag) = store.current().and_then(|t| t.tag) {
            win.add_css_class(&format!("tag-{}", tag.as_str()));
        }
        drop(store);
        self.tick();

        // Keep keyboard focus on the same task, or the same position.
        if let Some((id, idx)) = focused {
            let row = id.and_then(|id| self.row_for(id)).or_else(|| {
                let rows = self.visible_rows();
                rows.get(idx.min(rows.len().saturating_sub(1))).cloned()
            });
            if let Some(r) = row {
                r.grab_focus();
            }
        }
    }

    fn build_row(
        self: &Rc<Self>,
        t: &Task,
        is_current: bool,
        bucket: Bucket,
        compact: bool,
    ) -> adw::ActionRow {
        let id = t.id;
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&t.title).as_str())
            .title_lines(if compact { 3 } else { 2 })
            .activatable(true)
            .name(format!("task-{id}"))
            .css_classes(["task-row"])
            .build();
        if is_current {
            row.add_css_class("current");
        }
        if let Some(tag) = t.tag {
            let chip = gtk::Label::builder()
                .label(match tag {
                    Tag::Work => "W",
                    Tag::Personal => "P",
                })
                .valign(gtk::Align::Center)
                .css_classes(["chip", tag.as_str()])
                .build();
            row.add_prefix(&chip);
        }

        // Icon buttons (mouse). Not focusable so Tab/arrows stay on rows.
        let button = |icon: &str, tip: &str, action: &str, target: glib::Variant| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tip)
                .valign(gtk::Align::Center)
                .focusable(false)
                .css_classes(["flat", "circular", "row-btn"])
                .action_name(action)
                .action_target(&target)
                .build()
        };
        // Menu entries mirror the buttons; on the board (narrow columns) they replace them.
        let menu_item = |label: &str, action: &str, target: glib::Variant| {
            let item = gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(Some(action), Some(&target));
            item
        };
        let menu = gio::Menu::new();
        if compact {
            let sect = gio::Menu::new();
            if !is_current {
                sect.append_item(&menu_item("Make current", "win.promote", id.to_variant()));
            }
            sect.append_item(&menu_item("Cycle tag", "win.cycle-tag", id.to_variant()));
            sect.append_item(&menu_item("Done", "win.done", id.to_variant()));
            menu.append_section(None, &sect);
        }
        let sect = gio::Menu::new();
        for b in ORDER.into_iter().filter(|&b| b != bucket) {
            let target = (id, b.as_str().to_string()).to_variant();
            sect.append_item(&menu_item(
                &format!("Move to {}", b.label()),
                "win.move",
                target,
            ));
        }
        menu.append_section(None, &sect);
        let sect = gio::Menu::new();
        sect.append_item(&menu_item("Rename…", "win.rename", id.to_variant()));
        sect.append_item(&menu_item("Delete", "win.remove", id.to_variant()));
        menu.append_section(None, &sect);

        if !compact {
            if !is_current {
                row.add_suffix(&button(
                    "go-top-symbolic",
                    "Make current (⏎)",
                    "win.promote",
                    id.to_variant(),
                ));
            }
            row.add_suffix(&button(
                "tag-symbolic",
                "Cycle tag (t)",
                "win.cycle-tag",
                id.to_variant(),
            ));
        }
        row.add_suffix(
            &gtk::MenuButton::builder()
                .icon_name("view-more-symbolic")
                .menu_model(&menu)
                .valign(gtk::Align::Center)
                .focusable(false)
                .css_classes(["flat", "circular", "row-btn"])
                .build(),
        );
        if !compact {
            let tip = if is_current {
                "Done — delete and pull the next task (d)"
            } else {
                "Done — delete (d)"
            };
            row.add_suffix(&button(
                "object-select-symbolic",
                tip,
                "win.done",
                id.to_variant(),
            ));
        }

        let drag = gtk::DragSource::builder()
            .actions(gdk::DragAction::MOVE)
            .build();
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&id.to_value())));
        let r = row.clone();
        drag.connect_drag_begin(move |s, _| {
            s.set_icon(Some(&gtk::WidgetPaintable::new(Some(&r))), 0, 0);
        });
        row.add_controller(drag);
        row
    }

    // ---- rename -------------------------------------------------------

    fn rename_popover(self: &Rc<Self>, row: &adw::ActionRow) {
        let Some(id) = row_id(row) else { return };
        self.close_rename();
        let title = self
            .state
            .store()
            .get(id)
            .map(|t| t.title.clone())
            .unwrap_or_default();
        let entry = gtk::Entry::builder().text(&title).width_chars(32).build();
        let pop = gtk::Popover::builder().child(&entry).build();
        pop.set_parent(row);
        let this = self.clone();
        let p = pop.clone();
        entry.connect_activate(move |e| {
            this.pending_rename.set(Some((id, e.text().to_string())));
            p.popdown();
        });
        // `closed` fires for Enter, Escape and click-outside alike. Detach and
        // apply the rename from an idle so the popover is fully done with itself.
        let this = self.clone();
        pop.connect_closed(move |_| {
            let this = this.clone();
            glib::idle_add_local_once(move || {
                this.close_rename();
                if let Some((id, text)) = this.pending_rename.take() {
                    let _ = this.update(|s| s.rename(id, &text));
                }
            });
        });
        *self.rename_pop.borrow_mut() = Some(pop.clone());
        pop.popup();
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    fn close_rename(&self) {
        if let Some(p) = self.rename_pop.borrow_mut().take() {
            p.popdown();
            p.unparent();
        }
    }

    // ---- timer --------------------------------------------------------

    fn tick(&self) {
        let now = qf_core::unix_now();
        for (row, started) in self.current_rows.borrow().iter() {
            row.set_subtitle(&fmt_elapsed(now.saturating_sub(*started)));
        }
    }

    // ---- keyboard -----------------------------------------------------

    fn on_key(self: &Rc<Self>, key: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        if is_ctrl(mods) {
            match key {
                gdk::Key::_1 => self.set_page(Page::Queue),
                gdk::Key::_2 => self.set_page(Page::Board),
                gdk::Key::w | gdk::Key::q => self.hide(),
                _ => return glib::Propagation::Proceed,
            }
            return glib::Propagation::Stop;
        }
        if self.editable_focused() {
            return glib::Propagation::Proceed;
        }
        let row = self.focused_row();
        let id = row.as_ref().and_then(row_id);
        match key {
            gdk::Key::Escape => self.hide(),
            gdk::Key::n | gdk::Key::slash | gdk::Key::a => self.focus_entry(),
            gdk::Key::b => self.set_page(Page::Board),
            gdk::Key::q => self.set_page(Page::Queue),
            gdk::Key::l => self.toggle_later(),
            gdk::Key::j => self.focus_relative(1),
            gdk::Key::k => self.focus_relative(-1),
            gdk::Key::r | gdk::Key::F2 => {
                if let Some(row) = row {
                    self.rename_popover(&row);
                }
            }
            _ => {
                let Some(id) = id else {
                    return glib::Propagation::Proceed;
                };
                let changed = match key {
                    gdk::Key::J => self.update(|s| s.shift(id, 1)),
                    gdk::Key::K => self.update(|s| s.shift(id, -1)),
                    gdk::Key::d | gdk::Key::x | gdk::Key::Delete => {
                        self.update(|s| complete_task(s, id))
                    }
                    gdk::Key::t => self.update(|s| s.cycle_tag(id)),
                    gdk::Key::_1 => self.update(|s| s.promote(id)),
                    gdk::Key::_2 => self.update(|s| s.move_to(id, Bucket::Next, None)),
                    gdk::Key::_3 => self.update(|s| s.move_to(id, Bucket::Later, None)),
                    gdk::Key::_4 => self.update(|s| s.move_to(id, Bucket::Side, None)),
                    _ => return glib::Propagation::Proceed,
                };
                let _ = changed;
            }
        }
        glib::Propagation::Stop
    }

    fn set_page(&self, page: Page) {
        if let Some(stack) = self.stack.borrow().as_ref() {
            stack.set_visible_child_name(page.name());
        }
        self.focus_first_row();
    }

    fn toggle_later(&self) {
        if let Some(x) = self.later_expander.borrow().as_ref() {
            x.set_expanded(!x.is_expanded());
        }
        self.focus_first_row();
    }

    fn focus_entry(&self) {
        if let Some(e) = self.entry.borrow().as_ref() {
            e.grab_focus();
        }
    }

    fn editable_focused(&self) -> bool {
        self.win
            .borrow()
            .as_ref()
            .and_then(GtkWindowExt::focus)
            .is_some_and(|f| {
                f.is::<gtk::Editable>() || f.ancestor(gtk::Entry::static_type()).is_some()
            })
    }

    fn focused_row(&self) -> Option<adw::ActionRow> {
        let f = GtkWindowExt::focus(self.win.borrow().as_ref()?)?;
        f.clone()
            .downcast::<adw::ActionRow>()
            .ok()
            .or_else(|| f.ancestor(adw::ActionRow::static_type()).and_downcast())
    }

    fn current_page(&self) -> Page {
        self.stack
            .borrow()
            .as_ref()
            .and_then(|s| s.visible_child_name())
            .map(|n| Page::parse(&n))
            .unwrap_or(Page::Queue)
    }

    /// Rows the user can see on the visible page, in visual order.
    fn visible_rows(&self) -> Vec<adw::ActionRow> {
        let page = self.current_page();
        let later_open = self
            .later_expander
            .borrow()
            .as_ref()
            .is_some_and(|x| x.is_expanded());
        let mut out = Vec::new();
        for bl in self.lists.borrow().iter().filter(|b| b.page == page) {
            if page == Page::Queue && bl.bucket == Bucket::Later && !later_open {
                continue;
            }
            let mut child = bl.list.first_child();
            while let Some(c) = child {
                if let Some(r) = c.downcast_ref::<adw::ActionRow>() {
                    out.push(r.clone());
                }
                child = c.next_sibling();
            }
        }
        out
    }

    fn visual_index(&self, row: &adw::ActionRow) -> usize {
        self.visible_rows()
            .iter()
            .position(|r| r == row)
            .unwrap_or(0)
    }

    fn row_for(&self, id: u64) -> Option<adw::ActionRow> {
        self.visible_rows()
            .into_iter()
            .find(|r| row_id(r) == Some(id))
    }

    fn focus_first_row(&self) {
        if let Some(r) = self.visible_rows().first() {
            r.grab_focus();
        }
    }

    fn focus_relative(&self, delta: i32) {
        let rows = self.visible_rows();
        if rows.is_empty() {
            return;
        }
        let cur = self
            .focused_row()
            .and_then(|r| rows.iter().position(|x| *x == r));
        let next = match cur {
            Some(i) => (i as i32 + delta).clamp(0, rows.len() as i32 - 1) as usize,
            None => 0,
        };
        rows[next].grab_focus();
    }
}

/// "Done" completes the focused task. Only completing the current task pulls
/// the head of Next into an otherwise-empty Now bucket.
fn complete_task(store: &mut Store, id: u64) -> bool {
    if store.current().is_some_and(|task| task.id == id) {
        store.complete_current().is_some()
    } else {
        store.remove(id)
    }
}

/// ListBox reports the destination before the dragged row has been removed.
/// Account for that row when moving downward within the same bucket.
fn adjusted_drop_index(
    store: &Store,
    id: u64,
    bucket: Bucket,
    index: Option<usize>,
) -> Option<usize> {
    let index = index?;
    let source_index = store
        .get(id)
        .filter(|task| task.bucket == bucket)
        .and_then(|_| store.in_bucket(bucket).position(|task| task.id == id));
    Some(if source_index.is_some_and(|source| source < index) {
        index - 1
    } else {
        index
    })
}

/// Task rows carry their id in the widget name ("task-<id>").
fn row_id(row: &impl IsA<gtk::Widget>) -> Option<u64> {
    row.widget_name().strip_prefix("task-")?.parse().ok()
}

fn is_ctrl(mods: gdk::ModifierType) -> bool {
    mods.contains(gdk::ModifierType::CONTROL_MASK)
}

/// A drop target accepting a dragged task id; `f(id, target_widget, y)` returns
/// whether the drop was handled.
fn drop_target(f: impl Fn(u64, gtk::Widget, f64) -> bool + 'static) -> gtk::DropTarget {
    let target = gtk::DropTarget::new(u64::static_type(), gdk::DragAction::MOVE);
    target.connect_drop(
        move |t, value, _x, y| match (value.get::<u64>(), t.widget()) {
            (Ok(id), Some(w)) => f(id, w, y),
            _ => false,
        },
    );
    target
}

fn fmt_elapsed(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(store: &Store, bucket: Bucket) -> Vec<u64> {
        store.in_bucket(bucket).map(|task| task.id).collect()
    }

    #[test]
    fn elapsed_format() {
        assert_eq!(fmt_elapsed(0), "0:00");
        assert_eq!(fmt_elapsed(65), "1:05");
        assert_eq!(fmt_elapsed(3600), "1:00:00");
        assert_eq!(fmt_elapsed(3725), "1:02:05");
    }

    #[test]
    fn completing_current_from_main_window_pulls_next() {
        let mut store = Store::new();
        let now = store.add("now", Bucket::Now, None, false);
        let next = store.add("next", Bucket::Next, None, false);

        assert!(complete_task(&mut store, now));
        assert_eq!(store.current().map(|task| task.id), Some(next));
    }

    #[test]
    fn downward_drop_uses_post_removal_index() {
        let mut store = Store::new();
        let a = store.add("a", Bucket::Next, None, false);
        let b = store.add("b", Bucket::Next, None, false);
        let c = store.add("c", Bucket::Next, None, false);

        let index = adjusted_drop_index(&store, a, Bucket::Next, Some(2));
        assert!(store.move_to(a, Bucket::Next, index));

        assert_eq!(ids(&store, Bucket::Next), vec![b, a, c]);
    }
}
