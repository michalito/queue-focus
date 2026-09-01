//! Windows: the main window (Queue / Board pages) and the quick-add popup.
//! Both pages are views over the same store and are rebuilt on every change
//! (task counts are small; rebuilding is simpler and always correct).
//!
//! The queue page is three fixed bands: the current task's banner, a scrolling
//! card holding Side and Next, and a Later shelf pinned to the bottom. The head
//! of Now lives in the banner, so the rest of the Now bucket is listed under the
//! Next header — everything queued behind what you are doing.

use crate::state::SharedState;
use adw::prelude::*;
use gtk::{gdk, glib, pango};
use qf_core::{Bucket, Store, Tag, Task};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const CSS: &str = include_str!("style.css");

const QUEUE_SIZE: (i32, i32) = (400, 640);
const BOARD_SIZE: (i32, i32) = (1040, 640);

/// Bucket order on the board page.
const ORDER: [Bucket; 4] = [Bucket::Now, Bucket::Side, Bucket::Next, Bucket::Later];

/// The `?` popover, in order.
const SHORTCUTS: [(&str, &str); 11] = [
    ("j/k", "move"),
    ("J/K", "reorder"),
    ("⏎", "focus"),
    ("d", "done"),
    ("p", "pause"),
    ("1-4", "bucket"),
    ("t", "tag"),
    ("r", "rename"),
    ("l", "later"),
    ("n", "add"),
    ("b/q", "view"),
];

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

/// How much furniture a row carries. The board's columns are too narrow for
/// inline buttons, and Later — cold storage — gets a "→ next" shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowStyle {
    Queue,
    Later,
    Board,
}

/// One ListBox showing (part of) one bucket on one page.
struct BucketList {
    bucket: Bucket,
    page: Page,
    style: RowStyle,
    /// Queue page: the head of Now is the banner, so this list starts after it.
    skip_current: bool,
    list: gtk::ListBox,
}

/// A titled section: the count beside its header, and the "empty" placeholder
/// shown while every list under that header is empty.
struct Section {
    count: gtk::Label,
    placeholder: gtk::Label,
    lists: Vec<gtk::ListBox>,
}

pub struct Ui {
    app: adw::Application,
    state: SharedState,
    win: RefCell<Option<adw::ApplicationWindow>>,
    stack: RefCell<Option<adw::ViewStack>>,
    entry: RefCell<Option<gtk::Entry>>,
    lists: RefCell<Vec<BucketList>>,
    sections: RefCell<Vec<Section>>,
    /// Queue page: the current task's band, rebuilt with everything else.
    banner: RefCell<Option<gtk::Box>>,
    /// Every label showing the current task's elapsed time.
    timers: RefCell<Vec<gtk::Label>>,
    /// Later shelf: collapsed by default.
    later: RefCell<Option<(gtk::Revealer, gtk::Image)>>,
    shortcuts: RefCell<Option<gtk::MenuButton>>,
    /// The task being renamed in place, and the text typed so far — kept out of
    /// the widget so a rebuild triggered by another client does not lose it.
    renaming: Cell<Option<u64>>,
    rename_text: RefCell<String>,
    rename_entry: RefCell<Option<gtk::Entry>>,
    /// Set while a rename has only just started, so the old title is selected
    /// once rather than on every rebuild.
    rename_fresh: Cell<bool>,
    /// Set while rebuild() tears rows down, so losing focus does not recurse;
    /// `dirty` records a change that arrived while it was set.
    rebuilding: Cell<bool>,
    dirty: Cell<bool>,
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
            sections: RefCell::new(Vec::new()),
            banner: RefCell::new(None),
            timers: RefCell::new(Vec::new()),
            later: RefCell::new(None),
            shortcuts: RefCell::new(None),
            renaming: Cell::new(None),
            rename_text: RefCell::new(String::new()),
            rename_entry: RefCell::new(None),
            rename_fresh: Cell::new(false),
            rebuilding: Cell::new(false),
            dirty: Cell::new(false),
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

    pub fn hide(self: &Rc<Self>) {
        self.cancel_rename();
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
            .placeholder_text("Add…  !now  #w #p  @later @side  (⏎)")
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

        // Size to the visible page, not to the widest one: the board must not
        // hold the queue's 400px window open.
        let stack = adw::ViewStack::builder()
            .hhomogeneous(false)
            .vhomogeneous(false)
            .build();
        let header = adw::HeaderBar::builder()
            .title_widget(&self.build_view_switcher(&stack))
            .decoration_layout(":close")
            .build();
        header.pack_end(&self.build_shortcuts_button());

        // Shared quick-add entry under the header.
        let entry = gtk::Entry::builder()
            .placeholder_text("Add…  !now  #w #p  @later @side  (⏎)")
            .hexpand(true)
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

        stack.add_titled(&self.build_queue_page(), Some(Page::Queue.name()), "Queue");
        stack.add_titled(&self.build_board_page(), Some(Page::Board.name()), "Board");

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&entry_bar);
        toolbar.set_content(Some(&stack));
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

