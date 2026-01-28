use gio::prelude::*;
use gtk::gdk;
use gtk::gdk::prelude::*;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Entry, ListBox};
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use strsim::jaro_winkler;

#[derive(Clone)]
struct AppEntry {
    key: String,
    name: String,
    icon: Option<gio::Icon>,
    app_info: gio::AppInfo,
}

enum ViewItem {
    Header(String),
    App(AppEntry),
}

#[derive(Clone, Deserialize, Serialize)]
struct UsageEntry {
    count: u64,
    last_used: u64,
}

type UsageMap = HashMap<String, UsageEntry>;

const MAX_RESULTS: usize = 10;
const MAX_FREQUENT: usize = 5;

struct ViewState {
    items: Vec<ViewItem>,
    offset: usize,
    selected_index: Option<usize>,
}

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
    app.id()
        .map(|id| id.to_string())
        .unwrap_or_else(|| name.to_string())
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
                Some(AppEntry {
                    key,
                    name,
                    icon,
                    app_info: app,
                })
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

fn first_selectable_row(listbox: &ListBox) -> Option<gtk::ListBoxRow> {
    for child in listbox.children() {
        if let Ok(row) = child.downcast::<gtk::ListBoxRow>() {
            if row.is_selectable() {
                return Some(row);
            }
        }
    }
    None
}

fn build_section_row(title: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_selectable(false);
    row.set_activatable(false);
    let label = gtk::Label::new(None);
    label.set_markup(&format!("<b>{}</b>", title));
    label.set_xalign(0.0);
    label.set_margin_top(0);
    label.set_margin_bottom(0);
    row.set_margin_top(0);
    row.set_margin_bottom(0);
    row.add(&label);
    row
}

fn first_selectable_index(items: &[ViewItem]) -> Option<usize> {
    items
        .iter()
        .position(|item| matches!(item, ViewItem::App(_)))
}

fn next_selectable_index(items: &[ViewItem], start: usize, direction: i32) -> Option<usize> {
    let mut index = start as i32 + direction;
    while index >= 0 && (index as usize) < items.len() {
        if matches!(items[index as usize], ViewItem::App(_)) {
            return Some(index as usize);
        }
        index += direction;
    }
    None
}

fn ensure_visible(view_state: &mut ViewState) {
    let Some(selected) = view_state.selected_index else {
        view_state.offset = 0;
        return;
    };

    if view_state.items.is_empty() {
        view_state.offset = 0;
        return;
    }

    if view_state.items.len() <= MAX_RESULTS {
        view_state.offset = 0;
        return;
    }

    if selected < view_state.offset {
        view_state.offset = selected;
        return;
    }

    loop {
        let mut app_count = 0;
        let mut last_visible = None;
        for (idx, item) in view_state.items.iter().enumerate().skip(view_state.offset) {
            if matches!(item, ViewItem::App(_)) {
                if app_count >= MAX_RESULTS {
                    break;
                }
                app_count += 1;
            }
            last_visible = Some(idx);
            if app_count >= MAX_RESULTS {
                break;
            }
        }

        let Some(last_visible) = last_visible else {
            break;
        };
        if selected <= last_visible {
            break;
        }
        view_state.offset = view_state.offset.saturating_add(1);
    }
}

fn render_view(
    listbox: &ListBox,
    view_state: &ViewState,
    results: &Rc<RefCell<Vec<Option<AppEntry>>>>,
    usage: &UsageMap,
    show_usage: bool,
) {
    clear_listbox(listbox);

    let mut results_mut = results.borrow_mut();
    results_mut.clear();

    let mut app_count = 0;
    let mut visible_indices = Vec::new();
    for (idx, item) in view_state.items.iter().enumerate().skip(view_state.offset) {
        if matches!(item, ViewItem::App(_)) && app_count >= MAX_RESULTS {
            break;
        }
        match item {
            ViewItem::Header(title) => {
                results_mut.push(None);
                let row = build_section_row(title);
                listbox.add(&row);
            }
            ViewItem::App(app) => {
                results_mut.push(Some(app.clone()));
                let row = build_result_row(app, usage, show_usage);
                listbox.add(&row);
                app_count += 1;
            }
        }
        visible_indices.push(idx);
    }

    listbox.show_all();
    if let Some(selected) = view_state.selected_index {
        if let Some(row_index) = visible_indices.iter().position(|idx| *idx == selected) {
            if let Some(row) = listbox.row_at_index(row_index as i32) {
                listbox.select_row(Some(&row));
            }
        }
    }
}

fn build_view_items(apps: &[AppEntry], query: &str, usage: &UsageMap) -> Vec<ViewItem> {
    if !query.trim().is_empty() {
        let mut scored = score_apps(apps, query, usage);
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        return scored
            .into_iter()
            .map(|(_, app)| ViewItem::App(app.clone()))
            .collect();
    }

    let mut frequent: Vec<(i64, &AppEntry)> = apps
        .iter()
        .filter_map(|app| {
            usage.get(&app.key).map(|entry| {
                let score = (entry.count as i64 * 1000) + entry.last_used as i64;
                (score, app)
            })
        })
        .collect();
    frequent.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    let frequent: Vec<&AppEntry> = frequent
        .into_iter()
        .map(|(_, app)| app)
        .take(MAX_FREQUENT)
        .collect();

    let mut items = Vec::new();
    if !frequent.is_empty() {
        items.push(ViewItem::Header("Frequently Used".to_string()));
        for app in &frequent {
            items.push(ViewItem::App((*app).clone()));
        }
    }

    items.push(ViewItem::Header("All Apps".to_string()));

    let mut frequent_keys = HashSet::new();
    for app in frequent {
        frequent_keys.insert(app.key.clone());
    }

    for app in apps.iter().filter(|app| !frequent_keys.contains(&app.key)) {
        items.push(ViewItem::App(app.clone()));
    }

    items
}

