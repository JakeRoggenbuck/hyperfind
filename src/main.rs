use gtk::gdk;
use gtk::gdk::prelude::*;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Entry};

fn main() {
    let app = Application::builder()
        .application_id("com.example.hyperfind")
        .build();

    app.connect_activate(|app| {
        let entry = Entry::builder()
            .placeholder_text("Search…")
            .build();

        let window = ApplicationWindow::builder()
            .application(app)
            .decorated(false)
            .default_width(600)
            .default_height(60)
            .resizable(false)
            .build();

        window.add(&entry);

        window.set_type_hint(gdk::WindowTypeHint::PopupMenu);
        window.set_skip_taskbar_hint(true);
        window.set_skip_pager_hint(true);
        window.connect_realize(|window| {
            if let Some(gdk_window) = window.window() {
                gdk_window.set_override_redirect(true);
            }
        });

        window.show_all();

        entry.grab_focus();
    });

    app.run();
}
