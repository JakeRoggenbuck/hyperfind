use gtk::gdk;
use gtk::gdk::prelude::*;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, Entry};

fn main() {
    let app = Application::builder()
        .application_id("com.example.hyperfind")
        .build();

    app.connect_activate(|app| {
        if let Some(settings) = gtk::Settings::default() {
            settings.set_property("gtk-error-bell", &false);
        }

        let entry = Entry::builder().placeholder_text("Search…").build();

        let app_clone = app.clone();
        entry.connect_key_press_event(move |_, event| {
            if event.keyval() == gdk::keys::constants::Escape {
                app_clone.quit();
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });

        let window = ApplicationWindow::builder()
            .application(app)
            .decorated(false)
            .default_width(600)
            .default_height(60)
            .resizable(false)
            .build();

        window.set_position(gtk::WindowPosition::Center);

        window.add(&entry);

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

        window.show_all();

        let entry_clone = entry.clone();
        gtk::glib::idle_add_local_once(move || {
            entry_clone.grab_focus();
        });
    });

    app.run();
}
