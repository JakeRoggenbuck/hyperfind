use strsim::jaro_winkler;
use gio::prelude::*;
use gtk::gdk;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use gtk::gdk::prelude::*;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Entry, ListBox};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone)]
struct AppEntry {
    key: String,
    name: String,
    icon: Option<gio::Icon>,
    app_info: gio::AppInfo,
}

#[derive(Clone, Deserialize, Serialize)]
struct UsageEntry {
    count: u64,
    last_used: u64,
}

type UsageMap = HashMap<String, UsageEntry>;

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn usage_path() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("hyperfind")
            .join("usage.json")
    })
}

fn load_usage() -> UsageMap {
    let Some(path) = usage_path() else {
        return HashMap::new();
    };

    let Ok(contents) = fs::read_to_string(path) else {
        return HashMap::new();
    };

    serde_json::from_str(&contents).unwrap_or_default()
}

fn save_usage(usage: &UsageMap) {
    let Some(path) = usage_path() else {
        return;
    };

    if let Some(parent) = path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            eprintln!("Failed to create usage dir: {}", err);
            return;
        }
    }

    let Ok(payload) = serde_json::to_string(usage) else {
        return;
    };

    if let Err(err) = fs::write(path, payload) {
        eprintln!("Failed to save usage data: {}", err);
    }
}

fn record_usage(key: &str, usage: &mut UsageMap) {
    let entry = usage.entry(key.to_string()).or_insert(UsageEntry {
        count: 0,
        last_used: 0,
    });
    entry.count = entry.count.saturating_add(1);
    entry.last_used = now_unix();
}

fn usage_key(app: &gio::AppInfo, name: &str) -> String {
    app.id().map(|id| id.to_string()).unwrap_or_else(|| name.to_string())
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
                let icon = app.icon();
                let key = usage_key(&app, &name);
                Some(AppEntry { key, name, icon, app_info: app })
            }
        })
        .collect();

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

fn clear_listbox(listbox: &ListBox) {
    for child in listbox.children() {
        listbox.remove(&child);
    }
}

fn usage_label_text(app: &AppEntry, usage: &UsageMap, show_usage: bool) -> String {
    if !show_usage {
        return app.name.clone();
    }

    if let Some(entry) = usage.get(&app.key) {
        format!("{}  ({} uses)", app.name, entry.count)
    } else {
        format!("{}  (0 uses)", app.name)
    }
}

fn build_result_row(app: &AppEntry, usage: &UsageMap, show_usage: bool) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if let Some(icon) = &app.icon {
        let image = gtk::Image::from_gicon(icon, gtk::IconSize::Menu);
        image.set_pixel_size(20);
        row_box.pack_start(&image, false, false, 0);
    }
    let label_text = usage_label_text(app, usage, show_usage);
    let label = gtk::Label::new(Some(&label_text));
    label.set_xalign(0.0);
    row_box.pack_start(&label, true, true, 0);
    row.add(&row_box);
    row
}

fn score_apps<'a>(
    apps: &'a [AppEntry],
    query: &str,
    usage: &UsageMap,
) -> Vec<(i64, &'a AppEntry)> {
    if query.trim().is_empty() {
        return apps
            .iter()
            .filter_map(|app| {
                usage.get(&app.key).map(|entry| {
                    let score = (entry.count as i64 * 1000) + entry.last_used as i64;
                    (score, app)
                })
            })
            .collect();
    }

    apps.iter()
        .filter_map(|app| {
            let mut score = score_match(&app.name, query)?;
            if let Some(entry) = usage.get(&app.key) {
                score += entry.count as i64 * 10;
            }
            Some((score, app))
        })
        .collect()
}

fn select_first_row(listbox: &ListBox) {
    if let Some(row) = listbox.row_at_index(0) {
        listbox.select_row(Some(&row));
    }
}

fn update_results(
    listbox: &ListBox,
    apps: &[AppEntry],
    query: &str,
    results: &Rc<RefCell<Vec<AppEntry>>>,
    usage: &UsageMap,
    show_usage: bool,
) {
    clear_listbox(listbox);

    let mut scored = score_apps(apps, query, usage);
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));

    let mut results_mut = results.borrow_mut();
    results_mut.clear();

    for (_, app) in scored.into_iter().take(10) {
        results_mut.push(app.clone());
        let row = build_result_row(app, usage, show_usage);
        listbox.add(&row);
    }

    listbox.show_all();
    select_first_row(listbox);
}

fn launch_from_index(
    index: i32,
    results: &Rc<RefCell<Vec<AppEntry>>>,
    usage: &Rc<RefCell<UsageMap>>,
) -> bool {
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

    {
        let mut usage_mut = usage.borrow_mut();
        record_usage(&app.key, &mut usage_mut);
        save_usage(&usage_mut);
    }

    true
}


struct LauncherState {
    apps: Rc<Vec<AppEntry>>,
    results: Rc<RefCell<Vec<AppEntry>>>,
    usage: Rc<RefCell<UsageMap>>,
}

impl LauncherState {
    fn new() -> Self {
        Self {
            apps: Rc::new(load_apps()),
            results: Rc::new(RefCell::new(Vec::new())),
            usage: Rc::new(RefCell::new(load_usage())),
        }
    }
}

fn configure_settings() {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_property("gtk-error-bell", &false);
    }
}

fn build_listbox() -> ListBox {
    let listbox = ListBox::new();
    listbox.set_selection_mode(gtk::SelectionMode::Single);
    listbox
}

