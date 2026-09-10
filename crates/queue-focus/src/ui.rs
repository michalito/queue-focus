//! Windows: the main window (Queue / Board pages) and the quick-add popup.
//! Both pages are views over the same store and are rebuilt on every change
//! (task counts are small; rebuilding is simpler and always correct).
//!
//! The queue page is three fixed bands: the current task's banner, a scrolling
//! card holding Side and Next, and a Later shelf pinned to the bottom. The head
//! of Now lives in the banner, so the rest of the Now bucket is listed under the
//! Next header — everything queued behind what you are doing.

use crate::flash::SharedFlash;
use crate::settings::SharedSettings;
use crate::state::{SharedState, UpdateOutcome};
use adw::prelude::*;
use gtk::{gdk, glib, pango};
use qf_core::{
    Bucket, FlashColor, Intensity, Settings, Store, Tag, Task, Theme, INTERVAL_MAX, INTERVAL_MIN,
    MAX_TITLE_CHARS,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[cfg(test)]
mod gtk_tests;
mod placement;
use placement::{Destination, Focus, Placement, ORDER};

const CSS: &str = include_str!("style.css");

const QUEUE_SIZE: (i32, i32) = (400, 640);
const BOARD_SIZE: (i32, i32) = (1040, 640);

/// Bucket order in the "⏎ adds to" control: the usual answer comes first.
const BUCKET_ORDER: [Bucket; 4] = [Bucket::Next, Bucket::Now, Bucket::Side, Bucket::Later];

/// The `?` popover, in order.
const SHORTCUTS: [(&str, &str); 12] = [
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
    ("Ctrl+,", "settings"),
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

/// Puts one stored setting into the control that shows it.
type SettingsSync = Rc<dyn Fn(&Settings)>;

/// Force the window's colour scheme, or hand it back to the desktop.
pub fn apply_theme(theme: Theme) {
    adw::StyleManager::default().set_color_scheme(match theme {
        Theme::System => adw::ColorScheme::Default,
        Theme::Light => adw::ColorScheme::ForceLight,
        Theme::Dark => adw::ColorScheme::ForceDark,
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Queue,
    Board,
    Settings,
}

impl Page {
    fn name(self) -> &'static str {
        match self {
            Page::Queue => "queue",
            Page::Board => "board",
            Page::Settings => "settings",
        }
    }

    pub fn parse(s: &str) -> Page {
        match s {
            "board" => Page::Board,
            "settings" => Page::Settings,
            _ => Page::Queue,
        }
    }
}

/// How much furniture a row carries, and how it is dressed. The board's
/// quadrants are each shaped differently, so each gets its own style: Side is a
/// stack of free-standing cards, Next a card of hairline-separated rows, and
/// Later a bare column of dim single lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowStyle {
    /// Queue page: title, a "make current" button, and the menu.
    Queue,
    /// Queue page's Later shelf: dim, with a "→ next" shortcut.
    Later,
    /// Board's Side quadrant: a card of its own, tag chip leading.
    SideCard,
    /// Board's Next quadrant: a row in a card, under a hairline.
    BoardRow,
    /// Board's Later quadrant: one dim line, ellipsised.
    BoardLater,
}

impl RowStyle {
    fn of(list: placement::List) -> RowStyle {
        match (list.page, list.bucket) {
            (Page::Board, Bucket::Side) => RowStyle::SideCard,
            (Page::Board, Bucket::Later) => RowStyle::BoardLater,
            // The Now tail is listed under Next, so it wears Next's clothes.
            (Page::Board, _) => RowStyle::BoardRow,
            (_, Bucket::Later) => RowStyle::Later,
            _ => RowStyle::Queue,
        }
    }

    /// How many lines a title may take. Nothing wraps on the queue, whose rows
    /// are one line cut short; the board's quadrants are narrow enough to need
    /// the room. Later is cold storage either way: one line.
    fn wrap_lines(self) -> Option<i32> {
        match self {
            RowStyle::SideCard => Some(2),
            RowStyle::BoardRow => Some(3),
            _ => None,
        }
    }

    /// The space between a row's parts. Side's cards are the roomier ones.
    fn gap(self) -> i32 {
        match self {
            RowStyle::SideCard => 10,
            _ => 8,
        }
    }

    /// Whether rows in this style carry buttons beside the menu. The board's
    /// quadrants are too narrow for them, so there the menu carries everything.
    fn inline_buttons(self) -> bool {
        matches!(self, RowStyle::Queue | RowStyle::Later)
    }

    /// The class its list carries. Rows are dressed through it — `row` is
    /// GTK's own node name for a GtkListBoxRow — so they need none of their own.
    fn css(self) -> &'static str {
        match self {
            RowStyle::Queue => "queue-rows",
            RowStyle::Later => "later-rows",
            RowStyle::SideCard => "side-cards",
            RowStyle::BoardRow => "board-rows",
            RowStyle::BoardLater => "board-later-rows",
        }
    }
}

/// The current task's panel. The queue wears it as a full-width band under the
/// header bar; the board as a card in its own quadrant, with the word "Done"
/// spelt out and a tag chip standing by even when there is no tag to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeroStyle {
    Banner,
    Card,
}

/// One page's current-task panel, rebuilt with everything else.
struct Hero {
    page: Page,
    style: HeroStyle,
    root: gtk::Box,
}

/// One ListBox showing (part of) one bucket on one page.
struct BucketList {
    placement: placement::List,
    style: RowStyle,
    list: gtk::ListBox,
}

/// A titled section: the count beside its header, and the "empty" placeholder
/// shown while every list under that header is empty. The board's Now header
/// has no placeholder — the hero carries its own empty state.
struct Section {
    count: gtk::Label,
    placeholder: Option<gtk::Label>,
    placement: placement::Section,
}