    /// Queue / Board as a segmented control. AdwViewSwitcher would insist on
    /// an icon per page; the design wants the two words and nothing else.
    fn build_view_switcher(self: &Rc<Self>, stack: &adw::ViewStack) -> gtk::Box {
        let group = gtk::Box::builder().css_classes(["view-switch"]).build();
        let queue = gtk::ToggleButton::builder()
            .label("Queue")
            .tooltip_text("Queue (q)")
            .active(true)
            .build();
        let board = gtk::ToggleButton::builder()
            .label("Board")
            .tooltip_text("Board (b)")
            .group(&queue)
            .build();
        clickable(&queue);
        clickable(&board);
        group.append(&queue);
        group.append(&board);

        for (button, page) in [(&queue, Page::Queue), (&board, Page::Board)] {
            let stack = stack.clone();
            button.connect_toggled(move |b| {
                if b.is_active() {
                    stack.set_visible_child_name(page.name());
                }
            });
        }
        let (q, b) = (queue.clone(), board.clone());
        stack.connect_visible_child_name_notify(move |s| {
            match s.visible_child_name().as_deref().map(Page::parse) {
                Some(Page::Board) => b.set_active(true),
                _ => q.set_active(true),
            }
        });
        group
    }

    /// The cheat sheet behind the header bar's `?`.
    fn build_shortcuts_button(self: &Rc<Self>) -> gtk::MenuButton {
        let grid = gtk::Grid::builder()
            .row_spacing(5)
            .column_spacing(12)
            .css_classes(["shortcuts"])
            .build();
        for (i, (key, what)) in SHORTCUTS.iter().enumerate() {
            let key = gtk::Label::builder()
                .label(*key)
                .xalign(0.0)
                .css_classes(["key", "monospace"])
                .build();
            let what = gtk::Label::builder().label(*what).xalign(0.0).build();
            grid.attach(&key, 0, i as i32, 1, 1);
            grid.attach(&what, 1, i as i32, 1, 1);
        }
        let popover = gtk::Popover::builder()
            .child(&grid)
            .has_arrow(false)
            .build();
        // Added, not set: the builder would drop GTK's own `background` class,
        // which resets the font inherited from the button we hang off.
        popover.add_css_class("shortcuts-pop");
        // A child rather than `label`, which drags a dropdown arrow along.
        let button = gtk::MenuButton::builder()
            .child(&gtk::Label::new(Some("?")))
            .tooltip_text("Keyboard shortcuts")
            .popover(&popover)
            .valign(gtk::Align::Center)
            .css_classes(["flat", "hint-btn"])
            .build();
        clickable(&button);
        *self.shortcuts.borrow_mut() = Some(button.clone());
        button
    }

