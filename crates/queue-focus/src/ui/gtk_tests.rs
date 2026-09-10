//! Run with `make test-ui`: a real GTK window on a private X display.
use super::*;

fn settle() {
    let context = glib::MainContext::default();
    let until = std::time::Instant::now() + std::time::Duration::from_millis(200);
    while std::time::Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn drop_target_of(widget: &impl IsA<gtk::Widget>) -> gtk::DropTarget {
    let controllers = widget.observe_controllers();
    (0..controllers.n_items())
        .find_map(|i| controllers.item(i)?.downcast::<gtk::DropTarget>().ok())
        .unwrap()
}

fn emit_drop(widget: &impl IsA<gtk::Widget>, id: u64, y: f64) {
    let target = drop_target_of(widget);
    assert!(target.emit_by_name::<bool>("drop", &[&glib::BoxedValue(id.to_value()), &0.0f64, &y]));
    settle();
}

fn drag_payload(widget: &impl IsA<gtk::Widget>) -> Option<u64> {
    let controllers = widget.observe_controllers();
    let source = (0..controllers.n_items())
        .find_map(|i| controllers.item(i)?.downcast::<gtk::DragSource>().ok())
        .expect("task widget has a drag source");
    source
        .emit_by_name::<Option<gdk::ContentProvider>>("prepare", &[&0.0f64, &0.0f64])
        .map(|provider| {
            provider
                .value(u64::static_type())
                .unwrap()
                .get::<u64>()
                .unwrap()
        })
}

/// Drive the pointer over a drop target the way a real drag would.
fn emit_motion(widget: &impl IsA<gtk::Widget>, y: f64) {
    drop_target_of(widget).emit_by_name::<gdk::DragAction>("motion", &[&0.0f64, &y]);
}

fn emit_leave(widget: &impl IsA<gtk::Widget>) {
    drop_target_of(widget).emit_by_name::<()>("leave", &[]);
}

/// GTK may only be initialised from one thread, so the whole of this module is
/// one test: everything it drives needs a real window.
#[test]
#[ignore = "requires an isolated display; run make test-ui"]
fn real_widgets_drive_placement_drops_focus_and_drag_feedback() {
    adw::init().unwrap();
    placement_drops_and_focus();
    drag_feedback();
    hero_dressing();
}

/// Every widget under `root`, depth first.
fn descendants(root: &impl IsA<gtk::Widget>) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut stack = vec![root.clone().upcast::<gtk::Widget>()];
    while let Some(widget) = stack.pop() {
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            stack.push(next.clone());
            found.push(next);
        }
    }
    found
}

fn has_label(root: &impl IsA<gtk::Widget>, text: &str) -> bool {
    descendants(root)
        .into_iter()
        .any(|w| w.downcast::<gtk::Label>().is_ok_and(|l| l.label() == text))
}

fn has_class(root: &impl IsA<gtk::Widget>, class: &str) -> bool {
    descendants(root)
        .into_iter()
        .any(|w| w.has_css_class(class))
}

fn find_class(root: &impl IsA<gtk::Widget>, class: &str) -> Option<gtk::Widget> {
    descendants(root)
        .into_iter()
        .find(|w| w.has_css_class(class))
}

fn hero_of(ui: &Rc<Ui>, page: Page) -> gtk::Box {
    ui.heroes
        .borrow()
        .iter()
        .find(|h| h.page == page)
        .unwrap()
        .root
        .clone()
}

