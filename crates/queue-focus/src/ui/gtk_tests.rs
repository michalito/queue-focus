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

fn emit_drop(widget: &impl IsA<gtk::Widget>, id: u64, y: f64) {
    let controllers = widget.observe_controllers();
    let target = (0..controllers.n_items())
        .find_map(|i| controllers.item(i)?.downcast::<gtk::DropTarget>().ok())
        .unwrap();
    assert!(target.emit_by_name::<bool>("drop", &[&glib::BoxedValue(id.to_value()), &0.0f64, &y]));
    settle();
}

#[test]
#[ignore = "requires an isolated display; run make test-ui"]
fn placement_drives_real_widgets_drops_and_focus() {
    adw::init().unwrap();
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
        if page == Page::Queue {
            ids.extend(ui.banner.borrow().as_ref().and_then(row_id));
        }
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
    let banner = ui.banner.borrow().clone().unwrap();
    emit_drop(&banner, next, 0.0);
    assert_eq!(state.store().current().unwrap().id, next);
    ui.set_page(Page::Board);
    settle();
    assert_eq!(rendered_ids(), [next, current, side, later, tail]);
    ui.row_for(side).unwrap().grab_focus();
    ui.update(|s| s.remove(side)).unwrap();
    assert_eq!(rendered_ids(), [next, current, later, tail]);
    assert_eq!(ui.focused_row().as_ref().and_then(row_id), Some(later));
    ui.set_page(Page::Settings);
    assert!(rendered_ids().is_empty());
    ui.win.borrow().as_ref().unwrap().destroy();
    std::fs::remove_dir_all(dir).unwrap();
}