fn build_container(entry: &Entry, listbox: &ListBox) -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 6);
    container.set_margin_top(8);
    container.set_margin_bottom(8);
    container.set_margin_start(10);
    container.set_margin_end(10);
    container.pack_start(entry, false, false, 0);
    container.pack_start(listbox, true, true, 0);
    container
}

fn apply_window_hints(window: &ApplicationWindow) {
    window.set_type_hint(gdk::WindowTypeHint::PopupMenu);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_accept_focus(true);
    window.set_focus_on_map(true);
}

fn connect_override_redirect(window: &ApplicationWindow) {
    window.connect_realize(|window| {
        if let Some(gdk_window) = window.window() {
            gdk_window.set_override_redirect(true);
        }
    });
}

fn connect_keyboard_grab(window: &ApplicationWindow) {
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
}

fn connect_keyboard_ungrab(window: &ApplicationWindow) {
    window.connect_unmap_event(|_, _| {
        if let Some(display) = gdk::Display::default() {
            if let Some(seat) = display.default_seat() {
                seat.ungrab();
            }
        }
        gtk::glib::Propagation::Proceed
    });
}

fn configure_window(window: &ApplicationWindow) {
    apply_window_hints(window);
    connect_override_redirect(window);
    connect_keyboard_grab(window);
    connect_keyboard_ungrab(window);
}

fn build_window(app: &Application, container: &gtk::Box) -> ApplicationWindow {
    let window = ApplicationWindow::builder()
        .application(app)
        .decorated(false)
        .default_width(600)
        .default_height(320)
        .resizable(false)
        .build();

    window.set_position(gtk::WindowPosition::Center);
    window.add(container);
    configure_window(&window);

    window
}

fn refresh_results(listbox: &ListBox, state: &LauncherState, show_usage: bool) {
    let usage_borrow = state.usage.borrow();
    update_results(
        listbox,
        &state.apps,
        "",
        &state.results,
        &usage_borrow,
        show_usage,
    );
}

fn focus_entry_later(entry: &Entry) {
    let entry_clone = entry.clone();
    gtk::glib::idle_add_local_once(move || {
        entry_clone.grab_focus();
    });
}

fn connect_listbox_activation(listbox: &ListBox, state: &LauncherState, app: &Application) {
    let results_for_activate = Rc::clone(&state.results);
    let usage_for_activate = Rc::clone(&state.usage);
    let app_for_activate = app.clone();
    listbox.connect_row_activated(move |_, row| {
        if launch_from_index(row.index(), &results_for_activate, &usage_for_activate) {
            app_for_activate.quit();
        }
    });
}

fn connect_entry_key_handler(
    entry: &Entry,
    listbox: &ListBox,
    state: &LauncherState,
    app: &Application,
) {
    let entry_for_keys = entry.clone();
    let listbox_for_keys = listbox.clone();
    let results_for_keys = Rc::clone(&state.results);
    let usage_for_keys = Rc::clone(&state.usage);
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
                if launch_from_index(row.index(), &results_for_keys, &usage_for_keys) {
                    app_for_keys.quit();
                }
            }
            entry_for_keys.grab_focus();
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });
}

fn connect_entry_change_handler(
    entry: &Entry,
    listbox: &ListBox,
    state: &LauncherState,
    show_usage: bool,
) {
    let listbox_for_change = listbox.clone();
    let apps_for_change = Rc::clone(&state.apps);
    let results_for_change = Rc::clone(&state.results);
    let usage_for_change = Rc::clone(&state.usage);
    entry.connect_changed(move |entry| {
        let query = entry.text().to_string();
        let usage_borrow = usage_for_change.borrow();
        update_results(
            &listbox_for_change,
            &apps_for_change,
            &query,
            &results_for_change,
            &usage_borrow,
            show_usage,
        );
    });
}

fn connect_entry_handlers(
    entry: &Entry,
    listbox: &ListBox,
    state: &LauncherState,
    app: &Application,
    show_usage: bool,
) {
    connect_entry_key_handler(entry, listbox, state, app);
    connect_entry_change_handler(entry, listbox, state, show_usage);
}

fn build_ui(app: &Application, show_usage: bool) {
    configure_settings();

    let entry = Entry::builder()
        .placeholder_text("Search…")
        .build();

    let state = LauncherState::new();
    let listbox = build_listbox();

    connect_listbox_activation(&listbox, &state, app);
    connect_entry_handlers(&entry, &listbox, &state, app, show_usage);

    let container = build_container(&entry, &listbox);
    let window = build_window(app, &container);

    refresh_results(&listbox, &state, show_usage);

    window.show_all();
    focus_entry_later(&entry);
}

fn configure_command_line(app: &Application, show_usage: Rc<Cell<bool>>) {
    app.connect_command_line(move |app, cmd| {
        let args = cmd.arguments();
        if args.iter().any(|arg| arg == "--usage") {
            show_usage.set(true);
        }
        app.activate();
        0
    });
}

fn build_app() -> Application {
    let app = Application::builder()
        .application_id("com.example.hyperfind")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    let show_usage = Rc::new(Cell::new(false));
    configure_command_line(&app, Rc::clone(&show_usage));

    let show_usage = Rc::clone(&show_usage);
    app.connect_activate(move |app| {
        build_ui(app, show_usage.get());
    });

    app
}

fn main() {
    let app = build_app();
    app.run();
}