fn placement_drops_and_focus() {
    let dir = std::env::temp_dir().join(format!("qf-placement-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = crate::state::State::load_from(dir.join("tasks.json")).unwrap();
    let (settings, _) = crate::settings::SettingsStore::load_from(dir.join("settings.json"));
    let mut ids = Vec::new();
    state
        .update(|s| {
            for (name, bucket) in [
                ("current", Bucket::Now),
                ("tail", Bucket::Now),
                ("next", Bucket::Next),
                ("side", Bucket::Side),
                ("later", Bucket::Later),
            ] {
                ids.push(s.add(name, bucket, None, false));
            }
        })
        .unwrap();
    let [current, tail, next, side, later] = ids.try_into().unwrap();
    let app = adw::Application::builder()
        .application_id("org.queuefocus.PlacementTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();
    let flash = crate::flash::FlashClock::new(state.clone(), settings.clone());
    let ui = Ui::new(app, state.clone(), settings, flash);
    ui.show(Page::Queue);
    settle();
    // Inspect GTK's actual row order, independently of Placement::visible.
    let rendered_ids = || {
        let mut ids = Vec::new();
        let page = ui.current_page();
        // Both pages lead with a hero holding the head of Now.
        ids.extend(
            ui.heroes
                .borrow()
                .iter()
                .find(|h| h.page == page)
                .and_then(|h| row_id(&h.root)),
        );
        for list in ui
            .lists
            .borrow()
            .iter()
            .filter(|l| l.placement.page == page)
        {
            if list.style == RowStyle::Later
                && !ui.later.borrow().as_ref().unwrap().0.reveals_child()
            {
                continue;
            }
            let mut child = list.list.first_child();
            while let Some(row) = child {
                ids.push(row_id(&row).unwrap());
                child = row.next_sibling();
            }
        }
        ids
    };
    assert_eq!(rendered_ids(), [current, side, tail, next]);
    let section = ui
        .sections
        .borrow()
        .iter()
        .find(|s| s.placement.page == Page::Queue && s.placement.bucket == Bucket::Next)
        .unwrap()
        .count
        .clone();
    assert_eq!(section.text(), "2");
    ui.row_for(next).unwrap().grab_focus();
    let tail_list = ui
        .lists
        .borrow()
        .iter()
        .find(|l| {
            l.placement
                == placement::List {
                    page: Page::Queue,
                    bucket: Bucket::Now,
                }
        })
        .unwrap()
        .list
        .clone();
    assert_eq!(row_id(&tail_list.row_at_y(1).unwrap()), Some(tail));
    emit_drop(&tail_list, next, 1.0);
    assert_eq!(state.store().current().unwrap().id, current);
    assert_eq!(state.store().get(next).unwrap().bucket, Bucket::Now);
    assert_eq!(rendered_ids(), [current, side, next, tail]);
    assert_eq!(ui.focused_row().as_ref().and_then(row_id), Some(next));
    // The actual Next header controller appends to Next, not the Now tail.
    let next_header = ui
        .sections
        .borrow()
        .iter()
        .find(|s| s.placement.page == Page::Queue && s.placement.bucket == Bucket::Next)
        .unwrap()
        .count
        .parent()
        .unwrap();
    emit_drop(&next_header, next, 0.0);
    assert_eq!(state.store().get(next).unwrap().bucket, Bucket::Next);
    ui.row_for(tail).unwrap().grab_focus();
    ui.update(|s| s.move_to(tail, Bucket::Later, None)).unwrap();
    assert_eq!(ui.focused_row().as_ref().and_then(row_id), Some(next));
    ui.set_later_open(true);
    settle();
    assert_eq!(rendered_ids(), [current, side, next, later, tail]);
    let hero = |page: Page| {
        ui.heroes
            .borrow()
            .iter()
            .find(|h| h.page == page)
            .unwrap()
            .root
            .clone()
    };
    emit_drop(&hero(Page::Queue), next, 0.0);
    assert_eq!(state.store().current().unwrap().id, next);
    ui.set_page(Page::Board);
    settle();
    // Now=[next, current]: the hero shows the head and the rest of the bucket
    // is listed under Next, ahead of Next's own rows. Later needs no opening.
    assert_eq!(rendered_ids(), [next, side, current, later, tail]);
    let board_section = |bucket: Bucket| {
        ui.sections
            .borrow()
            .iter()
            .find(|s| s.placement.page == Page::Board && s.placement.bucket == bucket)
            .unwrap()
            .count
            .clone()
    };
    // The hero counts the one task on show; the tail counts under Next.
    assert_eq!(board_section(Bucket::Now).text(), "1");
    assert_eq!(board_section(Bucket::Next).text(), "1");
    assert_eq!(board_section(Bucket::Later).text(), "2");
    let board_list = |bucket: Bucket| {
        ui.lists
            .borrow()
            .iter()
            .find(|l| {
                l.placement
                    == placement::List {
                        page: Page::Board,
                        bucket,
                    }
            })
            .unwrap()
            .list
            .clone()
    };
    // The redesign's core claim, at the widget level: the head of Now is the
    // hero's, and the rest of the bucket is listed under the Next header — in
    // Next's own body, ahead of Next's own rows, wearing Next's clothes.
    assert_eq!(
        board_list(Bucket::Now).parent(),
        board_list(Bucket::Next).parent(),
        "the Now tail belongs under the Next header"
    );
    assert_ne!(
        board_list(Bucket::Now).parent(),
        board_list(Bucket::Side).parent()
    );
    assert!(board_list(Bucket::Now).has_css_class(RowStyle::BoardRow.css()));
    assert!(board_list(Bucket::Side).has_css_class(RowStyle::SideCard.css()));
    assert!(board_list(Bucket::Later).has_css_class(RowStyle::BoardLater.css()));
    // The Now quadrant has no "empty" line of its own — the hero carries one.
    assert!(ui
        .sections
        .borrow()
        .iter()
        .find(|s| s.placement.page == Page::Board && s.placement.bucket == Bucket::Now)
        .unwrap()
        .placeholder
        .is_none());
    // Dropping on the board's Next header appends to Next, not to the Now tail.
    emit_drop(&board_section(Bucket::Next).parent().unwrap(), tail, 0.0);
    assert_eq!(state.store().get(tail).unwrap().bucket, Bucket::Next);
    assert_eq!(rendered_ids(), [next, side, current, tail, later]);
    // Dropping on the board's hero takes over as the current task.
    emit_drop(&hero(Page::Board), tail, 0.0);
    assert_eq!(state.store().current().unwrap().id, tail);
    assert_eq!(rendered_ids(), [tail, side, next, current, later]);
    // The Now heading queues behind the current task, unlike the hero.
    let now_header = board_section(Bucket::Now).parent().unwrap();
    emit_drop(&now_header, side, 0.0);
    assert_eq!(state.store().current().unwrap().id, tail);
    assert_eq!(rendered_ids(), [tail, next, current, side, later]);
    assert_eq!(board_section(Bucket::Now).text(), "1");
    assert_eq!(board_section(Bucket::Next).text(), "3");
    ui.update(|s| s.move_to(side, Bucket::Side, None)).unwrap();
    settle();
    ui.row_for(side).unwrap().grab_focus();
    ui.update(|s| s.remove(side)).unwrap();
    assert_eq!(rendered_ids(), [tail, next, current, later]);
    // Side led the row after the hero, so focus falls to whatever took its place.
    assert_eq!(ui.focused_row().as_ref().and_then(row_id), Some(next));
    ui.set_page(Page::Settings);
    assert!(rendered_ids().is_empty());
    ui.win.borrow().as_ref().unwrap().destroy();
    std::fs::remove_dir_all(dir).unwrap();
}

/// The board's panel and the queue's band are one hero in two sets of clothes,
/// and the clothes are the point of the redesign.
fn hero_dressing() {
    let dir = std::env::temp_dir().join(format!("qf-hero-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = crate::state::State::load_from(dir.join("tasks.json")).unwrap();
    let (settings, _) = crate::settings::SettingsStore::load_from(dir.join("settings.json"));
    state
        .update(|s| {
            s.add("untagged current", Bucket::Now, None, false);
            s.add("queued", Bucket::Next, None, false);
        })
        .unwrap();
    let app = adw::Application::builder()
        .application_id("org.queuefocus.HeroTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();
    let flash = crate::flash::FlashClock::new(state.clone(), settings.clone());
    let ui = Ui::new(app, state.clone(), settings, flash);
    ui.show(Page::Board);
    settle();
    let (band, card) = (hero_of(&ui, Page::Queue), hero_of(&ui, Page::Board));

    // Both stand in for the current task's row, so the keyboard reaches either.
    assert!(row_id(&card).is_some());
    assert_eq!(row_id(&band), row_id(&card));
    // The board's quadrant header already says "Now"; the card must not repeat it.
    assert!(has_label(&band, Bucket::Now.label()));
    assert!(!has_label(&card, Bucket::Now.label()));
    // The card has the room to spell the action out; the band takes an icon.
    // (Both heroes' menus carry a "Done" item, so this asks about the button.)
    let done = find_class(&card, "hero-done").expect("the card spells Done out");
    assert!(has_label(&done, "Done"));
    assert!(find_class(&band, "hero-done").is_none());
    // The card keeps a chip standing even with no tag to show, so the click
    // that gives a task one is always in the same place.
    assert!(has_class(&card, "untagged"));
    assert!(!has_class(&band, "chip"));
    // Both carry the clock, and both are pausable.
    for hero in [&band, &card] {
        assert!(has_class(hero, "timer"));
        assert!(has_class(hero, "timer-btn"));
    }

    // Board titles remain complete even when the Queue banner truncates them.
    let long_title = "W".repeat(256);
    let current_id = state.store().current().unwrap().id;
    ui.update(|s| s.rename(current_id, &long_title)).unwrap();
    settle();
    let title = find_class(&card, "current-title")
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    assert_eq!(title.text(), long_title);
    assert_eq!(title.ellipsize(), pango::EllipsizeMode::None);
    assert!(!title.layout().is_ellipsized());
    assert!(title.layout().line_count() > 3);
    assert!(card.height() < ui.win.borrow().as_ref().unwrap().height());
    let scroll = title
        .ancestor(gtk::ScrolledWindow::static_type())
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    assert!(scroll.vadjustment().upper() > scroll.vadjustment().page_size());

    // A Board hero supplies the current task, including after a promotion.
    let current = state.store().current().unwrap().id;
    assert_eq!(drag_payload(&card), Some(current));
    let queued = state.store().in_bucket(Bucket::Next).next().unwrap().id;
    ui.update(|s| s.promote(queued)).unwrap();
    settle();
    assert_eq!(drag_payload(&card), Some(queued));
    let side_header = ui
        .sections
        .borrow()
        .iter()
        .find(|s| s.placement.page == Page::Board && s.placement.bucket == Bucket::Side)
        .unwrap()
        .count
        .parent()
        .unwrap();
    emit_drop(&side_header, drag_payload(&card).unwrap(), 0.0);
    assert_eq!(state.store().get(queued).unwrap().bucket, Bucket::Side);
    assert_eq!(drag_payload(&card), Some(current));

    // Emptied, each says how to fill itself and refuses to start a drag.
    ui.update(|s| s.remove(current)).unwrap();
    settle();
    assert!(row_id(&card).is_none());
    assert_eq!(drag_payload(&card), None);
    for hero in [&band, &card] {
        assert!(hero.has_css_class("empty"));
        assert!(!hero.is_focusable());
    }
    assert!(has_label(&card, "empty — drop a task here"));
    assert!(has_label(&band, "empty — promote one ↑"));

    ui.win.borrow().as_ref().unwrap().destroy();
    std::fs::remove_dir_all(dir).unwrap();
}

/// GTK draws no drag feedback of its own, so the board draws it: a line where
/// the task would land, a ring on the hero, and a faded origin. None of it may
/// be left behind when the pointer moves on.
fn drag_feedback() {
    // The feedback is drawn by the stylesheet, so this half needs it installed.
    load_css();
    let dir = std::env::temp_dir().join(format!("qf-drag-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = crate::state::State::load_from(dir.join("tasks.json")).unwrap();
    let (settings, _) = crate::settings::SettingsStore::load_from(dir.join("settings.json"));
    state
        .update(|s| {
            s.add("current", Bucket::Now, None, false);
            for bucket in [Bucket::Next, Bucket::Later] {
                for name in ["first", "second"] {
                    s.add(name, bucket, None, false);
                }
            }
        })
        .unwrap();
    let app = adw::Application::builder()
        .application_id("org.queuefocus.DragTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gtk::gio::Cancellable>).unwrap();
    let flash = crate::flash::FlashClock::new(state.clone(), settings.clone());
    let ui = Ui::new(app, state.clone(), settings, flash);
    ui.show(Page::Board);
    settle();

    let list = |bucket: Bucket| {
        ui.lists
            .borrow()
            .iter()
            .find(|l| {
                l.placement
                    == placement::List {
                        page: Page::Board,
                        bucket,
                    }
            })
            .unwrap()
            .list
            .clone()
    };
    let next = list(Bucket::Next);
    let (first, second) = (next.row_at_index(0).unwrap(), next.row_at_index(1).unwrap());
    let band = |row: &gtk::ListBoxRow| {
        let bounds = row.compute_bounds(&next).unwrap();
        let (top, height) = (f64::from(bounds.y()), f64::from(bounds.height()));
        (top + 2.0, top + height - 2.0)
    };
    let (first_top, first_bottom) = band(&first);
    let (_, second_bottom) = band(&second);
    let now_header = ui
        .sections
        .borrow()
        .iter()
        .find(|s| s.placement.page == Page::Board && s.placement.bucket == Bucket::Now)
        .unwrap()
        .count
        .parent()
        .unwrap();
    // With no Now tail, appending to Now lands before the first Next row.
    emit_motion(&now_header, 0.0);
    assert!(first.has_css_class("drop-before"));
    assert!(!now_header.has_css_class("drop-into"));
    emit_leave(&now_header);

    // A row's top half means "in front of this one"; its bottom half means the
    // one after it. The line follows the pointer rather than piling up behind.
    emit_motion(&next, first_top);
    assert!(first.has_css_class("drop-before"));
    emit_motion(&next, first_bottom);
    assert!(!first.has_css_class("drop-before"));
    assert!(
        second.has_css_class("drop-before"),
        "a row's bottom half points at the row after it"
    );
    emit_leave(&next);
    assert!(!second.has_css_class("drop-before"));

    // Past the last row's middle there is no row left to land in front of, so
    // the line goes to the end — and so does the drop.
    emit_motion(&next, second_bottom);
    assert!(!second.has_css_class("drop-before"));
    assert!(
        next.has_css_class("drop-end"),
        "below the last row's middle is the end of the bucket"
    );
    // Over the row a drag came from there is nothing to promise: it already
    // carries the fade, and dropping on itself does nothing. Knowing which row
    // that is needs the id read while the pointer is still moving.
    assert!(
        drop_target_of(&next).is_preload(),
        "without preload, motion cannot tell which row the drag came from"
    );
    Highlight::Before.show(next.upcast_ref(), first_top, row_id(&first));
    assert!(!first.has_css_class("drop-before"));
    unmark();

    // The space under the last row is still the bucket: the line goes at the
    // end, and dropping there appends. Two lists share this body, so the mark
    // has to be cleared wherever it was last put, not only on the target.
    let body = next.parent().unwrap();
    let space = find_class(&body, "board-drop-space").unwrap();
    // GTK bubbles drag motion: no ancestor of a row target may also append,
    // or its feedback would replace the insertion line after the row draws it.
    let mut ancestor = next.parent();
    while let Some(widget) = ancestor {
        let controllers = widget.observe_controllers();
        assert!(!(0..controllers.n_items()).any(|i| controllers
            .item(i)
            .is_some_and(|c| c.is::<gtk::DropTarget>())));
        ancestor = widget.parent();
    }
    assert_eq!(space.parent(), next.parent());
    emit_motion(&space, 0.0);
    assert!(next.has_css_class("drop-end"));
    emit_motion(&next, first_top);
    assert!(
        !next.has_css_class("drop-end"),
        "the end line must go when the pointer moves to a row"
    );
    assert!(first.has_css_class("drop-before"));
    emit_leave(&next);
    let later_second = list(Bucket::Later)
        .row_at_index(1)
        .and_then(|r| row_id(&r))
        .unwrap();
    let order = || {
        state
            .store()
            .in_bucket(Bucket::Next)
            .map(|t| t.id)
            .collect::<Vec<_>>()
    };
    let queued = order();
    emit_drop(&space, later_second, 0.0);
    assert_eq!(
        order(),
        [queued.as_slice(), &[later_second]].concat(),
        "a drop under the last row appends"
    );
    // And a drop lands where the line promised: a row's bottom half means
    // after that row, not in front of it. The rows above were destroyed by
    // that drop's rebuild, so this measures the list as it stands now.
    let fresh = list(Bucket::Next);
    let head = fresh.row_at_index(0).unwrap();
    let head_id = row_id(&head).unwrap();
    let head_bounds = head.compute_bounds(&fresh).unwrap();
    let head_bottom = f64::from(head_bounds.y() + head_bounds.height()) - 2.0;
    emit_drop(&fresh, later_second, head_bottom);
    assert_eq!(
        order()[..2],
        [head_id, later_second],
        "a drop in a row's bottom half lands after it, where the line was"
    );

    // The hero rings as a whole: dropping on it takes over as current.
    let hero = ui
        .heroes
        .borrow()
        .iter()
        .find(|h| h.page == Page::Board)
        .unwrap()
        .root
        .clone();
    emit_motion(&hero, 0.0);
    assert!(hero.has_css_class("drop-into"));
    emit_leave(&hero);
    assert!(!hero.has_css_class("drop-into"));

    // Picking a row up has to picture it before fading it, or the icon under
    // the pointer — a live paintable of the same row — fades with its origin.
    let row = next.row_at_index(0).unwrap();
    let bright = |paintable: &gdk::Paintable| {
        let texture = paintable
            .clone()
            .downcast::<gdk::Texture>()
            .expect("a still picture, not a live paintable of the fading row");
        let mut pixels = vec![0u8; texture.width() as usize * texture.height() as usize * 4];
        texture.download(&mut pixels, texture.width() as usize * 4);
        pixels.iter().map(|b| u64::from(*b)).sum::<u64>()
    };
    let unfaded = bright(&still_picture(row.upcast_ref()).unwrap().upcast());
    let icon = pick_up(row.upcast_ref());
    assert!(row.has_css_class("dragging"), "the row it came from fades");
    assert_eq!(
        bright(&icon),
        unfaded,
        "the icon is the row as it was before the fade"
    );
    settle();
    assert!(
        bright(&still_picture(row.upcast_ref()).unwrap().upcast()) < unfaded,
        "and the fade is real"
    );
    // Putting it down again takes the fade and the line with it, whether or not
    // the pointer ever left a target — a drag cancelled in place emits no leave.
    emit_motion(&next, first_top);
    assert!(row.has_css_class("drop-before"));
    put_down(row.upcast_ref());
    assert!(!row.has_css_class("dragging"));
    assert!(
        !row.has_css_class("drop-before"),
        "no line outlives the drag"
    );

    // A section's end line goes after the last row it actually shows. With a
    // task queued behind the current one and nothing in Next, that row belongs
    // to the Now tail — and an empty list is zero pixels tall, so a line drawn
    // on Next would show nothing at all.
    let tail = ui
        .update(|s| {
            let queued: Vec<u64> = s.in_bucket(Bucket::Next).map(|t| t.id).collect();
            for id in queued {
                s.move_to(id, Bucket::Later, None);
            }
            s.add("queued behind", Bucket::Now, None, false)
        })
        .unwrap();
    settle();
    let tail_list = list(Bucket::Now);
    assert_eq!(
        tail_list.row_at_index(0).and_then(|r| row_id(&r)),
        Some(tail)
    );
    assert!(
        list(Bucket::Next).row_at_index(0).is_none(),
        "Next is empty"
    );
    emit_motion(&space, 0.0);
    assert!(
        tail_list.has_css_class("drop-end"),
        "the line goes after the last row the section shows"
    );
    assert!(!list(Bucket::Next).has_css_class("drop-end"));
    emit_leave(&space);
    emit_motion(&now_header, 0.0);
    assert!(tail_list.has_css_class("drop-end"));
    assert!(!now_header.has_css_class("drop-into"));
    emit_leave(&now_header);
    ui.update(|s| s.move_to(tail, Bucket::Later, None)).unwrap();
    settle();
    emit_motion(&space, 0.0);
    assert!(
        space.has_css_class("drop-into"),
        "an empty target still gives feedback"
    );
    emit_leave(&space);
    let section = ui
        .sections
        .borrow()
        .iter()
        .find(|s| s.placement.page == Page::Board && s.placement.bucket == Bucket::Next)
        .unwrap()
        .count
        .parent()
        .unwrap();
    emit_motion(&section, 0.0);
    assert!(section.has_css_class("drop-into"));
    emit_leave(&section);
    emit_motion(&now_header, 0.0);
    assert!(now_header.has_css_class("drop-into"));
    emit_leave(&now_header);

    ui.win.borrow().as_ref().unwrap().destroy();
    std::fs::remove_dir_all(dir).unwrap();
}