fn score_apps<'a>(apps: &'a [AppEntry], query: &str, usage: &UsageMap) -> Vec<(i64, &'a AppEntry)> {
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

fn update_results(listbox: &ListBox, state: &LauncherState, query: &str, show_usage: bool) {
    let usage_borrow = state.usage.borrow();
    let mut view_state = state.view.borrow_mut();
    view_state.items = build_view_items(&state.apps, query, &usage_borrow);
    view_state.offset = 0;
    view_state.selected_index = first_selectable_index(&view_state.items);
    render_view(
        listbox,
        &view_state,
        &state.results,
        &usage_borrow,
        show_usage,
    );
}

fn move_selection(listbox: &ListBox, state: &LauncherState, direction: i32, show_usage: bool) {
    let usage_borrow = state.usage.borrow();
    let mut view_state = state.view.borrow_mut();
    let Some(current) = view_state.selected_index else {
        view_state.selected_index = first_selectable_index(&view_state.items);
        ensure_visible(&mut view_state);
        render_view(
            listbox,
            &view_state,
            &state.results,
            &usage_borrow,
            show_usage,
        );
        return;
    };

    if let Some(next) = next_selectable_index(&view_state.items, current, direction) {
        view_state.selected_index = Some(next);
        ensure_visible(&mut view_state);
        render_view(
            listbox,
            &view_state,
            &state.results,
            &usage_borrow,
            show_usage,
        );
    }
}

fn launch_from_index(
    index: i32,
    results: &Rc<RefCell<Vec<Option<AppEntry>>>>,
    usage: &Rc<RefCell<UsageMap>>,
) -> bool {
    if index < 0 {
        return false;
    }

    let results = results.borrow();
    let index = index as usize;
    let Some(Some(app)) = results.get(index) else {
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

#[derive(Clone)]
struct LauncherState {
    apps: Rc<Vec<AppEntry>>,
    results: Rc<RefCell<Vec<Option<AppEntry>>>>,
    usage: Rc<RefCell<UsageMap>>,
    view: Rc<RefCell<ViewState>>,
}

impl LauncherState {
    fn new() -> Self {
        Self {
            apps: Rc::new(load_apps()),
            results: Rc::new(RefCell::new(Vec::new())),
            usage: Rc::new(RefCell::new(load_usage())),
            view: Rc::new(RefCell::new(ViewState {
                items: Vec::new(),
                offset: 0,
                selected_index: None,
            })),
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

fn build_container(title: &gtk::Label, entry: &Entry, listbox: &ListBox) -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 6);
    container.set_margin_top(8);
    container.set_margin_bottom(8);
    container.set_margin_start(10);
    container.set_margin_end(10);
    container.pack_start(title, false, false, 0);
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
    update_results(listbox, state, "", show_usage);
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
    show_usage: bool,
) {
    let entry_for_keys = entry.clone();
    let listbox_for_keys = listbox.clone();
    let results_for_keys = Rc::clone(&state.results);
    let usage_for_keys = Rc::clone(&state.usage);
    let state_for_keys = state.clone();
    let app_for_keys = app.clone();
    entry.connect_key_press_event(move |_, event| {
        let key = event.keyval();
        if key == gdk::keys::constants::Escape {
            app_for_keys.quit();
            return gtk::glib::Propagation::Stop;
        }
        if key == gdk::keys::constants::Down {
            move_selection(&listbox_for_keys, &state_for_keys, 1, show_usage);
            return gtk::glib::Propagation::Stop;
        }
        if key == gdk::keys::constants::Up {
            move_selection(&listbox_for_keys, &state_for_keys, -1, show_usage);
            return gtk::glib::Propagation::Stop;
        }
        if key == gdk::keys::constants::Return || key == gdk::keys::constants::KP_Enter {
            let row = listbox_for_keys
                .selected_row()
                .or_else(|| first_selectable_row(&listbox_for_keys));
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
    let state_for_change = state.clone();
    entry.connect_changed(move |entry| {
        let query = entry.text().to_string();
        update_results(&listbox_for_change, &state_for_change, &query, show_usage);
    });
}

fn connect_entry_handlers(
    entry: &Entry,
    listbox: &ListBox,
    state: &LauncherState,
    app: &Application,
    show_usage: bool,
) {
    connect_entry_key_handler(entry, listbox, state, app, show_usage);
    connect_entry_change_handler(entry, listbox, state, show_usage);
}

fn build_ui(app: &Application, show_usage: bool) {
    configure_settings();

    let title = gtk::Label::new(Some("HyperFind"));
    title.set_xalign(0.0);

    let entry = Entry::builder().placeholder_text("Search…").build();

    let state = LauncherState::new();
    let listbox = build_listbox();

    connect_listbox_activation(&listbox, &state, app);
    connect_entry_handlers(&entry, &listbox, &state, app, show_usage);

    let container = build_container(&title, &entry, &listbox);
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
