/* niri-switch  Copyright (C) 2025  Kiki/Bouba Team */
mod imp;
mod window_info;
mod window_item;

use gtk4::glib;
use gtk4::subclass::prelude::*;
use gtk4::{SingleSelection, prelude::*};
use niri_ipc::Window;
use window_info::WindowInfo;

/* Here we create custom widget for displaying window info by
 * subclassing gtk4::Box */
glib::wrapper! {
    pub struct WindowList(ObjectSubclass<imp::WindowList>)
        @extends gtk4::Widget, gtk4::Box,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

#[derive(Clone, Copy)]
pub enum Direction {
    Forward,
    Backward,
}

impl Default for WindowList {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl WindowList {
    /// Given list of niri Windows fill the GTK list of windows
    pub fn fill_the_list(&self, windows: &Vec<Window>, store: &super::GlobalStoreRef) {
        let imp = self.imp();
        let list_store = get_list_store(&imp.list);

        for window in windows {
            /* Try to get information about the app that coresponds to the window */
            let window_info = get_widow_info_for_niri_window(window, store);
            list_store.append(&window_info);
        }

        let pending_advances = imp.pending_advances.replace(0);
        let pending_direction = if pending_advances < 0 {
            Direction::Backward
        } else {
            Direction::Forward
        };
        for _ in 0..pending_advances.unsigned_abs() {
            self.advance_the_selection(pending_direction);
        }

        if imp.activation_pending.replace(false) {
            self.activate_selected();
        }
    }

    /// Moves the current selection one step in the given direction
    /// If the new position goes past the end or before the beginning, the selection wraps around
    pub fn advance_the_selection(&self, direction: Direction) {
        let imp = self.imp();
        let selection_model = get_selection_model(&imp.list);
        let list_store = get_list_store(&imp.list);

        let Some(new_selected) =
            next_selection(selection_model.selected(), list_store.n_items(), direction)
        else {
            let pending_shift = match direction {
                Direction::Forward => 1,
                Direction::Backward => -1,
            };
            imp.pending_advances
                .set(imp.pending_advances.get().saturating_add(pending_shift));
            return;
        };

        imp.list
            .scroll_to(new_selected, gtk4::ListScrollFlags::FOCUS, None);
        imp.list
            .scroll_to(new_selected, gtk4::ListScrollFlags::SELECT, None);
    }

    /// Remove all the windows added to the GTK window list
    pub fn clear_the_list(&self) {
        let imp = self.imp();
        let list_store = get_list_store(&imp.list);
        list_store.remove_all();
        imp.activation_pending.set(false);
        imp.pending_advances.set(0);
    }

    /// Bring focus to the inner list
    pub fn focus_to_list(&self) {
        let imp = self.imp();
        imp.list.grab_focus();
    }

    /// Activate the currently highlighted window. If the model is still loading,
    /// defer activation until `fill_the_list` has produced a selection.
    pub fn activate_selected(&self) {
        let selection_model = get_selection_model(&self.imp().list);

        match selection_model.selected_item().and_downcast::<WindowInfo>() {
            Some(window_info) => {
                self.imp().activation_pending.set(false);
                self.emit_by_name::<()>("window-selected", &[&window_info.id()]);
            }
            None => self.imp().activation_pending.set(true),
        }
    }

    /// Activate an item chosen with pointer input or Enter.
    pub fn activate_position(&self, position: u32) {
        let window_info = get_selection_model(&self.imp().list)
            .item(position)
            .and_downcast::<WindowInfo>()
            .expect("Model item has to be a 'WindowInfo'");

        self.emit_by_name::<()>("window-selected", &[&window_info.id()]);
    }

    /// Drop a deferred activation when loading resulted in an empty window list.
    pub fn cancel_pending_activation(&self) {
        self.imp().activation_pending.set(false);
    }
}

/// Calculate the next item, including wrapping at both ends of the list.
fn next_selection(selected: u32, item_count: u32, direction: Direction) -> Option<u32> {
    if item_count == 0 {
        return None;
    }

    if selected >= item_count {
        return Some(match direction {
            Direction::Forward => 0,
            Direction::Backward => item_count - 1,
        });
    }

    Some(match direction {
        Direction::Forward if selected + 1 == item_count => 0,
        Direction::Forward => selected + 1,
        Direction::Backward if selected == 0 => item_count - 1,
        Direction::Backward => selected - 1,
    })
}

/// Retrieves glib selection model from GTK4 window list
fn get_selection_model(list: &gtk4::ListView) -> SingleSelection {
    list.model()
        .expect("ListView needs to have a model")
        .downcast::<gtk4::SingleSelection>()
        .expect("Needs to be a 'SingleSelection' type")
}

/// Retrieves GIO list store from GTK4 window list
fn get_list_store(list: &gtk4::ListView) -> gio::ListStore {
    let selection_model = get_selection_model(list);
    selection_model
        .model()
        .and_downcast::<gio::ListStore>()
        .expect("Needs to be a 'ListStore type")
}

/// Given a niri Window description returns a WindowInfo GObject
fn get_widow_info_for_niri_window(
    window: &niri_ipc::Window,
    store: &super::GlobalStoreRef,
) -> WindowInfo {
    let store = store.lock().unwrap();
    let app_id = window.app_id.clone().unwrap_or_default();
    let window_title = window.title.clone().unwrap_or_default();

    /* Try to get information about the app that coresponds to the window */
    match store.app_database.get_app_info(&app_id) {
        Some(app_info) => {
            let icon = app_info
                .icon
                .map(|icon| gio::Icon::deserialize(&icon).unwrap());
            WindowInfo::new(window.id, &window_title, &app_info.display_name, icon)
        }
        None => WindowInfo::new(window.id, &window_title, &app_id, None),
    }
}

#[cfg(test)]
mod tests {
    use super::{Direction, next_selection};

    #[test]
    fn forward_selection_wraps_from_last_to_first() {
        assert_eq!(next_selection(2, 3, Direction::Forward), Some(0));
    }

    #[test]
    fn backward_selection_wraps_from_first_to_last() {
        assert_eq!(next_selection(0, 3, Direction::Backward), Some(2));
    }

    #[test]
    fn empty_list_has_no_selection() {
        assert_eq!(next_selection(0, 0, Direction::Forward), None);
    }
}
