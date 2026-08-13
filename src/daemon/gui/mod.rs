/* niri-switch  Copyright (C) 2025  Kiki/Bouba Team */
mod store;
mod style;
mod window_list;

use super::dbus;
use super::niri_socket::NiriSocket;

use gio::prelude::*;
use glib::closure_local;
use gtk4::glib::clone;
use gtk4::prelude::*;
use gtk4_layer_shell::LayerShell;
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
use window_list::Direction;
use window_list::WindowList;

/* Type aliases to make signatures more readable */
type GlobalStoreRef = Arc<Mutex<store::GlobalStore>>;
type WindowWeakRef = glib::WeakRef<gtk4::ApplicationWindow>;

const GTK4_APP_ID: &str = "org.kikibouba.NiriSwitch";
const CLIENT_REQUEST_CAP: usize = 20;

/// Handle key press events on the main window
fn handle_key_pressed(key: gdk4::Key, window_ref: &WindowWeakRef) -> glib::Propagation {
    if key == gdk4::Key::Escape {
        let window = window_ref
            .upgrade()
            .expect("Controller shouldn't outlive the window");
        window.close();
    }
    glib::Propagation::Proceed
}

/// Confirm the current selection when the shortcut key or its modifier is released.
///
/// niri consumes the Tab event used by its global binding, so a layer-shell client
/// cannot rely on receiving that release. The Alt/Super release is still delivered
/// after the overlay has keyboard focus and provides the usual task-switcher flow.
fn handle_key_released(key: gdk4::Key, list: &WindowList) {
    if matches!(
        key,
        gdk4::Key::Tab
            | gdk4::Key::ISO_Left_Tab
            | gdk4::Key::Alt_L
            | gdk4::Key::Alt_R
            | gdk4::Key::Super_L
            | gdk4::Key::Super_R
            | gdk4::Key::Meta_L
            | gdk4::Key::Meta_R
            | gdk4::Key::Hyper_L
            | gdk4::Key::Hyper_R
    ) {
        list.activate_selected();
    }
}

/// Confirm when the held Alt/Mod modifier disappears from the modifier state.
fn handle_modifiers_changed(
    state: gdk4::ModifierType,
    modifier_seen: &Cell<bool>,
    list: &WindowList,
) -> glib::Propagation {
    let selector_modifiers = gdk4::ModifierType::ALT_MASK
        | gdk4::ModifierType::SUPER_MASK
        | gdk4::ModifierType::META_MASK
        | gdk4::ModifierType::HYPER_MASK;

    if state.intersects(selector_modifiers) {
        modifier_seen.set(true);
    } else if modifier_seen.replace(false) {
        list.activate_selected();
    }

    glib::Propagation::Proceed
}

/// Updates the cached window list with new windows, and remove the old ones
fn update_window_cache(windows: &[niri_ipc::Window], store: &GlobalStoreRef) {
    /* Create a set of current window ids */
    let current_id_set: HashSet<u64> = windows.iter().map(|window| window.id).collect();

    let mut store = store.lock().unwrap();
    /* Update the cache with the new id set */
    store.window_cache.update_cache(current_id_set);
}

/// Put the windows in the cached positions
fn sort_windows_by_cached_order(windows: &mut [niri_ipc::Window], store: &GlobalStoreRef) {
    let store = store.lock().unwrap();

    /* Create a lookup table that connects window id to the position in cached list */
    let index_lookup: HashMap<u64, usize> = store
        .window_cache
        .into_iter()
        .enumerate()
        .map(|(idx, id)| (*id, idx))
        .collect();

    /* Sort the windows by the indices */
    windows.sort_by_key(|window| index_lookup.get(&window.id).unwrap());
}

/// Handle selecting previous window in the overlay
async fn handle_previous_selection(list: &WindowList) {
    let window = list
        .root()
        .and_downcast::<gtk4::ApplicationWindow>()
        .expect("Root widget has to be an 'ApplicationWindow'");

    /* If window is already shown, move back the selection */
    if window.is_visible() {
        list.advance_the_selection(Direction::Backward);
    }
    /* Else: do nothing */
}

/// Handle request to activate the daemon
async fn handle_daemon_activated(list: &WindowList, store: &GlobalStoreRef) {
    let window = list
        .root()
        .and_downcast::<gtk4::ApplicationWindow>()
        .expect("Root widget has to be an 'ApplicationWindow'");

    /* If window is already shown, simply advance the selection */
    if window.is_visible() {
        list.advance_the_selection(Direction::Forward);
        return;
    }
    /* Else reload the listed windows, state might have changed since the last time.
     * This is also the initial filling of the list. */
    list.clear_the_list();

    /* Present before the blocking window query so the held shortcut modifier and
     * its release are captured. Confirmation is deferred while the model loads. */
    window.present();
    list.focus_to_list();

    /* niri socket uses blocking calls, so it will be run on a separate thread */
    let store_ref = store.clone();
    let mut windows = gio::spawn_blocking(move || {
        let mut store = store_ref.lock().unwrap();
        store.niri_socket.list_windows()
    })
    .await
    .expect("Request for windows shouldn't fail");

    /* No need to display anything if there is no window */
    if windows.is_empty() {
        list.cancel_pending_activation();
        window.close();
        return;
    }

    /* Window list could have changed since the last time */
    update_window_cache(&windows, store);

    /* Put windows in positions that they were last time */
    sort_windows_by_cached_order(&mut windows, store);

    /* If there is more then one window, swap the first two */
    if windows.len() > 1 {
        windows.swap(0, 1);
    }

    /* Append windows to the list model */
    list.fill_the_list(&windows, store);

    /* The window was presented before loading so that it could catch a quick Tab
     * release. If that happened, filling the list confirms the first selection. */
}

