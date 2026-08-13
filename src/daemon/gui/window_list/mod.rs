/* niri-switch  Copyright (C) 2025  Kiki/Bouba Team */
mod imp;
mod window_info;
mod window_item;

use gdk4::prelude::*;
use gtk4::glib;
use gtk4::glib::clone;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use niri_ipc::Window;
use window_info::WindowInfo;
use window_item::WindowItem;

const MAX_SCREEN_WIDTH_FRACTION: f64 = 0.90;
const PANEL_HORIZONTAL_PADDING: i32 = 20;

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
    /// Given a list of niri windows, build the minimum number of balanced rows
    /// needed to keep every row inside 90% of the current monitor width.
    pub fn fill_the_list(&self, windows: &[Window], store: &super::GlobalStoreRef) {
        let window_infos: Vec<_> = windows
            .iter()
            .map(|window| get_widow_info_for_niri_window(window, store))
            .collect();

        self.build_cards(window_infos);
        self.set_selected(0);

        let imp = self.imp();
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

    fn build_cards(&self, window_infos: Vec<WindowInfo>) {
        let imp = self.imp();
        let mut cards = Vec::with_capacity(window_infos.len());
        let mut card_widths = Vec::with_capacity(window_infos.len());

        for (position, window_info) in window_infos.iter().cloned().enumerate() {
            let item = WindowItem::default();
            item.set_window_info(window_info);

            let card = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            card.add_css_class("window-card");
            card.append(&item);

            let gesture = gtk4::GestureClick::new();
            gesture.connect_released(clone!(
                #[weak(rename_to = list)]
                self,
                move |_, _, _, _| list.activate_position(position as u32)
            ));
            card.add_controller(gesture);

            let (_, natural_width, _, _) = card.measure(gtk4::Orientation::Horizontal, -1);
            card_widths.push(natural_width.max(1));
            cards.push(card);
        }

        let max_row_width = self.maximum_row_width();
        let row_count = choose_row_count(&card_widths, max_row_width);
        let counts = balanced_row_counts(cards.len(), row_count);

        let mut cards_iter = cards.iter();
        for count in counts {
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            row.set_halign(gtk4::Align::Center);
            row.set_hexpand(false);
            row.add_css_class("window-row");

            for card in cards_iter.by_ref().take(count) {
                row.append(card);
            }
            imp.rows.append(&row);
        }

        *imp.windows.borrow_mut() = window_infos;
        *imp.cards.borrow_mut() = cards;
        self.queue_resize();
    }

    fn maximum_row_width(&self) -> i32 {
        let monitor_width = self
            .root()
            .and_downcast::<gtk4::Window>()
            .and_then(|window| window.surface())
            .and_then(|surface| surface.display().monitor_at_surface(&surface))
            .map(|monitor| monitor.geometry().width())
            .unwrap_or(1920);

        ((f64::from(monitor_width) * MAX_SCREEN_WIDTH_FRACTION).floor() as i32
            - PANEL_HORIZONTAL_PADDING)
            .max(1)
    }

    /// Move the current selection one step, wrapping across rows at both ends.
    pub fn advance_the_selection(&self, direction: Direction) {
        let imp = self.imp();
        let item_count = imp.windows.borrow().len() as u32;
        let Some(new_selected) = next_selection(imp.selected.get(), item_count, direction) else {
            let pending_shift = match direction {
                Direction::Forward => 1,
                Direction::Backward => -1,
            };
            imp.pending_advances
                .set(imp.pending_advances.get().saturating_add(pending_shift));
            return;
        };

        self.set_selected(new_selected);
    }

    fn set_selected(&self, position: u32) {
        let imp = self.imp();
        let cards = imp.cards.borrow();
        if position as usize >= cards.len() {
            return;
        }

        let old_position = imp.selected.replace(position);
        if let Some(card) = cards.get(old_position as usize) {
            card.remove_css_class("selected");
        }
        cards[position as usize].add_css_class("selected");
    }

    /// Remove all windows and rows from the switcher.
    pub fn clear_the_list(&self) {
        let imp = self.imp();
        while let Some(child) = imp.rows.first_child() {
            imp.rows.remove(&child);
        }
        imp.windows.borrow_mut().clear();
        imp.cards.borrow_mut().clear();
        imp.selected.set(gtk4::INVALID_LIST_POSITION);
        imp.activation_pending.set(false);
        imp.activation_committed.set(false);
        imp.pending_advances.set(0);
    }

    /// Bring keyboard focus to the switcher.
    pub fn focus_to_list(&self) {
        self.grab_focus();
    }

    /// Activate the currently highlighted window. If data is still loading,
    /// defer activation until `fill_the_list` produces the first selection.
    pub fn activate_selected(&self) {
        let imp = self.imp();
        if imp.activation_committed.get() {
            return;
        }

        let position = imp.selected.get() as usize;
        match imp.windows.borrow().get(position).cloned() {
            Some(window_info) => {
                imp.activation_pending.set(false);
                imp.activation_committed.set(true);
                self.emit_by_name::<()>("window-selected", &[&window_info.id()]);
            }
            None => imp.activation_pending.set(true),
        }
    }

    /// Activate an item chosen with pointer input or Enter.
    pub fn activate_position(&self, position: u32) {
        let imp = self.imp();
        if imp.activation_committed.replace(true) {
            return;
        }

        let Some(window_info) = imp.windows.borrow().get(position as usize).cloned() else {
            imp.activation_committed.set(false);
            return;
        };
        imp.selected.set(position);
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

/// Divide `item_count` as evenly as possible, putting each remainder item in
/// an earlier row. For example, seven items across three rows become 3, 2, 2.
fn balanced_row_counts(item_count: usize, row_count: usize) -> Vec<usize> {
    if item_count == 0 || row_count == 0 {
        return Vec::new();
    }

    let row_count = row_count.min(item_count);
    let base = item_count / row_count;
    let remainder = item_count % row_count;
    (0..row_count)
        .map(|row| base + usize::from(row < remainder))
        .collect()
}

/// Select the smallest row count whose widest balanced row fits. Card widths
/// include their CSS margin and padding because they come from GTK measurement.
fn choose_row_count(card_widths: &[i32], maximum_width: i32) -> usize {
    if card_widths.is_empty() {
        return 0;
    }

    for row_count in 1..=card_widths.len() {
        let counts = balanced_row_counts(card_widths.len(), row_count);
        let mut start = 0;
        let fits = counts.into_iter().all(|count| {
            let end = start + count;
            let width: i32 = card_widths[start..end].iter().sum();
            start = end;
            width <= maximum_width
        });
        if fits {
            return row_count;
        }
    }

    card_widths.len()
}

/// Given a niri Window description, return a WindowInfo GObject.
fn get_widow_info_for_niri_window(
    window: &niri_ipc::Window,
    store: &super::GlobalStoreRef,
) -> WindowInfo {
    let store = store.lock().unwrap();
    let app_id = window.app_id.clone().unwrap_or_default();
    let window_title = window.title.clone().unwrap_or_default();

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
    use super::{Direction, balanced_row_counts, choose_row_count, next_selection};

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

    #[test]
    fn balanced_rows_put_remainder_in_earlier_rows() {
        assert_eq!(balanced_row_counts(7, 2), vec![4, 3]);
        assert_eq!(balanced_row_counts(8, 3), vec![3, 3, 2]);
    }

    #[test]
    fn chooses_minimum_number_of_rows_that_fit() {
        assert_eq!(choose_row_count(&[150; 7], 600), 2);
        assert_eq!(choose_row_count(&[150; 9], 600), 3);
        assert_eq!(choose_row_count(&[150; 4], 600), 1);
    }
}
