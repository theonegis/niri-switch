/* niri-switch  Copyright (C) 2025  Kiki/Bouba Team */
use super::window_info::WindowInfo;
use glib::subclass::InitializingObject;
use glib::subclass::Signal;
use gtk4::subclass::prelude::*;
use std::{
    cell::{Cell, RefCell},
    sync::OnceLock,
};

use gtk4::prelude::*;

/* Custom widget for displaying app cards in independently centered rows. */
#[derive(Default, gtk4::CompositeTemplate)]
#[template(resource = "/org/kikibouba/niriswitch/window_list/window_list.ui")]
pub struct WindowList {
    #[template_child]
    pub rows: TemplateChild<gtk4::Box>,

    /// Window data and its matching card widgets share the same stable index.
    pub windows: RefCell<Vec<WindowInfo>>,
    pub cards: RefCell<Vec<gtk4::Box>>,
    pub selected: Cell<u32>,

    /// Tab can be released while the window model is still loading. Remember the
    /// confirmation and apply it as soon as the first selection becomes available.
    pub activation_pending: Cell<bool>,

    /// Prevent the key-release and modifier-state callbacks from committing the
    /// same selection twice.
    pub activation_committed: Cell<bool>,

    /// Preserve repeated shortcut activations that arrive while the window model
    /// is still being loaded.
    pub pending_advances: Cell<i32>,
}

#[glib::object_subclass]
impl ObjectSubclass for WindowList {
    const NAME: &'static str = "WindowList";
    type Type = super::WindowList;
    type ParentType = gtk4::Box;

    fn class_init(class: &mut Self::Class) {
        class.bind_template();
        class.set_css_name("window-list-wrapper");
    }

    fn instance_init(obj: &InitializingObject<Self>) {
        obj.init_template();
    }
}

impl ObjectImpl for WindowList {
    fn signals() -> &'static [Signal] {
        static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
        SIGNALS.get_or_init(|| {
            vec![
                /* This signal will be emited with the id of the chosen window */
                Signal::builder("window-selected")
                    .param_types([u64::static_type()])
                    .build(),
            ]
        })
    }

    fn constructed(&self) {
        self.parent_constructed();
        let obj = self.obj();
        self.selected.set(gtk4::INVALID_LIST_POSITION);
        obj.set_focusable(true);
        obj.set_overflow(gtk4::Overflow::Hidden);
    }
}

impl WidgetImpl for WindowList {
    fn snapshot(&self, snapshot: &gtk4::Snapshot) {
        let widget = self.obj();
        let bounds =
            gtk4::graphene::Rect::new(0.0, 0.0, widget.width() as f32, widget.height() as f32);
        let rounded_bounds = gtk4::gsk::RoundedRect::from_rect(bounds, 12.0);

        let panel_color = gdk4::RGBA::new(0.93, 0.94, 0.94, 0.65);

        // Paint one self-contained translucent panel. Native Wayland blur is
        // intentionally not requested, so there is no second surface to drift.
        snapshot.push_rounded_clip(&rounded_bounds);
        snapshot.append_color(&panel_color, &bounds);
        self.parent_snapshot(snapshot);
        snapshot.pop();

        let border_color = gdk4::RGBA::new(1.0, 1.0, 1.0, 0.72);
        snapshot.append_border(
            &rounded_bounds,
            &[1.0; 4],
            &[border_color, border_color, border_color, border_color],
        );
    }
}
impl BoxImpl for WindowList {}
