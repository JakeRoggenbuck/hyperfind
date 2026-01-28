use strsim::jaro_winkler;
use gio::prelude::*;
use gtk::gdk;
use gtk::gdk::prelude::*;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Entry, ListBox};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct AppEntry {
    name: String,
    app_info: gio::AppInfo,
}


fn score_match(name: &str, query: &str) -> Option<i64> {
    let query = query.trim();
    if query.is_empty() {
        return Some(0);
    }

    let name_l = name.to_lowercase();
    let query_l = query.to_lowercase();

    if name_l.contains(&query_l) {
        let bonus = 1000i64;
        let penalty = (name_l.len() as i64 - query_l.len() as i64).max(0);
        return Some(bonus - penalty);
    }

    let score = jaro_winkler(&name_l, &query_l);
    if score < 0.75 {
        return None;
    }

    Some((score * 1000.0) as i64)
}

fn load_apps() -> Vec<AppEntry> {
    let mut apps: Vec<AppEntry> = gio::AppInfo::all()
        .into_iter()
        .filter(|app| app.should_show())
        .filter_map(|app| {
            let name = app.display_name().to_string();
            if name.trim().is_empty() {
                None
            } else {
                Some(AppEntry { name, app_info: app })
            }
        })
        .collect();

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

fn update_results(
    listbox: &ListBox,
    apps: &[AppEntry],
    query: &str,
    results: &Rc<RefCell<Vec<AppEntry>>>,
) {
    for child in listbox.children() {
        listbox.remove(&child);
    }

    let mut scored: Vec<(i64, &AppEntry)> = apps
        .iter()
        .filter_map(|app| score_match(&app.name, query).map(|score| (score, app)))
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));

    let mut results_mut = results.borrow_mut();
    results_mut.clear();

    for (_, app) in scored.into_iter().take(10) {
        results_mut.push(app.clone());
        let row = gtk::ListBoxRow::new();
        let label = gtk::Label::new(Some(&app.name));
        label.set_xalign(0.0);
        row.add(&label);
        listbox.add(&row);
    }

    listbox.show_all();
    if let Some(row) = listbox.row_at_index(0) {
        listbox.select_row(Some(&row));
    }
}

fn launch_from_index(index: i32, results: &Rc<RefCell<Vec<AppEntry>>>) -> bool {
    if index < 0 {
        return false;
    }

    let results = results.borrow();
    let index = index as usize;
    let Some(app) = results.get(index) else {
        return false;
    };

    if let Err(err) = app
        .app_info
        .launch(&[], Option::<&gio::AppLaunchContext>::None)
    {
        eprintln!("Failed to launch {}: {}", app.name, err);
        return false;
    }

    true
}

fn main() {
    let app = Application::builder()
        .application_id("com.example.hyperfind")
        .build();

    app.connect_activate(|app| {
        if let Some(settings) = gtk::Settings::default() {
            settings.set_property("gtk-error-bell", &false);
        }

        let entry = Entry::builder()
            .placeholder_text("Search…")
            .build();

        let apps = Rc::new(load_apps());
        let results = Rc::new(RefCell::new(Vec::new()));

        let listbox = ListBox::new();
        listbox.set_selection_mode(gtk::SelectionMode::Single);

        let listbox_for_activate = listbox.clone();
        let results_for_activate = Rc::clone(&results);
        let app_for_activate = app.clone();
        listbox.connect_row_activated(move |_, row| {
            if launch_from_index(row.index(), &results_for_activate) {
                app_for_activate.quit();
            }
        });

        let entry_for_keys = entry.clone();
        let listbox_for_keys = listbox.clone();
        let results_for_keys = Rc::clone(&results);
        let app_for_keys = app.clone();
        entry.connect_key_press_event(move |_, event| {
            let key = event.keyval();
            if key == gdk::keys::constants::Escape {
                app_for_keys.quit();
                return gtk::glib::Propagation::Stop;
            }
            if key == gdk::keys::constants::Return || key == gdk::keys::constants::KP_Enter {
                let row = listbox_for_keys
                    .selected_row()
                    .or_else(|| listbox_for_keys.row_at_index(0));
                if let Some(row) = row {
                    if launch_from_index(row.index(), &results_for_keys) {
                        app_for_keys.quit();
                    }
                }
                entry_for_keys.grab_focus();
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });

        let listbox_for_change = listbox.clone();
        let apps_for_change = Rc::clone(&apps);
        let results_for_change = Rc::clone(&results);
        entry.connect_changed(move |entry| {
            let query = entry.text().to_string();
            update_results(&listbox_for_change, &apps_for_change, &query, &results_for_change);
        });

        let container = gtk::Box::new(gtk::Orientation::Vertical, 6);
        container.set_margin_top(8);
        container.set_margin_bottom(8);
        container.set_margin_start(10);
        container.set_margin_end(10);
        container.pack_start(&entry, false, false, 0);
        container.pack_start(&listbox, true, true, 0);

        let window = ApplicationWindow::builder()
            .application(app)
            .decorated(false)
            .default_width(600)
            .default_height(320)
            .resizable(false)
            .build();

        window.set_position(gtk::WindowPosition::Center);

        window.add(&container);

        window.set_type_hint(gdk::WindowTypeHint::PopupMenu);
        window.set_skip_taskbar_hint(true);
        window.set_skip_pager_hint(true);
        window.set_accept_focus(true);
        window.set_focus_on_map(true);
        window.connect_realize(|window| {
            if let Some(gdk_window) = window.window() {
                gdk_window.set_override_redirect(true);
            }
        });
        window.connect_map_event(|window, _| {
            if let Some(gdk_window) = window.window() {
                gdk_window.focus(gdk::ffi::GDK_CURRENT_TIME as u32);
                if let Some(display) = gdk::Display::default() {
                    if let Some(seat) = display.default_seat() {
                        let _ = seat.grab(
                            &gdk_window,
                            gdk::SeatCapabilities::KEYBOARD,
                            true,
                            None,
                            None,
                            None,
                        );
                    }
                }
            }
            gtk::glib::Propagation::Proceed
        });
        window.connect_unmap_event(|_, _| {
            if let Some(display) = gdk::Display::default() {
                if let Some(seat) = display.default_seat() {
                    seat.ungrab();
                }
            }
            gtk::glib::Propagation::Proceed
        });

        update_results(&listbox, &apps, "", &results);

        window.show_all();

        let entry_clone = entry.clone();
        gtk::glib::idle_add_local_once(move || {
            entry_clone.grab_focus();
        });
    });

    app.run();
}