/// Handle event from the D-Bus connection
async fn handle_dbus_event(event: dbus::DbusEvent, list: &WindowList, store: &GlobalStoreRef) {
    use dbus::DbusEvent::*;
    match event {
        Activate => handle_daemon_activated(list, store).await,
        Previous => handle_previous_selection(list).await,
    }
}

/// Move focus to the chosen window
pub fn change_focused_window(window_id: u64, store: &GlobalStoreRef) {
    /* Create async context and next spawn separate thread that will perform the
     * blocking calls */
    glib::spawn_future_local(clone!(
        #[strong]
        store,
        async move {
            /* Move the chosen window to the front of the window list */
            store.lock().unwrap().window_cache.move_to_front(&window_id);

            /* Socket uses blocking calls, so we create a separete thread */
            gio::spawn_blocking(move || {
                let mut store = store.lock().unwrap();
                store.niri_socket.change_focused_window(window_id);
            })
            .await
            .expect("Blocking call must succeed");
        }
    ));
}

/// Creates the main window and widgets
fn activate(application: &gtk4::Application, global_store: &GlobalStoreRef) {
    /* Create widget for displaying list of windows */
    let window_list = window_list::WindowList::default();

    /* Create a strong referance to the store object so that it can be passed
     * to the next closure. The closure can outlive the current scope so it
     * has to own a reference to this object */
    let store_ref = global_store.clone();

    /* Connect to the window-selected signal of the WindowList widget and trigger
     * change of focus */
    window_list.connect_closure(
        "window-selected",
        false,
        closure_local!(move |list: &WindowList, window_id: u64| {
            /* Change focus to the selected window */
            change_focused_window(window_id, &store_ref);

            /* Hide the overlay after changing the focus */
            let window = list
                .root()
                .and_downcast::<gtk4::Window>()
                .expect("Root widget has to be a 'Window'");
            window.close()
        }),
    );

    /* Create main window */
    let window = gtk4::ApplicationWindow::builder()
        .application(application)
        .child(&window_list)
        .build();

    /* GtkWindow adds the generic `background` CSS class automatically. Themes
     * such as Orchis paint that class as an opaque rectangle before the child
     * snapshot, which remains visible outside our rounded panel. */
    window.remove_css_class("background");

    /* Create a weak reference to the window, this will be moved to keyboard controller
     * which will later be attached to the window - with strong referance this could
     * potentially cause a reference cycle and memory leak */
    let window_ref = window.downgrade();
    let keyboard_controller = gtk4::EventControllerKey::new();
    keyboard_controller
        .connect_key_pressed(move |_, key, _, _| handle_key_pressed(key, &window_ref));
    keyboard_controller.connect_key_released(clone!(
        #[weak]
        window_list,
        move |_, key, _, _| handle_key_released(key, &window_list)
    ));
    let modifier_seen = Cell::new(false);
    keyboard_controller.connect_modifiers(clone!(
        #[weak]
        window_list,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, state| handle_modifiers_changed(state, &modifier_seen, &window_list)
    ));

    window.add_controller(keyboard_controller);

    /* Move this window to the shell layer, this allows to escape Niri compositor
     * and display window on top of everything else */
    window.init_layer_shell();
    window.set_decorated(false);
    window.set_resizable(false);
    window.add_css_class("niri-switch-window");
    /* A layer-shell surface with no anchors is centered by the compositor. */
    window.set_anchor(gtk4_layer_shell::Edge::Left, false);
    window.set_anchor(gtk4_layer_shell::Edge::Top, false);
    window.set_anchor(gtk4_layer_shell::Edge::Right, false);
    window.set_anchor(gtk4_layer_shell::Edge::Bottom, false);
    window.set_margin(gtk4_layer_shell::Edge::Left, 0);
    window.set_margin(gtk4_layer_shell::Edge::Top, 0);
    window.set_margin(gtk4_layer_shell::Edge::Right, 0);
    window.set_margin(gtk4_layer_shell::Edge::Bottom, 0);
    window.set_layer(gtk4_layer_shell::Layer::Overlay);
    window.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::Exclusive);
    window.set_namespace(Some("niri-switch"));
    window.set_hide_on_close(true);
    window.set_exclusive_zone(0);

    /* DBus server will communicate with GTK app via async channel */
    let (sender, receiver) = async_channel::bounded(CLIENT_REQUEST_CAP);

    /* Start dbus server for communication with client app */
    glib::spawn_future_local(async move {
        dbus::server_loop(sender)
            .await
            .expect("DBus server shouldn't fail");
    });

    /* Start a task that handles events from D-Bus */
    glib::spawn_future_local(clone!(
        #[weak]
        window_list,
        #[strong]
        global_store,
        async move {
            while let Ok(event) = receiver.recv().await {
                handle_dbus_event(event, &window_list, &global_store).await;
            }
        }
    ));
}

/// Start the GUI for choosing next window to focus
pub fn start_gui(niri_socket: NiriSocket) {
    /* This use of atomic smart pointer and mutex allow for multiple owners that can
     * acquire the store object and mutate it from the context of different threads */
    let store_ref = Arc::new(Mutex::new(store::GlobalStore::new(niri_socket)));

    /* Load GTK resources, this will load the compressed *.ui files */
    gio::resources_register_include!("composite_templates.gresource")
        .expect("Registering resources should not fail");

    let application = gtk4::Application::new(Some(GTK4_APP_ID), Default::default());

    application.connect_startup(|_| style::load_css());
    application.connect_activate(move |app| activate(app, &store_ref));

    /* Need to pass no arguments explicitely, otherwise gtk will try to parse our
     * custom cli options */
    let no_args: Vec<String> = vec![];
    application.run_with_args(&no_args);
}