pub struct Ui {
    app: adw::Application,
    state: SharedState,
    settings: SharedSettings,
    flash: SharedFlash,
    win: RefCell<Option<adw::ApplicationWindow>>,
    stack: RefCell<Option<adw::ViewStack>>,
    entry: RefCell<Option<gtk::Entry>>,
    /// Header bar furniture that changes with the page.
    header: RefCell<Option<(adw::HeaderBar, gtk::Widget, gtk::Label, gtk::Button)>>,
    /// The quick-add band, hidden on the settings page.
    entry_bar: RefCell<Option<gtk::Widget>>,
    /// The page the gear was pressed from, so pressing it again goes back.
    came_from: Cell<Page>,
    /// Pushes the stored settings into the settings page's controls.
    settings_sync: RefCell<Vec<SettingsSync>>,
    /// Set while `sync_settings` writes into those controls, so the handlers
    /// they fire do not write the same value straight back.
    syncing: Cell<bool>,
    /// The "next flash in …" line and the button beside it.
    countdown: RefCell<Option<(gtk::Label, gtk::Button)>>,
    /// A settings failure waiting for a window to be shown in: heading, body.
    pending_problem: RefCell<Option<(String, String)>>,
    lists: RefCell<Vec<BucketList>>,
    sections: RefCell<Vec<Section>>,
    rendered: RefCell<Placement>,
    /// The current task's panel, one per page that shows one.
    heroes: RefCell<Vec<Hero>>,
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
    pub fn new(
        app: adw::Application,
        state: SharedState,
        settings: SharedSettings,
        flash: SharedFlash,
    ) -> Rc<Ui> {
        let ui = Rc::new(Ui {
            app,
            state: state.clone(),
            settings: settings.clone(),
            flash,
            win: RefCell::new(None),
            stack: RefCell::new(None),
            entry: RefCell::new(None),
            header: RefCell::new(None),
            entry_bar: RefCell::new(None),
            came_from: Cell::new(Page::Queue),
            settings_sync: RefCell::new(Vec::new()),
            syncing: Cell::new(false),
            countdown: RefCell::new(None),
            pending_problem: RefCell::new(None),
            rendered: RefCell::new(Placement::default()),
            lists: RefCell::new(Vec::new()),
            sections: RefCell::new(Vec::new()),
            heroes: RefCell::new(Vec::new()),
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
        // A setting changed here, or from another client over D-Bus: put the
        // new values into the controls and act on the ones that show.
        let weak = Rc::downgrade(&ui);
        settings.on_change(move || {
            if let Some(ui) = weak.upgrade() {
                apply_theme(ui.settings.get().theme);
                ui.sync_settings();
            }
        });
        let weak = Rc::downgrade(&ui);
        settings.on_problem(move |message| {
            if let Some(ui) = weak.upgrade() {
                // The service usually runs with no window at all, and the
                // store only complains once per outage. Keep the message until
                // there is somewhere to show it.
                ui.queue_settings_problem("Could not save your settings", message);
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
        self.show_pending_problem();
    }

    /// Keep a settings failure until there is a visible window for its alert.
    /// The caller has already written the same failure to stderr.
    pub fn queue_settings_problem(&self, heading: &str, message: &str) {
        *self.pending_problem.borrow_mut() = Some((heading.into(), message.into()));
        self.show_pending_problem();
    }

    /// Show a settings failure that happened while nobody was looking. It is
    /// reported once, so it has to wait rather than be dropped.
    fn show_pending_problem(&self) {
        if self.visible_window().is_none() {
            return;
        }
        let Some((heading, message)) = self.pending_problem.borrow_mut().take() else {
            return;
        };
        self.show_alert(&heading, &message);
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

    /// Run a persisted mutation asked for from this window and report on it.
    fn update<R>(&self, f: impl FnOnce(&mut Store) -> R) -> Result<R, std::io::Error> {
        self.report(self.state.update(f))
    }

    /// Mark a task done. Only completing the current task pulls the head of
    /// Next, and only that can be undone (from the top bar).
    fn complete(&self, id: u64) -> bool {
        self.report(self.state.complete(id)).unwrap_or(false)
    }

    /// A failure to commit is an error the user must see; a change that
    /// committed but may not be crash-safe is a warning. Both go to the
    /// window the user is looking at, and always to stderr.
    fn report<R>(
        &self,
        result: Result<UpdateOutcome<R>, std::io::Error>,
    ) -> Result<R, std::io::Error> {
        match result {
            Ok(outcome) => {
                let (value, warning) = outcome.into_parts();
                if let Some(warning) = warning {
                    self.alert("Task change saved with a warning", &warning.to_string());
                }
                Ok(value)
            }
            Err(e) => {
                self.alert("Could not safely save task changes", &e.to_string());
                Err(e)
            }
        }
    }

    fn alert(&self, heading: &str, body: &str) {
        eprintln!("queue-focus: {heading}: {body}");
        self.show_alert(heading, body);
    }

    /// The dialog on its own. Whoever has already logged the trouble uses this
    /// so it is not written to the log twice.
    fn show_alert(&self, heading: &str, body: &str) {
        let Some(win) = self.visible_window() else {
            return;
        };
        let dialog = adw::AlertDialog::new(Some(heading), Some(body));
        dialog.add_response("ok", "OK");
        dialog.present(Some(&win));
    }

    /// A dialog presented on a hidden window is never seen: only offer visible ones.
    fn visible_window(&self) -> Option<gtk::Window> {
        let main = self
            .win
            .borrow()
            .as_ref()
            .filter(|w| w.is_visible())
            .map(|w| w.clone().upcast::<gtk::Window>());
        main.or_else(|| {
            self.quick
                .borrow()
                .as_ref()
                .filter(|(w, _)| w.is_visible())
                .map(|(w, _)| w.clone())
        })
    }

    /// Tiny floating entry for capture from anywhere.
    pub fn quick_add_dialog(self: &Rc<Self>) {
        if let Some((w, e)) = self.quick.borrow().as_ref() {
            w.present();
            e.grab_focus();
            self.show_pending_problem();
            return;
        }
        let entry = gtk::Entry::builder()
            .placeholder_text("Add…  !now  #w #p  @later @side  (⏎)")
            .max_length(MAX_TITLE_CHARS as i32)
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
            let default = this.settings.get().default_bucket;
            if this.submit(e, default) {
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
        self.show_pending_problem();
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
        let switcher = self.build_view_switcher(&stack);
        let header = adw::HeaderBar::builder()
            .title_widget(&switcher)
            .decoration_layout(":close")
            .build();
        // Packed end-first: the shortcuts button sits beside the close button
        // and the gear goes to its left, as the design has them.
        header.pack_end(&self.build_shortcuts_button());
        let gear = gtk::Button::builder()
            .child(&gtk::Label::new(Some("⚙")))
            .tooltip_text("Settings (Ctrl+,)")
            .valign(gtk::Align::Center)
            .css_classes(["flat", "hint-btn", "gear-btn"])
            .build();
        clickable(&gear);
        let this = self.clone();
        gear.connect_clicked(move |_| this.toggle_settings());
        header.pack_end(&gear);
        // The switcher has no place on the settings page; a plain title does.
        let title = gtk::Label::builder()
            .label("Settings")
            .css_classes(["settings-title"])
            .build();
        *self.header.borrow_mut() = Some((
            header.clone(),
            switcher.clone().upcast::<gtk::Widget>(),
            title,
            gear,
        ));

        // Shared quick-add entry under the header.
        let entry = gtk::Entry::builder()
            .placeholder_text("Add…  !now  #w #p  @later @side  (⏎)")
            .max_length(MAX_TITLE_CHARS as i32)
            .hexpand(true)
            .css_classes(["main-entry"])
            .build();
        let entry_bar = gtk::Box::builder().css_classes(["entry-bar"]).build();
        entry_bar.append(&entry);
        let this = self.clone();
        entry.connect_activate(move |e| {
            let default = this.settings.get().default_bucket;
            this.submit(e, default);
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
        stack.add_titled(
            &self.build_settings_page(),
            Some(Page::Settings.name()),
            "Settings",
        );

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&entry_bar);
        toolbar.set_content(Some(&stack));
        win.set_content(Some(&toolbar));
        *self.entry_bar.borrow_mut() = Some(entry_bar.upcast::<gtk::Widget>());

        // The board wants a wider window; settings keeps the queue's.
        let w = win.clone();
        let this = self.clone();
        stack.connect_visible_child_name_notify(move |s| {
            let page = s
                .visible_child_name()
                .as_deref()
                .map(Page::parse)
                .unwrap_or(Page::Queue);
            let (dw, dh) = match page {
                Page::Board => BOARD_SIZE,
                _ => QUEUE_SIZE,
            };
            w.set_default_size(dw, dh);
            this.dress_header(page);
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
                Some(Page::Queue) => q.set_active(true),
                // Settings hides the switcher. Activating a button here would
                // toggle it, and its handler would pull the stack straight
                // back off the page we have just moved to.
                _ => {}
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

        page.append(&self.make_hero(Page::Queue, HeroStyle::Banner, &["current-banner"]));

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
        let placeholder = placeholder_label();
        let placement = placement::Section {
            page: Page::Queue,
            bucket,
        };
        let lists = self.make_lists(placement);

        // A header appends to the bucket it names, which is the last list under
        // it: Next's own rows, not the Now tail queued in front of them.
        self.header_drop(&head_box, bucket, &lists);
        self.empty_drop(&placeholder, bucket);
        card.append(&head_box);
        card.append(&placeholder);
        for list in &lists {
            card.append(list);
        }
        self.sections.borrow_mut().push(Section {
            count,
            placeholder: Some(placeholder),
            placement,
        });
    }

    /// Every list under one header, in the order they are shown.
    fn make_lists(self: &Rc<Self>, section: placement::Section) -> Vec<gtk::ListBox> {
        section
            .lists()
            .into_iter()
            .map(|region| self.make_list(region))
            .collect()
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
        shelf.append(&toggle);

        let placeholder = placeholder_label();
        let list = self.make_list(placement::List {
            page: Page::Queue,
            bucket: Bucket::Later,
        });
        self.header_drop(&toggle, Bucket::Later, std::slice::from_ref(&list));
        self.empty_drop(&placeholder, Bucket::Later);
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
            placeholder: Some(placeholder),
            placement: placement::Section {
                page: Page::Queue,
                bucket: Bucket::Later,
            },
        });
        shelf.upcast()
    }

    /// Board page: four quadrants. Now and Next take the wide left half, Side
    /// and Later the narrow right one, and the bottom row grows.
    fn build_board_page(self: &Rc<Self>) -> gtk::Widget {
        let grid = gtk::Grid::builder()
            .column_homogeneous(true)
            .column_spacing(14)
            .row_spacing(6)
            .css_classes(["board"])
            .build();
        // GTK has no proportional columns, so the design's 1.5fr : 1fr is five
        // equal ones, spanned three and two.
        for (bucket, x, width, y) in [
            (Bucket::Now, 0, 3, 0),
            (Bucket::Side, 3, 2, 0),
            (Bucket::Next, 0, 3, 1),
            (Bucket::Later, 3, 2, 1),
        ] {
            let quadrant = self.build_board_quadrant(bucket);
            // The top row is as tall as it needs to be; the bottom takes what
            // is left, so Next and Later are the two that scroll.
            quadrant.set_vexpand(y == 1);
            grid.attach(&quadrant, x, y, width, 1);
        }
        grid.upcast()
    }

    /// One quadrant: the bucket's header over a body shaped to suit it.
    fn build_board_quadrant(self: &Rc<Self>, bucket: Bucket) -> gtk::Box {
        let (head_box, count) = section_header(bucket, false);
        let quadrant = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["board-column"])
            .build();
        head_box.add_css_class(bucket.as_str());
        quadrant.append(&head_box);
        let placement = placement::Section {
            page: Page::Board,
            bucket,
        };

        if bucket == Bucket::Now {
            // The hero shows the one task on show and nothing else — whatever
            // is queued behind it is listed under Next — so this quadrant has
            // no list, and no "empty" line of its own either.
            let hero = self.make_hero(Page::Board, HeroStyle::Card, &["now-hero"]);
            hero.set_vexpand(true);
            quadrant.append(&hero);
            // The heading appends behind the current task; the hero promotes.
            self.append_drop(&head_box, bucket, Highlight::Ring);
            self.sections.borrow_mut().push(Section {
                count,
                placeholder: None,
                placement,
            });
            return quadrant;
        }

        let placeholder = placeholder_label();
        placeholder.add_css_class(&format!("{}-empty", bucket.as_str()));
        if bucket == Bucket::Side {
            // Side's is a box of its own rather than a line of text.
            placeholder.set_xalign(0.5);
            placeholder.set_valign(gtk::Align::Center);
        }
        let lists = self.make_lists(placement);

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            // Fill the viewport rather than only the rows, so the space under
            // the last one still belongs to the bucket.
            .vexpand(true)
            .build();
        body.append(&placeholder);
        for list in &lists {
            body.append(list);
        }
        self.header_drop(&head_box, bucket, &lists);
        self.empty_drop(&placeholder, bucket);
        // A sibling target covers spare space without seeing motion events
        // that bubble from the list or placeholder and replacing their marks.
        let drop_space = gtk::Box::builder()
            .vexpand(true)
            .css_classes(["board-drop-space"])
            .build();
        body.append(&drop_space);
        self.append_drop(&drop_space, bucket, end_of(&lists));

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&body)
            .build();
        match bucket {
            // Side sits in the top row: it takes the room it needs, between a
            // card's worth and a screenful.
            Bucket::Side => {
                scroll.set_propagate_natural_height(true);
                scroll.set_min_content_height(72);
                scroll.set_max_content_height(196);
            }
            // Next is a card of its own; Later is a bare column.
            Bucket::Next => {
                scroll.add_css_class("board-card");
                scroll.set_vexpand(true);
            }
            _ => scroll.set_vexpand(true),
        }
        quadrant.append(&scroll);
        self.sections.borrow_mut().push(Section {
            count,
            placeholder: Some(placeholder),
            placement,
        });
        quadrant
    }

    // ---- settings page ------------------------------------------------

    /// Settings: the reminder first, then the rules that keep it quiet, then
    /// the small things. One scrolling column of cards in the queue's clothes.
    fn build_settings_page(self: &Rc<Self>) -> gtk::Widget {
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(22)
            .css_classes(["settings-page"])
            .build();

        let card = self.settings_card(
            &column,
            "Reminder",
            Some(
                "The screen flashes the current task and its time. Never while Now is empty. \
                 The GNOME Shell extension draws the flash.",
            ),
        );
        self.interval_row(&card);
        self.switch_row(
            &card,
            true,
            "Vary the flash",
            Some("Picks one of six styles at random, so you do not learn to ignore the one."),
            |s| s.vary,
            |s, on| s.vary = on,
        );
        self.choice_row(
            &card,
            "Intensity",
            None,
            &Intensity::ALL.map(|i| (i, i.label())),
            |s| s.intensity,
            |s, v| s.intensity = v,
        );
        self.choice_row(
            &card,
            "Color",
            None,
            &FlashColor::ALL.map(|c| (c, c.label())),
            |s| s.color,
            |s, v| s.color = v,
        );
        self.try_it_row(&card);

        let card = self.settings_card(&column, "Quiet", Some("When the reminder holds back."));
        self.switch_row(
            &card,
            false,
            "While the timer is paused",
            Some("A paused task is a deliberate break."),
            |s| s.quiet_paused,
            |s, on| s.quiet_paused = on,
        );
        self.switch_row(
            &card,
            true,
            "Outside hours",
            None,
            |s| s.quiet_hours,
            |s, on| s.quiet_hours = on,
        );
        self.quiet_hours_row(&card);

        let card = self.settings_card(&column, "Appearance", None);
        self.choice_row(
            &card,
            "Theme",
            None,
            &Theme::ALL.map(|t| (t, t.label())),
            |s| s.theme,
            |s, v| s.theme = v,
        );

        let card = self.settings_card(&column, "Top bar", None);
        self.switch_row(
            &card,
            false,
            "Show elapsed time",
            Some("Off keeps the title alone: <tt>● ship v0.1</tt>"),
            |s| s.show_timer,
            |s, on| s.show_timer = on,
        );

        let card = self.settings_card(&column, "Quick add", None);
        self.choice_row(
            &card,
            "⏎ adds to",
            Some("Ctrl+⏎ always goes to Now; @markers still win."),
            &BUCKET_ORDER.map(|b| (b, b.label())),
            |s| s.default_bucket,
            |s, v| s.default_bucket = v,
        );

        gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&column)
            .vexpand(true)
            .build()
            .upcast()
    }

    /// A heading, an optional line of explanation, and the card the rows go in.
    fn settings_card(
        self: &Rc<Self>,
        column: &gtk::Box,
        title: &str,
        description: Option<&str>,
    ) -> gtk::Box {
        let section = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .build();
        let heading = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .css_classes(["settings-heading"])
            .build();
        heading.append(
            &gtk::Label::builder()
                .label(title)
                .xalign(0.0)
                .css_classes(["settings-group-title"])
                .build(),
        );
        if let Some(description) = description {
            heading.append(
                &gtk::Label::builder()
                    .label(description)
                    .xalign(0.0)
                    .wrap(true)
                    .css_classes(["settings-description"])
                    .build(),
            );
        }
        section.append(&heading);

        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["settings-card"])
            .build();
        section.append(&card);
        column.append(&section);
        card
    }

    /// One row of a settings card: a label on the left, a control on the right.
    fn settings_row(card: &gtk::Box, divided: bool) -> gtk::Box {
        let classes: Vec<&str> = if divided {
            vec!["settings-row", "divided"]
        } else {
            vec!["settings-row"]
        };
        let row = gtk::Box::builder().spacing(12).css_classes(classes).build();
        card.append(&row);
        row
    }

    /// The label and its explanation. Subtitles may carry Pango markup so a
    /// setting can quote what it changes.
    fn row_label(title: &str, subtitle: Option<&str>) -> gtk::Box {
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(1)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        text.append(
            &gtk::Label::builder()
                .label(title)
                .xalign(0.0)
                .ellipsize(pango::EllipsizeMode::End)
                .build(),
        );
        if let Some(subtitle) = subtitle {
            text.append(
                &gtk::Label::builder()
                    .label(subtitle)
                    .use_markup(true)
                    .xalign(0.0)
                    .wrap(true)
                    .wrap_mode(pango::WrapMode::WordChar)
                    .css_classes(["settings-subtitle"])
                    .build(),
            );
        }
        text
    }

    /// "Flash every": the value, the slider, and the marks along it.
    fn interval_row(self: &Rc<Self>, card: &gtk::Box) {
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .css_classes(["settings-row", "settings-slider-row"])
            .build();

        let top = gtk::Box::builder().spacing(8).build();
        top.append(
            &gtk::Label::builder()
                .label("Flash every")
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );
        let value = gtk::Label::builder()
            .css_classes(["settings-value", "monospace", "numeric"])
            .build();
        top.append(&value);
        row.append(&top);

        let scale = gtk::Scale::with_range(
            gtk::Orientation::Horizontal,
            INTERVAL_MIN as f64,
            INTERVAL_MAX as f64,
            1.0,
        );
        scale.set_draw_value(false);
        scale.set_round_digits(0);
        scale.set_hexpand(true);
        for (at, label) in [
            (INTERVAL_MIN as f64, "1m"),
            (15.0, "15"),
            (30.0, "30"),
            (60.0, "60"),
            (INTERVAL_MAX as f64, "90"),
        ] {
            scale.add_mark(at, gtk::PositionType::Bottom, Some(label));
        }
        scale.update_property(&[gtk::accessible::Property::Label("Flash every, in minutes")]);
        clickable(&scale);
        row.append(&scale);
        card.append(&row);

        let this = self.clone();
        scale.connect_value_changed(move |s| {
            if this.syncing.get() {
                return;
            }
            let minutes = s.value().round().max(0.0) as u32;
            this.settings.update(|s| s.interval_min = minutes);
        });

        let (scale, value) = (scale.clone(), value.clone());
        self.on_settings(move |s| {
            scale.set_value(s.interval_min as f64);
            value.set_label(&format!("{} min", s.interval_min));
        });
    }

    /// A row whose control is a switch.
    fn switch_row(
        self: &Rc<Self>,
        card: &gtk::Box,
        divided: bool,
        title: &str,
        subtitle: Option<&str>,
        get: impl Fn(&Settings) -> bool + 'static,
        set: impl Fn(&mut Settings, bool) + 'static,
    ) {
        let row = Self::settings_row(card, divided);
        row.append(&Self::row_label(title, subtitle));
        let switch = gtk::Switch::builder()
            .valign(gtk::Align::Center)
            .css_classes(["settings-switch"])
            .build();
        clickable(&switch);
        // Named for a screen reader: the switch itself carries no text.
        switch.update_property(&[gtk::accessible::Property::Label(title)]);
        row.append(&switch);

        let this = self.clone();
        switch.connect_active_notify(move |s| {
            if this.syncing.get() {
                return;
            }
            let on = s.is_active();
            this.settings.update(|settings| set(settings, on));
        });

        let switch = switch.clone();
        self.on_settings(move |s| switch.set_active(get(s)));
    }

    /// A row whose control is a segmented button group.
    fn choice_row<T: Copy + PartialEq + 'static>(
        self: &Rc<Self>,
        card: &gtk::Box,
        title: &str,
        subtitle: Option<&str>,
        options: &[(T, &str)],
        get: impl Fn(&Settings) -> T + 'static,
        set: impl Fn(&mut Settings, T) + 'static,
    ) {
        let row = Self::settings_row(card, card.first_child().is_some());
        row.append(&Self::row_label(title, subtitle));

        // Four choices only fit across a 400px window at a tighter padding.
        let classes: Vec<&str> = if options.len() > 3 {
            vec!["view-switch", "narrow"]
        } else {
            vec!["view-switch"]
        };
        let group = gtk::Box::builder()
            .valign(gtk::Align::Center)
            .css_classes(classes)
            .build();
        let set = Rc::new(set);
        let mut buttons: Vec<(T, gtk::ToggleButton)> = Vec::new();
        for (value, label) in options {
            let button = gtk::ToggleButton::builder().label(*label).build();
            if let Some((_, first)) = buttons.first() {
                button.set_group(Some(first));
            }
            clickable(&button);
            group.append(&button);

            let this = self.clone();
            let set = set.clone();
            let value = *value;
            button.connect_toggled(move |b| {
                if !b.is_active() || this.syncing.get() {
                    return;
                }
                this.settings.update(|settings| set(settings, value));
            });
            buttons.push((value, button));
        }
        row.append(&group);

        self.on_settings(move |s| {
            let active = get(s);
            for (value, button) in &buttons {
                if *value == active {
                    button.set_active(true);
                }
            }
        });
    }

    /// "Try it": how long until the next flash, and a button for one now.
    fn try_it_row(self: &Rc<Self>, card: &gtk::Box) {
        let row = Self::settings_row(card, true);
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(1)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        text.append(&gtk::Label::builder().label("Try it").xalign(0.0).build());
        let countdown = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["settings-subtitle"])
            .build();
        text.append(&countdown);
        row.append(&text);

        let button = gtk::Button::builder()
            .label("Flash now")
            .valign(gtk::Align::Center)
            .build();
        clickable(&button);
        let this = self.clone();
        button.connect_clicked(move |_| {
            if !this.flash.flash_now() {
                this.alert(
                    "Nothing to flash",
                    "The reminder shows the current task, and Now is empty.",
                );
            }
        });
        row.append(&button);

        *self.countdown.borrow_mut() = Some((countdown, button));
        self.refresh_countdown();
    }

    /// The hours the reminder is allowed to flash in, shown only while the
    /// switch above it is on.
    fn quiet_hours_row(self: &Rc<Self>, card: &gtk::Box) {
        let row = Self::settings_row(card, true);
        row.append(
            &gtk::Label::builder()
                .label("Flash only between")
                .xalign(0.0)
                .hexpand(true)
                .css_classes(["settings-dim"])
                .build(),
        );
        row.append(&self.time_entry("Flash from", |s| s.quiet_from, |s, t| s.quiet_from = t));
        row.append(
            &gtk::Label::builder()
                .label("–")
                .css_classes(["settings-dim"])
                .build(),
        );
        row.append(&self.time_entry("Flash until", |s| s.quiet_to, |s, t| s.quiet_to = t));

        // The row is built into the card and then revealed, so turning the
        // switch on does not have to rebuild anything.
        card.remove(&row);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(150)
            .child(&row)
            .build();
        card.append(&revealer);

        let revealer = revealer.clone();
        self.on_settings(move |s| revealer.set_reveal_child(s.quiet_hours));
    }

    /// A five-character clock face. Anything that is not a time is refused and
    /// the entry goes back to the value it is showing for.
    fn time_entry(
        self: &Rc<Self>,
        name: &str,
        get: impl Fn(&Settings) -> qf_core::TimeOfDay + 'static,
        set: impl Fn(&mut Settings, qf_core::TimeOfDay) + 'static,
    ) -> gtk::Entry {
        let entry = gtk::Entry::builder()
            .width_chars(5)
            .max_width_chars(5)
            .max_length(5)
            .xalign(0.5)
            .valign(gtk::Align::Center)
            .css_classes(["time-entry", "monospace", "numeric"])
            .build();
        entry.update_property(&[gtk::accessible::Property::Label(name)]);

        let get = Rc::new(get);
        let set = Rc::new(set);
        // Commit on Enter and when the entry is left; a value that will not
        // parse is dropped rather than half-applied.
        let commit = {
            let this = self.clone();
            let (get, set) = (get.clone(), set.clone());
            move |e: &gtk::Entry| {
                if this.syncing.get() {
                    return;
                }
                match qf_core::TimeOfDay::parse(&e.text()) {
                    Some(time) => this.settings.update(|s| set(s, time)),
                    None => e.set_text(&get(&this.settings.get()).to_string()),
                }
                // A time the store rounded or refused must not stay on screen.
                e.set_text(&get(&this.settings.get()).to_string());
            }
        };
        let activate = commit.clone();
        entry.connect_activate(move |e| activate(e));
        let focus = gtk::EventControllerFocus::new();
        let leave = commit.clone();
        let weak = entry.downgrade();
        focus.connect_leave(move |_| {
            if let Some(entry) = weak.upgrade() {
                leave(&entry);
            }
        });
        entry.add_controller(focus);
        let this = self.clone();
        let restore = get.clone();
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |c, key, _, _| {
            let Some(entry) = c.widget().and_downcast::<gtk::Entry>() else {
                return glib::Propagation::Proceed;
            };
            if key == gdk::Key::Escape {
                let stored = restore(&this.settings.get()).to_string();
                if entry.text() == stored {
                    this.hide();
                } else {
                    entry.set_text(&stored);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        entry.add_controller(keys);

        let field = entry.clone();
        self.on_settings(move |s| field.set_text(&get(s).to_string()));
        entry
    }

    /// Register something that has to follow the stored settings.
    fn on_settings(&self, f: impl Fn(&Settings) + 'static) {
        // Copied out of the store, and behind the same guard `sync_settings`
        // uses: putting a value into a control makes the control tell us about
        // it, and that handler would otherwise write to a store this call is
        // still reading.
        let settings = self.settings.get().clone();
        let was_syncing = self.syncing.replace(true);
        f(&settings);
        self.syncing.set(was_syncing);
        self.settings_sync.borrow_mut().push(Rc::new(f));
    }

    /// Put the stored settings back into every control on the page. The guard
    /// stops each control's own handler writing the value straight back.
    fn sync_settings(&self) {
        if self.syncing.replace(true) {
            return;
        }
        let settings = self.settings.get().clone();
        // Cloned out of the borrow: a control may be built while syncing.
        let sync: Vec<SettingsSync> = self.settings_sync.borrow().clone();
        for f in sync {
            f(&settings);
        }
        self.syncing.set(false);
        self.refresh_countdown();
    }

    /// The line under "Try it", once a second and after every change.
    fn refresh_countdown(&self) {
        let Some((label, button)) = self.countdown.borrow().clone() else {
            return;
        };
        let status = self.flash.status();
        match status.remaining {
            Some(secs) => label.set_label(&format!("Next flash in {}", fmt_elapsed(secs))),
            None => label.set_label(&format!("No flash: {}", status.hold.reason())),
        }
        // Nothing in Now is the one hold a flash cannot be asked for either.
        button.set_sensitive(status.hold != qf_core::Hold::NoCurrentTask);
    }

    /// Show the settings page, or leave it for wherever the gear was pressed.
    fn toggle_settings(self: &Rc<Self>) {
        if self.current_page() == Page::Settings {
            self.set_page(self.came_from.get());
        } else {
            self.set_page(Page::Settings);
        }
    }

    /// The header bar wears the switcher on the task pages and a plain title
    /// on the settings page, where the gear it came from stays lit.
    fn dress_header(&self, page: Page) {
        let Some((header, switcher, title, gear)) = self.header.borrow().clone() else {
            return;
        };
        let settings = page == Page::Settings;
        if settings {
            header.set_title_widget(Some(&title));
            gear.add_css_class("active");
        } else {
            header.set_title_widget(Some(&switcher));
            gear.remove_css_class("active");
        }
        if let Some(bar) = self.entry_bar.borrow().as_ref() {
            bar.set_visible(!settings);
        }
    }

    fn make_list(self: &Rc<Self>, placement: placement::List) -> gtk::ListBox {
        let style = RowStyle::of(placement);
        let classes = vec!["bucket-list", style.css()];
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

        // Drop on the list → wherever the insertion line was promising.
        let this = self.clone();
        list.add_controller(drop_target(Highlight::Before, move |id, target, y| {
            let before = target
                .downcast_ref::<gtk::ListBox>()
                .and_then(|l| anchor_at(l, y))
                .and_then(|r| row_id(&r));
            this.update(|s| {
                Destination::List {
                    list: placement,
                    before,
                }
                .apply(s, id)
            })
            .unwrap_or(false)
        }));

        self.lists.borrow_mut().push(BucketList {
            placement,
            style,
            list: list.clone(),
        });
        list
    }

    /// Dropping on a bucket's header appends to it — this is the only way into
    /// a collapsed or empty bucket. The line shows after the last row it holds.
    fn header_drop(
        self: &Rc<Self>,
        header: &impl IsA<gtk::Widget>,
        bucket: Bucket,
        lists: &[gtk::ListBox],
    ) {
        self.append_drop(header, bucket, end_of(lists));
    }

    /// A list is zero-height while empty, so its placeholder has to accept the
    /// drop that would otherwise have landed on the list.
    fn empty_drop(self: &Rc<Self>, placeholder: &impl IsA<gtk::Widget>, bucket: Bucket) {
        self.append_drop(placeholder, bucket, Highlight::Ring);
    }

    fn append_drop(
        self: &Rc<Self>,
        widget: &impl IsA<gtk::Widget>,
        bucket: Bucket,
        highlight: Highlight,
    ) {
        let this = self.clone();
        widget.add_controller(drop_target(highlight, move |id, _, _| {
            this.update(|s| Destination::Append(bucket).apply(s, id))
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
            .and_then(|r| row_id(&r))
            .and_then(|id| Focus::capture(&self.visible_ids(), id));

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
        self.timers.borrow_mut().clear();
        *self.rename_entry.borrow_mut() = None;

        let placement = Placement::new(&store);
        self.build_heroes(current);

        for bl in self.lists.borrow().iter() {
            bl.list.remove_all();
            let leads = placement.leads(bl.placement);
            for (i, id) in placement.rows(bl.placement).iter().enumerate() {
                let t = store.get(*id).expect("placement comes from this store");
                let row = self.build_row(t, bl.placement, bl.style, leads && i == 0);
                bl.list.append(&row);
            }
        }
        for section in self.sections.borrow().iter() {
            let n = placement.count(section.placement);
            section.count.set_label(&n.to_string());
            if let Some(placeholder) = &section.placeholder {
                placeholder.set_visible(n == 0);
            }
        }
        *self.rendered.borrow_mut() = placement;

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
        if let Some(id) = focused.and_then(|focus| focus.restore(&self.visible_ids())) {
            if let Some(row) = self.row_for(id) {
                row.grab_focus();
            }
        }
    }

    /// The current task's panel. Dropping on it promotes, which on the queue
    /// page is the only way to move a task right to the front.
    fn make_hero(self: &Rc<Self>, page: Page, style: HeroStyle, classes: &[&str]) -> gtk::Box {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(if style == HeroStyle::Card { 8 } else { 6 })
            .css_classes(classes.to_vec())
            .build();
        let this = self.clone();
        root.add_controller(drop_target(Highlight::Ring, move |id, _, _| {
            this.update(|s| Destination::Banner.apply(s, id))
                .unwrap_or(false)
        }));
        if page == Page::Board {
            root.add_controller(task_drag_source());
        }
        self.heroes.borrow_mut().push(Hero {
            page,
            style,
            root: root.clone(),
        });
        root
    }

    fn build_heroes(self: &Rc<Self>, current: Option<&Task>) {
        let heroes: Vec<(Page, HeroStyle, gtk::Box)> = self
            .heroes
            .borrow()
            .iter()
            .map(|h| (h.page, h.style, h.root.clone()))
            .collect();
        for (page, style, root) in heroes {
            self.build_hero(page, style, &root, current);
        }
    }

    /// The one task you're doing right now: the loudest thing on the page.
    fn build_hero(
        self: &Rc<Self>,
        page: Page,
        style: HeroStyle,
        root: &gtk::Box,
        current: Option<&Task>,
    ) {
        while let Some(child) = root.first_child() {
            root.remove(&child);
        }

        let Some(task) = current else {
            root.add_css_class("empty");
            // Not a task, so not a stop for the keyboard either.
            root.set_widget_name("hero");
            root.set_cursor_from_name(None);
            root.set_focusable(false);
            // The board's header names the quadrant already.
            if style == HeroStyle::Banner {
                root.append(
                    &gtk::Label::builder()
                        .label(Bucket::Now.label())
                        .xalign(0.0)
                        .css_classes(["bucket-header"])
                        .build(),
                );
            }
            let empty = placeholder_label();
            let text = match style {
                HeroStyle::Banner => "empty — promote one ↑",
                HeroStyle::Card => "empty — drop a task here",
            };
            empty.set_label(text);
            // The last task's title was written here; it has to go with it.
            root.update_property(&[gtk::accessible::Property::Label(text)]);
            if style == HeroStyle::Card {
                empty.set_vexpand(true);
                empty.set_valign(gtk::Align::Center);
                empty.set_halign(gtk::Align::Center);
            }
            root.append(&empty);
            return;
        };
        root.remove_css_class("empty");
        let id = task.id;
        // The hero stands in for the current task's row: j/k reach it, and
        // `row_id` finds it, so d/t/r act on it like any other task.
        root.set_widget_name(&format!("task-{id}"));
        if page == Page::Board {
            root.set_cursor_from_name(Some("grab"));
        }
        root.set_focusable(true);
        root.update_property(&[gtk::accessible::Property::Label(&task.title)]);

        let top = gtk::Box::builder().spacing(8).build();
        if style == HeroStyle::Banner {
            top.append(
                &gtk::Label::builder()
                    .label(Bucket::Now.label())
                    .css_classes(["now-label"])
                    .build(),
            );
        }
        // The board keeps a chip standing even when there is no tag, so the
        // click that gives a task one is always in the same place.
        if task.tag.is_some() || style == HeroStyle::Card {
            top.append(&self.chip_button(id, task.tag));
        }
        top.append(&self.hero_timer(task.is_paused()));
        root.append(&top);

        let title = if self.renaming.get() == Some(id) && page == self.current_page() {
            let entry = self.rename_entry(id);
            // Keep the title's weight while editing so the hero does not jump.
            entry.add_css_class("rename-title");
            entry.upcast::<gtk::Widget>()
        } else {
            gtk::Label::builder()
                .label(&task.title)
                .xalign(0.0)
                .hexpand(true)
                .wrap(true)
                // Break inside words: a bare URL must not widen the window.
                .wrap_mode(pango::WrapMode::WordChar)
                .lines(if style == HeroStyle::Banner { 3 } else { -1 })
                .ellipsize(if style == HeroStyle::Banner {
                    pango::EllipsizeMode::End
                } else {
                    pango::EllipsizeMode::None
                })
                .css_classes(["current-title"])
                .build()
                .upcast()
        };
        let done = self.done_button(id, style);
        let menu = self.task_menu(id, true, Bucket::Now, &["flat", "hero-btn"]);

        match style {
            // The queue's band is one line of title with its buttons beside it.
            HeroStyle::Banner => {
                let row = gtk::Box::builder().spacing(8).build();
                row.append(&title);
                row.append(&done);
                row.append(&menu);
                root.append(&row);
            }
            // The board spells the title out large, over a line of actions.
            HeroStyle::Card => {
                // Keep all of a long title reachable without making the top
                // quadrant taller than the window and hiding Next/Later.
                let title_scroll = gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .min_content_height(60)
                    .max_content_height(196)
                    .propagate_natural_height(true)
                    .vexpand(true)
                    .child(&title)
                    .build();
                root.append(&title_scroll);
                menu.set_hexpand(true);
                menu.set_halign(gtk::Align::End);
                // 4px on top of the panel's 8px gap: the actions sit a little
                // further from the title than the title does from the clock.
                let row = gtk::Box::builder().spacing(6).margin_top(4).build();
                row.append(&done);
                row.append(&menu);
                root.append(&row);
            }
        }
    }

    /// The tag chip as a button: one click cycles the tag.
    fn chip_button(self: &Rc<Self>, id: u64, tag: Option<Tag>) -> gtk::Button {
        let button = gtk::Button::builder()
            .child(&chip_label(tag))
            .tooltip_text("Cycle tag (t)")
            .valign(gtk::Align::Center)
            .focusable(false)
            .css_classes(["flat", "chip-btn"])
            .build();
        button.update_property(&[gtk::accessible::Property::Label("Cycle tag")]);
        clickable(&button);
        let this = self.clone();
        button.connect_clicked(move |_| {
            let _ = this.update(|s| s.cycle_tag(id));
        });
        button
    }

    /// The clock. The pause glyph keeps its space so the time never shifts.
    fn hero_timer(self: &Rc<Self>, paused: bool) -> gtk::Button {
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
        let button = gtk::Button::builder()
            .child(&clock)
            .tooltip_text(if paused { "Resume (p)" } else { "Pause (p)" })
            .halign(gtk::Align::End)
            .hexpand(true)
            .focusable(false)
            .css_classes(classes)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(if paused {
            "Resume"
        } else {
            "Pause"
        })]);
        clickable(&button);
        let this = self.clone();
        button.connect_clicked(move |_| {
            let _ = this.update(|s| s.toggle_pause());
        });
        self.timers.borrow_mut().push(timer);
        button
    }

    /// Done: delete the current task and pull the next one up. The board has
    /// the room to say so in words.
    fn done_button(self: &Rc<Self>, id: u64, style: HeroStyle) -> gtk::Button {
        const TIP: &str = "Done — delete and pull the next task (d)";
        let this = self.clone();
        if style == HeroStyle::Banner {
            return icon_button(
                "object-select-symbolic",
                TIP,
                &["flat", "hero-btn"],
                move || {
                    this.complete(id);
                },
            );
        }
        let content = gtk::Box::builder().spacing(8).build();
        content.append(&gtk::Image::from_icon_name("object-select-symbolic"));
        content.append(&gtk::Label::new(Some("Done")));
        let button = gtk::Button::builder()
            .child(&content)
            .tooltip_text(TIP)
            .valign(gtk::Align::Center)
            .focusable(false)
            .css_classes(["hero-done"])
            .build();
        clickable(&button);
        button.connect_clicked(move |_| {
            this.complete(id);
        });
        button
    }

    fn build_row(
        self: &Rc<Self>,
        task: &Task,
        list: placement::List,
        style: RowStyle,
        leads: bool,
    ) -> gtk::ListBoxRow {
        let id = task.id;
        // Both pages hold a row per task, but only the visible one may host the
        // editor — otherwise rebuild() focuses a widget nobody can see.
        let renaming = self.renaming.get() == Some(id) && list.page == self.current_page();
        let mut classes = vec!["task-row"];
        if leads {
            // Whatever leads a section draws no hairline above it.
            classes.push("leads");
        }
        let row = gtk::ListBoxRow::builder()
            // Double-clicking inside the rename entry must not promote the row.
            .activatable(!renaming)
            .name(format!("task-{id}"))
            .css_classes(classes)
            .build();
        row.update_property(&[gtk::accessible::Property::Label(&task.title)]);
        // Every row can be picked up and dragged, and says so. GTK CSS has no
        // `cursor` property, so it is a per-widget call.
        row.set_cursor_from_name(Some("grab"));
        let content = gtk::Box::builder()
            .spacing(style.gap())
            .css_classes(["row-box"])
            .build();
        row.set_child(Some(&content));

        // Side's cards lead with the tag: they are the ones read out of order.
        if style == RowStyle::SideCard {
            if let Some(tag) = task.tag {
                content.append(&chip_label(Some(tag)));
            }
        }
        if renaming {
            content.append(&self.rename_entry(id));
        } else {
            let title = gtk::Label::builder()
                .label(&task.title)
                .xalign(0.0)
                .hexpand(true)
                .valign(gtk::Align::Center)
                .ellipsize(pango::EllipsizeMode::End)
                .css_classes(["row-title"])
                .build();
            if let Some(lines) = style.wrap_lines() {
                // Break inside words too, so one long URL cannot set the
                // quadrant's minimum width.
                title.set_wrap(true);
                title.set_wrap_mode(pango::WrapMode::WordChar);
                title.set_lines(lines);
            }
            content.append(&title);
        }
        if style != RowStyle::SideCard {
            if let Some(tag) = task.tag {
                content.append(&chip_label(Some(tag)));
            }
        }
        if style.inline_buttons() {
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
        content.append(&self.task_menu(id, false, list.bucket, &["flat", "row-btn"]));

        row.add_controller(task_drag_source());
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
            this.complete(id);
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
            .max_length(MAX_TITLE_CHARS as i32)
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
        self.refresh_countdown();
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
                gdk::Key::comma => self.toggle_settings(),
                gdk::Key::w | gdk::Key::q => self.hide(),
                _ => return glib::Propagation::Proceed,
            }
            return glib::Propagation::Stop;
        }
        if self.editable_focused() {
            return glib::Propagation::Proceed;
        }
        // Settings holds controls, not tasks. Letting the row keys through
        // here would complete or retag the current task from a page that
        // never mentions it.
        if self.current_page() == Page::Settings {
            match key {
                gdk::Key::Escape => self.hide(),
                gdk::Key::q => self.set_page(Page::Queue),
                gdk::Key::b => self.set_page(Page::Board),
                gdk::Key::question => self.toggle_shortcuts(),
                _ => return glib::Propagation::Proceed,
            }
            return glib::Propagation::Stop;
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
                    gdk::Key::d | gdk::Key::x | gdk::Key::Delete => Ok(self.complete(id)),
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
        // Wherever the settings page was reached from is where the gear goes
        // back to, whether that was the gear itself or a request over D-Bus.
        let from = self.current_page();
        if page == Page::Settings && from != Page::Settings {
            self.came_from.set(from);
        }
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
    fn visible_ids(&self) -> Vec<u64> {
        let later_open = self
            .later
            .borrow()
            .as_ref()
            .is_some_and(|(r, _)| r.reveals_child());
        self.rendered
            .borrow()
            .visible(self.current_page(), later_open)
    }

    fn visible_rows(&self) -> Vec<gtk::Widget> {
        let page = self.current_page();
        let mut widgets = std::collections::HashMap::new();
        if let Some(hero) = self.heroes.borrow().iter().find(|h| h.page == page) {
            if let Some(id) = row_id(&hero.root) {
                widgets.insert(id, hero.root.clone().upcast());
            }
        }
        for bl in self
            .lists
            .borrow()
            .iter()
            .filter(|bl| bl.placement.page == page)
        {
            let mut child = bl.list.first_child();
            while let Some(row) = child {
                if let Some(id) = row_id(&row) {
                    widgets.insert(id, row.clone());
                }
                child = row.next_sibling();
            }
        }
        self.visible_ids()
            .iter()
            .filter_map(|id| widgets.remove(id))
            .collect()
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

/// A task's tag, in one letter. An untagged one shows an outlined dash rather
/// than nothing, so the chip can be a control that is always there.
fn chip_label(tag: Option<Tag>) -> gtk::Label {
    gtk::Label::builder()
        .label(match tag {
            Some(Tag::Work) => "W",
            Some(Tag::Personal) => "P",
            None => "–",
        })
        .valign(gtk::Align::Center)
        .css_classes(match tag {
            Some(tag) => ["chip", tag.as_str()],
            None => ["chip", "untagged"],
        })
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

/// Read the task at pickup time: a hero survives rebuilds and may now show a
/// different task, or be empty. Asking the controller avoids a widget cycle.
fn task_drag_source() -> gtk::DragSource {
    let drag = gtk::DragSource::builder()
        .actions(gdk::DragAction::MOVE)
        .build();
    drag.connect_prepare(|source, _, _| {
        let id = row_id(&source.widget()?)?;
        Some(gdk::ContentProvider::for_value(&id.to_value()))
    });
    drag.connect_drag_begin(|source, _| {
        if let Some(widget) = source.widget() {
            source.set_icon(Some(&pick_up(&widget)), 0, 0);
        }
    });
    drag.connect_drag_end(|source, _, _| {
        if let Some(widget) = source.widget() {
            put_down(&widget);
        }
    });
    drag
}

/// Pick a row up: the icon the drag carries away, and the fade left on the row
/// it came from so the copy under the pointer is plainly the one being moved.
/// The order is the point — a live paintable would take the fade with it and
/// dim the card under the pointer too — so both happen here, together.
fn pick_up(widget: &gtk::Widget) -> gdk::Paintable {
    match still_picture(widget) {
        Some(picture) => {
            widget.add_css_class("dragging");
            picture.upcast()
        }
        // Nothing drawn yet to picture, so nothing to spoil by fading either.
        None => gtk::WidgetPaintable::new(Some(widget)).upcast(),
    }
}

/// Put a row back down, however the drag ended. A drop and a pointer leaving
/// clear the mark themselves, but a drag cancelled in place — Escape — emits
/// neither, and a line left pointing nowhere would outlive the drag.
fn put_down(widget: &gtk::Widget) {
    widget.remove_css_class("dragging");
    unmark();
}

/// A still picture of a widget, for a drag icon that has to keep looking like
/// the row did when it was picked up. None before the widget has been drawn.
fn still_picture(widget: &gtk::Widget) -> Option<gdk::Texture> {
    let renderer = widget.native()?.renderer()?;
    let snapshot = gtk::Snapshot::new();
    gtk::WidgetPaintable::new(Some(widget)).snapshot(
        &snapshot,
        widget.width() as f64,
        widget.height() as f64,
    );
    Some(renderer.render_texture(&snapshot.to_node()?, None))
}

/// Task rows carry their id in the widget name ("task-<id>").
fn row_id(row: &impl IsA<gtk::Widget>) -> Option<u64> {
    row.widget_name().strip_prefix("task-")?.parse().ok()
}

fn is_ctrl(mods: gdk::ModifierType) -> bool {
    mods.contains(gdk::ModifierType::CONTROL_MASK)
}

/// What a drop target draws while a drag is over it. GTK offers no feedback of
/// its own, so where the task would land has to be spelt out.
enum Highlight {
    /// Ring the target itself: the hero, or an empty bucket's placeholder.
    Ring,
    /// The target is a list: mark the row the task would land in front of.
    Before,
    /// The line after the last row the section shows — for the targets that
    /// append. A section can hold more than one list (Next holds the Now tail
    /// in front of its own rows), and which of them ends it depends on what is
    /// in them, so the choice is made while the drag is over it.
    End(Vec<glib::WeakRef<gtk::ListBox>>),
}

/// `Highlight::End` over every list under one header, in the order shown.
fn end_of(lists: &[gtk::ListBox]) -> Highlight {
    Highlight::End(lists.iter().map(|list| list.downgrade()).collect())
}

thread_local! {
    /// Where the drag is pointing right now. One drag means one mark, and
    /// clearing it has to reach whatever was marked last — which is not always
    /// something the target under the pointer can see. The Next quadrant holds
    /// two lists in one body, so its end line and a row's line belong to
    /// different widgets, and either can be the one left over.
    static MARKED: RefCell<Vec<(gtk::Widget, &'static str)>> = const { RefCell::new(Vec::new()) };
}

fn mark(widget: &impl IsA<gtk::Widget>, class: &'static str) {
    widget.add_css_class(class);
    MARKED.with(|marked| marked.borrow_mut().push((widget.clone().upcast(), class)));
}

fn unmark() {
    MARKED.with(|marked| {
        for (widget, class) in marked.borrow_mut().drain(..) {
            widget.remove_css_class(class);
        }
    });
}

impl Highlight {
    /// The list this highlight draws in, if it draws in one. `Before` never
    /// holds its list: the list owns the controller that owns this, and holding
    /// it back would be a cycle nothing could break.
    fn list(&self, target: &gtk::Widget) -> Option<gtk::ListBox> {
        match self {
            Highlight::Ring => None,
            Highlight::Before => target.downcast_ref::<gtk::ListBox>().cloned(),
            Highlight::End(lists) => {
                let shown: Vec<gtk::ListBox> =
                    lists.iter().filter_map(glib::WeakRef::upgrade).collect();
                // An empty list is zero pixels tall, so a line drawn on it
                // would not show at all.
                shown
                    .iter()
                    .rev()
                    .find(|list| list.first_child().is_some())
                    .cloned()
            }
        }
    }

    fn show(&self, target: &gtk::Widget, y: f64, dragged: Option<u64>) {
        unmark();
        let Some(list) = self.list(target) else {
            mark(target, "drop-into");
            return;
        };
        if matches!(self, Highlight::Before) {
            if let Some(row) = anchor_at(&list, y) {
                // Over the row it came from there is nothing to promise: the
                // drop is a no-op, so it gets the fade and no line.
                if row_id(&row) != dragged {
                    mark(&row, "drop-before");
                }
                return;
            }
        }
        // Anywhere else on a bucket: the task lands at the end.
        mark(&list, "drop-end");
    }
}

/// The row a drop at `y` would land in front of: the one under the pointer
/// while the pointer is in its top half, the one after it below that, and none
/// at all past the last row's middle — which means the end of the list.
fn anchor_at(list: &gtk::ListBox, y: f64) -> Option<gtk::ListBoxRow> {
    let row = list.row_at_y(y as i32)?;
    let Some(bounds) = row.compute_bounds(list) else {
        return Some(row);
    };
    if y < f64::from(bounds.y() + bounds.height() / 2.0) {
        return Some(row);
    }
    row.next_sibling().and_downcast::<gtk::ListBoxRow>()
}

/// A drop target accepting a dragged task id; `f(id, target_widget, y)` returns
/// whether the drop was handled.
fn drop_target(
    highlight: Highlight,
    f: impl Fn(u64, gtk::Widget, f64) -> bool + 'static,
) -> gtk::DropTarget {
    let target = gtk::DropTarget::new(u64::static_type(), gdk::DragAction::MOVE);
    // Read the id while the pointer is still moving, so the feedback can tell
    // which row the drag came from.
    target.set_preload(true);
    target.connect_motion(move |t, _x, y| {
        if let Some(w) = t.widget() {
            let dragged = t.value().and_then(|v| v.get::<u64>().ok());
            highlight.show(&w, y, dragged);
        }
        gdk::DragAction::MOVE
    });
    target.connect_leave(|_| unmark());
    target.connect_drop(
        move |t, value, _x, y| match (value.get::<u64>(), t.widget()) {
            (Ok(id), Some(w)) => {
                // A drop emits no leave of its own.
                unmark();
                f(id, w, y)
            }
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

    /// The page names are the words `Show(view)` and the command line accept,
    /// and the names the stack stores its children under. They have to survive
    /// the round trip, and an unknown view has to land somewhere sensible.
    #[test]
    fn page_names_round_trip_and_anything_else_opens_the_queue() {
        for page in [Page::Queue, Page::Board, Page::Settings] {
            assert_eq!(Page::parse(page.name()), page, "{}", page.name());
        }
        for unknown in ["", "Board", "queue ", "nonsense", "add", "toggle"] {
            assert_eq!(Page::parse(unknown), Page::Queue, "{unknown}");
        }
    }

    /// Which list a row is in decides how it is dressed, and two of the
    /// board's four quadrants look nothing like each other. Nothing else
    /// checks the mapping, so a swap here would silently reshape the page.
    #[test]
    fn row_style_dresses_each_bucket_for_the_page_it_is_on() {
        let of = |page, bucket| RowStyle::of(placement::List { page, bucket });
        // The queue is one card of one-line rows, whatever the bucket — except
        // the Later shelf, which is dim and offers a way back out.
        for bucket in [Bucket::Now, Bucket::Next, Bucket::Side] {
            assert_eq!(of(Page::Queue, bucket), RowStyle::Queue);
        }
        assert_eq!(of(Page::Queue, Bucket::Later), RowStyle::Later);
        // The board dresses every quadrant differently, and the Now tail wears
        // Next's clothes because Next is the header it is listed under.
        assert_eq!(of(Page::Board, Bucket::Side), RowStyle::SideCard);
        assert_eq!(of(Page::Board, Bucket::Next), RowStyle::BoardRow);
        assert_eq!(of(Page::Board, Bucket::Now), RowStyle::BoardRow);
        assert_eq!(of(Page::Board, Bucket::Later), RowStyle::BoardLater);

        for (style, lines, buttons, gap, class) in [
            (RowStyle::Queue, None, true, 8, "queue-rows"),
            (RowStyle::Later, None, true, 8, "later-rows"),
            (RowStyle::SideCard, Some(2), false, 10, "side-cards"),
            (RowStyle::BoardRow, Some(3), false, 8, "board-rows"),
            (RowStyle::BoardLater, None, false, 8, "board-later-rows"),
        ] {
            assert_eq!(style.wrap_lines(), lines, "{style:?} title lines");
            assert_eq!(style.inline_buttons(), buttons, "{style:?} inline buttons");
            assert_eq!(style.gap(), gap, "{style:?} gap");
            assert_eq!(style.css(), class, "{style:?} class");
            // Every style's class is its own, or the stylesheet crosses wires.
            assert_eq!(
                [
                    RowStyle::Queue,
                    RowStyle::Later,
                    RowStyle::SideCard,
                    RowStyle::BoardRow,
                    RowStyle::BoardLater,
                ]
                .into_iter()
                .filter(|other| other.css() == class)
                .count(),
                1,
                "{class} is shared"
            );
        }
    }

    #[test]
    fn elapsed_format() {
        assert_eq!(fmt_elapsed(0), "00:00");
        assert_eq!(fmt_elapsed(65), "01:05");
        assert_eq!(fmt_elapsed(762), "12:42");
        assert_eq!(fmt_elapsed(3600), "1:00:00");
        assert_eq!(fmt_elapsed(3725), "1:02:05");
    }
}