    /// Queue page: the current task, then one card of Side + Next, then Later.
    fn build_queue_page(self: &Rc<Self>) -> gtk::Widget {
        let page = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();

        let banner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["current-banner"])
            .build();
        // The head of Now is not in any list, so dropping on the banner is the
        // only way to drag a task to the front of the queue.
        let this = self.clone();
        banner.add_controller(drop_target(move |id, _, _| {
            this.update(|s| s.promote(id)).unwrap_or(false)
        }));
        page.append(&banner);
        *self.banner.borrow_mut() = Some(banner);

        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Start)
            .css_classes(["queue-card"])
            .build();
        self.add_queue_section(&card, Bucket::Side, false);
        self.add_queue_section(&card, Bucket::Next, true);

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["queue-column"])
            .build();
        column.append(&card);
        page.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&column)
                .vexpand(true)
                .build(),
        );

        page.append(&self.build_later_shelf());
        page.upcast()
    }

    fn add_queue_section(self: &Rc<Self>, card: &gtk::Box, bucket: Bucket, divided: bool) {
        let (head_box, count) = section_header(bucket, divided);
        self.header_drop(&head_box, bucket);
        card.append(&head_box);

        let placeholder = placeholder_label();
        // The list is zero-height while empty, so the placeholder has to accept
        // the drop that would otherwise have landed on the list.
        self.header_drop(&placeholder, bucket);
        card.append(&placeholder);

        let mut lists = Vec::new();
        // Next also shows the tail of Now: what is queued behind the banner.
        if bucket == Bucket::Next {
            let tail = self.make_list(Bucket::Now, Page::Queue, RowStyle::Queue, true);
            card.append(&tail);
            lists.push(tail);
        }
        let list = self.make_list(bucket, Page::Queue, RowStyle::Queue, false);
        card.append(&list);
        lists.push(list);

        self.sections.borrow_mut().push(Section {
            count,
            placeholder,
            lists,
        });
    }

    /// Later hangs below the scroll area so it never pushes the queue around.
    fn build_later_shelf(self: &Rc<Self>) -> gtk::Widget {
        let shelf = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["later-footer"])
            .build();

        let label = gtk::Label::builder()
            .label(Bucket::Later.label())
            .css_classes(["bucket-header"])
            .build();
        let count = gtk::Label::builder()
            .css_classes(["section-count"])
            .hexpand(true)
            .xalign(0.0)
            .build();
        let caret = gtk::Image::from_icon_name("pan-end-symbolic");
        caret.add_css_class("later-caret");
        let head_box = gtk::Box::builder().spacing(8).build();
        head_box.append(&label);
        head_box.append(&count);
        head_box.append(&caret);
        let toggle = gtk::Button::builder()
            .child(&head_box)
            .css_classes(["flat", "later-toggle"])
            .build();
        clickable(&toggle);
        let this = self.clone();
        toggle.connect_clicked(move |_| this.toggle_later());
        self.header_drop(&toggle, Bucket::Later);
        shelf.append(&toggle);

        let placeholder = placeholder_label();
        self.header_drop(&placeholder, Bucket::Later);
        let list = self.make_list(Bucket::Later, Page::Queue, RowStyle::Later, false);
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["later-list"])
            .build();
        body.append(&placeholder);
        body.append(&list);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(150)
            .child(
                &gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .propagate_natural_height(true)
                    .max_content_height(180)
                    .child(&body)
                    .build(),
            )
            .build();
        shelf.append(&revealer);

        *self.later.borrow_mut() = Some((revealer, caret));
        self.sections.borrow_mut().push(Section {
            count,
            placeholder,
            lists: vec![list],
        });
        shelf.upcast()
    }

    /// Board page: four columns side by side.
    fn build_board_page(self: &Rc<Self>) -> gtk::Widget {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .homogeneous(true)
            .css_classes(["board"])
            .build();
        for b in ORDER {
            let (head_box, count) = section_header(b, false);
            self.header_drop(&head_box, b);
            let placeholder = placeholder_label();
            self.header_drop(&placeholder, b);
            let list = self.make_list(b, Page::Board, RowStyle::Board, false);

            let col = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .css_classes(["board-column"])
                .build();
            col.append(&head_box);
            col.append(&placeholder);
            col.append(
                &gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .child(&list)
                    .vexpand(true)
                    .build(),
            );
            row.append(&col);
            self.sections.borrow_mut().push(Section {
                count,
                placeholder,
                lists: vec![list],
            });
        }
        row.upcast()
    }

    fn make_list(
        self: &Rc<Self>,
        bucket: Bucket,
        page: Page,
        style: RowStyle,
        skip_current: bool,
    ) -> gtk::ListBox {
        let classes: Vec<&str> = match style {
            RowStyle::Board => vec!["boxed-list", "bucket-list"],
            _ => vec!["bucket-list"],
        };
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .activate_on_single_click(false) // double-click / Enter = make current
            .valign(gtk::Align::Start)
            .css_classes(classes)
            .build();

        let this = self.clone();
        list.connect_row_activated(move |_, row| {
            if let Some(id) = row_id(row) {
                let _ = this.update(|s| s.promote(id));
            }
        });

        // Drop on the list → before the row under the pointer (or at the end).
        let this = self.clone();
        let head_offset = usize::from(skip_current);
        list.add_controller(drop_target(move |id, target, y| {
            let index = target
                .downcast_ref::<gtk::ListBox>()
                .and_then(|l| l.row_at_y(y as i32))
                .map(|r| r.index() as usize + head_offset);
            this.update(|s| {
                let index = adjusted_drop_index(s, id, bucket, index);
                s.move_to(id, bucket, index)
            })
            .unwrap_or(false)
        }));

        self.lists.borrow_mut().push(BucketList {
            bucket,
            page,
            style,
            skip_current,
            list: list.clone(),
        });
        list
    }

    /// Dropping on a bucket's header appends to it — this is the only way into
    /// a collapsed or empty bucket.
    fn header_drop(self: &Rc<Self>, header: &impl IsA<gtk::Widget>, bucket: Bucket) {
        let this = self.clone();
        header.add_controller(drop_target(move |id, _, _| {
            this.update(|s| s.move_to(id, bucket, None))
                .unwrap_or(false)
        }));
    }

    // ---- rebuilding ---------------------------------------------------

    fn rebuild(self: &Rc<Self>) {
        let Some(win) = self.win.borrow().clone() else {
            return;
        };
        if self.rebuilding.get() {
            // A mutation landed mid-rebuild; catch up once this one unwinds.
            self.dirty.set(true);
            return;
        }
        self.rebuilding.set(true);
        self.dirty.set(false);

        let focused = self
            .focused_row()
            .map(|r| (row_id(&r), self.visual_index(&r)));

        let store = self.state.store();
        // A rename outlives neither its task nor a mutation that removes it.
        if self
            .renaming
            .get()
            .is_some_and(|id| store.get(id).is_none())
        {
            self.renaming.set(None);
        }
        let current = store.current();
        let current_id = current.map(|t| t.id);
        self.timers.borrow_mut().clear();
        *self.rename_entry.borrow_mut() = None;

        self.build_banner(current);

        for bl in self.lists.borrow().iter() {
            bl.list.remove_all();
            let tasks = store
                .in_bucket(bl.bucket)
                .skip(usize::from(bl.skip_current))
                .collect::<Vec<&Task>>();
            for t in tasks {
                let row = self.build_row(t, Some(t.id) == current_id, bl.bucket, bl.style, bl.page);
                bl.list.append(&row);
            }
        }
        for section in self.sections.borrow().iter() {
            // Composite sections (currently Queue's Next) contain more than
            // one bucket list, so count the rows the section actually shows.
            let n = section
                .lists
                .iter()
                .map(|list| {
                    let mut n = 0;
                    let mut child = list.first_child();
                    while let Some(row) = child {
                        n += 1;
                        child = row.next_sibling();
                    }
                    n
                })
                .sum::<usize>();
            section.count.set_label(&n.to_string());
            section
                .placeholder
                .set_visible(section.lists.iter().all(|l| l.first_child().is_none()));
        }

        // Window tint follows the current task's tag.
        for tag in [Tag::Work, Tag::Personal] {
            win.remove_css_class(&format!("tag-{}", tag.as_str()));
        }
        if let Some(tag) = current.and_then(|t| t.tag) {
            win.add_css_class(&format!("tag-{}", tag.as_str()));
        }
        drop(store);
        self.tick();
        self.rebuilding.set(false);
        if self.dirty.replace(false) {
            let this = self.clone();
            glib::idle_add_local_once(move || this.rebuild());
            return;
        }

        if let Some(entry) = self.rename_entry.borrow().clone() {
            let fresh = self.rename_fresh.replace(false);
            glib::idle_add_local_once(move || {
                entry.grab_focus();
                if fresh {
                    entry.select_region(0, -1);
                } else {
                    entry.set_position(-1);
                }
            });
            return;
        }

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

    /// The one task you're doing right now: the loudest thing on the page.
    fn build_banner(self: &Rc<Self>, current: Option<&Task>) {
        let Some(banner) = self.banner.borrow().clone() else {
            return;
        };
        while let Some(child) = banner.first_child() {
            banner.remove(&child);
        }

        let Some(task) = current else {
            banner.add_css_class("empty");
            // Not a task, so not a stop for the keyboard either.
            banner.set_widget_name("banner");
            banner.set_focusable(false);
            banner.append(
                &gtk::Label::builder()
                    .label(Bucket::Now.label())
                    .xalign(0.0)
                    .css_classes(["bucket-header"])
                    .build(),
            );
            let empty = placeholder_label();
            empty.set_label("empty — promote one ↑");
            banner.append(&empty);
            return;
        };
        banner.remove_css_class("empty");
        let id = task.id;
        let paused = task.is_paused();
        // The banner stands in for the current task's row: j/k reach it, and
        // `row_id` finds it, so d/t/r act on it like any other task.
        banner.set_widget_name(&format!("task-{id}"));
        banner.set_focusable(true);
        banner.update_property(&[gtk::accessible::Property::Label(&task.title)]);

        let top = gtk::Box::builder().spacing(8).build();
        top.append(
            &gtk::Label::builder()
                .label(Bucket::Now.label())
                .css_classes(["now-label"])
                .build(),
        );
        if let Some(tag) = task.tag {
            let this = self.clone();
            let button = gtk::Button::builder()
                .child(&chip_label(tag))
                .tooltip_text("Cycle tag (t)")
                .valign(gtk::Align::Center)
                .focusable(false)
                .css_classes(["flat", "chip-btn"])
                .build();
            clickable(&button);
            button.connect_clicked(move |_| {
                let _ = this.update(|s| s.cycle_tag(id));
            });
            top.append(&button);
        }

        // Timer. The pause glyph keeps its space so the clock never shifts.
        let timer = gtk::Label::builder()
            .css_classes(["timer", "monospace", "numeric"])
            .build();
        let glyph = gtk::Image::from_icon_name(if paused {
            "media-playback-start-symbolic"
        } else {
            "media-playback-pause-symbolic"
        });
        glyph.add_css_class("pause-glyph");
        let clock = gtk::Box::builder().spacing(6).build();
        clock.append(&glyph);
        clock.append(&timer);
        let mut classes = vec!["flat", "timer-btn"];
        if paused {
            classes.push("paused");
        }
        let timer_button = gtk::Button::builder()
            .child(&clock)
            .tooltip_text(if paused { "Resume (p)" } else { "Pause (p)" })
            .halign(gtk::Align::End)
            .hexpand(true)
            .focusable(false)
            .css_classes(classes)
            .build();
        clickable(&timer_button);
        let this = self.clone();
        timer_button.connect_clicked(move |_| {
            let _ = this.update(|s| s.toggle_pause());
        });
        top.append(&timer_button);
        self.timers.borrow_mut().push(timer);
        banner.append(&top);

        let title_row = gtk::Box::builder().spacing(8).build();
        if self.renaming.get() == Some(id) && self.current_page() == Page::Queue {
            let entry = self.rename_entry(id);
            // Keep the title's weight while editing so the banner does not jump.
            entry.add_css_class("rename-title");
            title_row.append(&entry);
        } else {
            title_row.append(
                &gtk::Label::builder()
                    .label(&task.title)
                    .xalign(0.0)
                    .hexpand(true)
                    .wrap(true)
                    // Break inside words: a bare URL must not widen the window.
                    .wrap_mode(pango::WrapMode::WordChar)
                    .lines(3)
                    .ellipsize(pango::EllipsizeMode::End)
                    .css_classes(["current-title"])
                    .build(),
            );
        }
        let this = self.clone();
        title_row.append(&icon_button(
            "object-select-symbolic",
            "Done — delete and pull the next task (d)",
            &["flat", "banner-btn"],
            move || {
                let _ = this.update(|s| complete_task(s, id));
            },
        ));
        title_row.append(&self.task_menu(id, true, Bucket::Now, &["flat", "banner-btn"]));
        banner.append(&title_row);
    }

    fn build_row(
        self: &Rc<Self>,
        task: &Task,
        is_current: bool,
        bucket: Bucket,
        style: RowStyle,
        page: Page,
    ) -> gtk::ListBoxRow {
        let id = task.id;
        // Both pages hold a row per task, but only the visible one may host the
        // editor — otherwise rebuild() focuses a widget nobody can see.
        let renaming = self.renaming.get() == Some(id) && page == self.current_page();
        let row = gtk::ListBoxRow::builder()
            // Double-clicking inside the rename entry must not promote the row.
            .activatable(!renaming)
            .name(format!("task-{id}"))
            .css_classes(["task-row"])
            .build();
        row.update_property(&[gtk::accessible::Property::Label(&task.title)]);
        if is_current {
            row.add_css_class("current");
        }
        let content = gtk::Box::builder()
            .spacing(8)
            .css_classes(["row-box"])
            .build();
        row.set_child(Some(&content));

        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        if renaming {
            text.append(&self.rename_entry(id));
        } else {
            let title = gtk::Label::builder()
                .label(&task.title)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(pango::EllipsizeMode::End)
                .css_classes(["row-title"])
                .build();
            if style == RowStyle::Board {
                // Break inside words too, so one long URL cannot set the
                // column's minimum width.
                title.set_wrap(true);
                title.set_wrap_mode(pango::WrapMode::WordChar);
                title.set_lines(3);
            }
            text.append(&title);
        }
        // Only the board still shows the current task as a row, so it carries
        // the clock there.
        if is_current {
            let timer = gtk::Label::builder()
                .xalign(0.0)
                .css_classes(["timer", "monospace", "numeric"])
                .build();
            text.append(&timer);
            self.timers.borrow_mut().push(timer);
        }
        content.append(&text);

        if let Some(tag) = task.tag {
            content.append(&chip_label(tag));
        }
        if style != RowStyle::Board && !is_current {
            let this = self.clone();
            content.append(&icon_button(
                "go-top-symbolic",
                "Make current (⏎)",
                &["flat", "row-btn"],
                move || {
                    let _ = this.update(|s| s.promote(id));
                },
            ));
        }
        if style == RowStyle::Later {
            let this = self.clone();
            let to_next = gtk::Button::builder()
                .label("→ next")
                .tooltip_text("Move to Next")
                .valign(gtk::Align::Center)
                .focusable(false)
                .css_classes(["flat", "to-next-btn"])
                .build();
            clickable(&to_next);
            to_next.connect_clicked(move |_| {
                let _ = this.update(|s| s.move_to(id, Bucket::Next, None));
            });
            content.append(&to_next);
        }
        content.append(&self.task_menu(id, is_current, bucket, &["flat", "row-btn"]));

        let drag = gtk::DragSource::builder()
            .actions(gdk::DragAction::MOVE)
            .build();
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&id.to_value())));
        // Ask the controller for its widget rather than capturing the row: a
        // captured row would own the closure that owns the row.
        drag.connect_drag_begin(|s, _| {
            if let Some(w) = s.widget() {
                s.set_icon(Some(&gtk::WidgetPaintable::new(Some(&w))), 0, 0);
            }
        });
        row.add_controller(drag);
        row
    }

    /// The ⋮ menu. Mirrors every keyboard action so the mouse never loses out.
    fn task_menu(
        self: &Rc<Self>,
        id: u64,
        is_current: bool,
        bucket: Bucket,
        classes: &[&str],
    ) -> gtk::MenuButton {
        let popover = gtk::Popover::builder()
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .build();
        popover.add_css_class("task-menu");
        let items = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();

        if !is_current {
            let this = self.clone();
            items.append(&menu_item(
                &popover,
                "Make current",
                Some("⏎"),
                false,
                move || {
                    let _ = this.update(|s| s.promote(id));
                },
            ));
        }
        let this = self.clone();
        items.append(&menu_item(
            &popover,
            "Cycle tag",
            Some("t"),
            false,
            move || {
                let _ = this.update(|s| s.cycle_tag(id));
            },
        ));
        let this = self.clone();
        items.append(&menu_item(&popover, "Done", Some("d"), false, move || {
            let _ = this.update(|s| complete_task(s, id));
        }));

        items.append(&menu_separator());
        for b in ORDER.into_iter().filter(|&b| b != bucket) {
            let this = self.clone();
            items.append(&menu_item(
                &popover,
                &format!("Move to {}", b.label()),
                None,
                false,
                move || {
                    let _ = this.update(|s| s.move_to(id, b, None));
                },
            ));
        }

        items.append(&menu_separator());
        let this = self.clone();
        items.append(&menu_item(
            &popover,
            "Rename…",
            Some("r"),
            false,
            move || {
                this.begin_rename(id);
            },
        ));
        let this = self.clone();
        items.append(&menu_item(&popover, "Delete", None, true, move || {
            let _ = this.update(|s| s.remove(id));
        }));

        popover.set_child(Some(&items));
        let menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text("Menu")
            .popover(&popover)
            .valign(gtk::Align::Center)
            .focusable(false)
            .css_classes(classes.to_vec())
            .build();
        clickable(&menu);
        menu
    }

    // ---- rename in place ----------------------------------------------

    fn begin_rename(self: &Rc<Self>, id: u64) {
        let Some((title, bucket)) = self
            .state
            .store()
            .get(id)
            .map(|t| (t.title.clone(), t.bucket))
        else {
            return;
        };
        // Never put an editor somewhere the user cannot see it.
        if bucket == Bucket::Later && self.current_page() == Page::Queue {
            self.set_later_open(true);
        }
        *self.rename_text.borrow_mut() = title;
        self.renaming.set(Some(id));
        self.rename_fresh.set(true);
        self.rebuild();
    }

    /// An entry standing in for a title. Enter commits, Escape and losing focus
    /// abandon; both are deferred so the widget is done with itself first.
    fn rename_entry(self: &Rc<Self>, id: u64) -> gtk::Entry {
        let entry = gtk::Entry::builder()
            .text(self.rename_text.borrow().as_str())
            .hexpand(true)
            .css_classes(["rename-entry"])
            .build();

        let this = self.clone();
        entry.connect_changed(move |e| {
            *this.rename_text.borrow_mut() = e.text().to_string();
        });
        let this = self.clone();
        entry.connect_activate(move |e| {
            let text = e.text().to_string();
            let this = this.clone();
            glib::idle_add_local_once(move || {
                if this.renaming.replace(None) != Some(id) {
                    return;
                }
                // A failed save never notifies, so put the title back by hand.
                if this.update(|s| s.rename(id, &text)).is_err() {
                    this.rebuild();
                }
            });
        });
        let this = self.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| match key {
            gdk::Key::Escape => {
                this.cancel_rename();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        entry.add_controller(keys);
        let this = self.clone();
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(move |_| {
            // Focus also leaves when the whole window is deactivated. Switching
            // away from Queue Focus must not throw the edit away.
            let window_active = this
                .win
                .borrow()
                .as_ref()
                .is_some_and(|w| w.is_active() && w.is_visible());
            if window_active {
                this.cancel_rename();
            }
        });
        entry.add_controller(focus);

        *self.rename_entry.borrow_mut() = Some(entry.clone());
        entry
    }

    fn cancel_rename(self: &Rc<Self>) {
        if self.rebuilding.get() || self.renaming.replace(None).is_none() {
            return;
        }
        let this = self.clone();
        glib::idle_add_local_once(move || this.rebuild());
    }

    // ---- timer --------------------------------------------------------

    fn tick(&self) {
        let timers = self.timers.borrow();
        if timers.is_empty() {
            return;
        }
        let now = qf_core::unix_now();
        let elapsed = self
            .state
            .store()
            .current()
            .and_then(|t| t.elapsed_secs(now))
            .map(fmt_elapsed)
            .unwrap_or_default();
        for label in timers.iter() {
            label.set_label(&elapsed);
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
        // With nothing focused the row keys act on the current task — the one
        // the page is built around.
        let id = self
            .focused_row()
            .as_ref()
            .and_then(row_id)
            .or_else(|| self.current_id());
        match key {
            gdk::Key::Escape => self.hide(),
            gdk::Key::n | gdk::Key::slash | gdk::Key::a => self.focus_entry(),
            gdk::Key::b => self.set_page(Page::Board),
            gdk::Key::q => self.set_page(Page::Queue),
            gdk::Key::l => self.toggle_later(),
            gdk::Key::j => self.focus_relative(1),
            gdk::Key::k => self.focus_relative(-1),
            gdk::Key::p => {
                let _ = self.update(|s| s.toggle_pause());
            }
            gdk::Key::question => self.toggle_shortcuts(),
            gdk::Key::r | gdk::Key::F2 => {
                if let Some(id) = id {
                    self.begin_rename(id);
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

    fn set_page(self: &Rc<Self>, page: Page) {
        // The editor belongs to the page it was opened on.
        self.cancel_rename();
        if let Some(stack) = self.stack.borrow().as_ref() {
            stack.set_visible_child_name(page.name());
        }
        self.focus_first_row();
    }

    fn toggle_later(&self) {
        let open = self
            .later
            .borrow()
            .as_ref()
            .is_some_and(|(r, _)| r.reveals_child());
        self.set_later_open(!open);
        self.focus_first_row();
    }

    fn set_later_open(&self, open: bool) {
        // Clone out before touching GTK: set_reveal_child notifies synchronously.
        let Some((revealer, caret)) = self.later.borrow().clone() else {
            return;
        };
        revealer.set_reveal_child(open);
        caret.set_icon_name(Some(if open {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        }));
    }

    fn toggle_shortcuts(&self) {
        if let Some(button) = self.shortcuts.borrow().as_ref() {
            match button.popover() {
                Some(p) if p.is_visible() => button.popdown(),
                _ => button.popup(),
            }
        }
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

    /// The task the keyboard is on: the nearest ancestor carrying a task id.
    /// That is a row in a list, or the banner — which is not a row at all.
    fn focused_row(&self) -> Option<gtk::Widget> {
        let win = self.win.borrow().clone()?;
        let mut widget = GtkWindowExt::focus(&win);
        while let Some(w) = widget {
            if row_id(&w).is_some() {
                return Some(w);
            }
            widget = w.parent();
        }
        None
    }

    fn current_id(&self) -> Option<u64> {
        self.state.store().current().map(|t| t.id)
    }

    fn current_page(&self) -> Page {
        self.stack
            .borrow()
            .as_ref()
            .and_then(|s| s.visible_child_name())
            .map(|n| Page::parse(&n))
            .unwrap_or(Page::Queue)
    }

    /// Tasks the user can see on the visible page, in visual order. On the
    /// queue page the banner leads: it is the current task's "row".
    fn visible_rows(&self) -> Vec<gtk::Widget> {
        let page = self.current_page();
        let later_open = self
            .later
            .borrow()
            .as_ref()
            .is_some_and(|(r, _)| r.reveals_child());
        let mut out = Vec::new();
        if page == Page::Queue {
            let banner = self.banner.borrow().clone();
            out.extend(banner.filter(|b| row_id(b).is_some()).map(|b| b.upcast()));
        }
        for bl in self.lists.borrow().iter().filter(|b| b.page == page) {
            if bl.style == RowStyle::Later && !later_open {
                continue;
            }
            let mut child = bl.list.first_child();
            while let Some(c) = child {
                if c.is::<gtk::ListBoxRow>() {
                    out.push(c.clone());
                }
                child = c.next_sibling();
            }
        }
        out
    }

    fn visual_index(&self, row: &gtk::Widget) -> usize {
        self.visible_rows()
            .iter()
            .position(|r| r == row)
            .unwrap_or(0)
    }

    fn row_for(&self, id: u64) -> Option<gtk::Widget> {
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

/// A bucket's header: uppercase label plus its count.
fn section_header(bucket: Bucket, divided: bool) -> (gtk::Box, gtk::Label) {
    let label = gtk::Label::builder()
        .label(bucket.label())
        .css_classes(["bucket-header"])
        .build();
    let count = gtk::Label::builder()
        .css_classes(["section-count"])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let classes: Vec<&str> = if divided {
        vec!["section-header", "divided"]
    } else {
        vec!["section-header"]
    };
    let head_box = gtk::Box::builder()
        .spacing(6)
        .css_classes(classes)
        .baseline_position(gtk::BaselinePosition::Center)
        .build();
    head_box.append(&label);
    head_box.append(&count);
    (head_box, count)
}

fn placeholder_label() -> gtk::Label {
    gtk::Label::builder()
        .label("empty")
        .xalign(0.0)
        .css_classes(["placeholder"])
        .build()
}

fn chip_label(tag: Tag) -> gtk::Label {
    gtk::Label::builder()
        .label(match tag {
            Tag::Work => "W",
            Tag::Personal => "P",
        })
        .valign(gtk::Align::Center)
        .css_classes(["chip", tag.as_str()])
        .build()
}

/// Everything clickable gets the pointer cursor the design asks for. GTK CSS
/// has no `cursor` property, so it is a per-widget call.
fn clickable(widget: &impl IsA<gtk::Widget>) {
    widget.set_cursor_from_name(Some("pointer"));
}

/// A square icon button that stays out of the keyboard's way: Tab and the arrow
/// keys keep moving between rows.
fn icon_button(icon: &str, tip: &str, classes: &[&str], f: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tip)
        .valign(gtk::Align::Center)
        .focusable(false)
        .css_classes(classes.to_vec())
        .build();
    clickable(&button);
    button.connect_clicked(move |_| f());
    button
}

/// One line of a hand-built menu: a flat button with an optional key hint.
fn menu_item(
    popover: &gtk::Popover,
    label: &str,
    accel: Option<&str>,
    destructive: bool,
    f: impl Fn() + 'static,
) -> gtk::Button {
    let content = gtk::Box::builder().spacing(12).build();
    content.append(
        &gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .hexpand(true)
            .build(),
    );
    if let Some(accel) = accel {
        content.append(
            &gtk::Label::builder()
                .label(accel)
                .css_classes(["menu-accel", "monospace"])
                .build(),
        );
    }
    let mut classes = vec!["flat", "menu-item"];
    if destructive {
        classes.push("destructive");
    }
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(classes)
        .build();
    // With both a label and a key hint inside, GTK cannot pick a name for it,
    // so spell it out or a screen reader announces an unnamed button.
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    clickable(&button);
    let popover = popover.downgrade();
    // The action rebuilds the row this popover hangs off, so run it once the
    // popover is done emitting rather than tearing it down under itself.
    let f = Rc::new(f);
    button.connect_clicked(move |_| {
        if let Some(popover) = popover.upgrade() {
            popover.popdown();
        }
        let f = f.clone();
        glib::idle_add_local_once(move || f());
    });
    button
}

fn menu_separator() -> gtk::Separator {
    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    separator.add_css_class("menu-sep");
    separator
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
        format!("{m:02}:{s:02}")
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
        assert_eq!(fmt_elapsed(0), "00:00");
        assert_eq!(fmt_elapsed(65), "01:05");
        assert_eq!(fmt_elapsed(762), "12:42");
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

    /// The queue page lists the tail of Now under Next, so a drop there is
    /// offset past the banner's task and can never displace it.
    #[test]
    fn dropping_into_the_now_tail_never_displaces_the_current_task() {
        let mut store = Store::new();
        let current = store.add("current", Bucket::Now, None, false);
        let queued = store.add("queued", Bucket::Now, None, false);
        let other = store.add("other", Bucket::Next, None, false);

        // Row 0 of the tail list is store index 1: one past the current task.
        let head_offset = 1;
        let index = adjusted_drop_index(&store, other, Bucket::Now, Some(head_offset));
        assert!(store.move_to(other, Bucket::Now, index));

        assert_eq!(ids(&store, Bucket::Now), vec![current, other, queued]);
        assert_eq!(store.current().map(|t| t.id), Some(current));
    }
}
