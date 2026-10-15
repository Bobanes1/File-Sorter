//! The File Sorter icon in the notification area (next to the clock) while it keeps
//! running in the background. Windows only.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use eframe::egui;

use crate::i18n::Lang;
use crate::winwin::MainWindow;

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::atomic::Ordering;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    pub struct Tray {
        icon: TrayIcon,
    }

    impl Tray {
        pub fn set_tooltip(&self, text: &str) {
            let _ = self.icon.set_tooltip(Some(text));
        }
    }

    pub fn create(
        lang: Lang,
        tooltip: &str,
        window: MainWindow,
        ctx: egui::Context,
        quit: Arc<AtomicBool>,
    ) -> Option<Tray> {
        let open = MenuItem::new(lang.tray_open(), true, None);
        let quit_item = MenuItem::new(lang.tray_quit(), true, None);
        let menu = Menu::new();
        menu.append_items(&[&open, &PredefinedMenuItem::separator(), &quit_item]).ok()?;
        let app_icon = crate::gui::app_icon();
        let icon = Icon::from_rgba(app_icon.rgba, app_icon.width, app_icon.height).ok()?;
        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip(tooltip)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| log::info!("Could not create the tray icon: {e}"))
            .ok()?;

        let click_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            let open = matches!(
                event,
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                    | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }
            );
            if open {
                window.bring_back();
                click_ctx.request_repaint();
            }
        }));
        let (open_id, quit_id) = (open.id().clone(), quit_item.id().clone());
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == open_id {
                window.bring_back();
                ctx.request_repaint();
            } else if event.id == quit_id {
                quit.store(true, Ordering::SeqCst);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx.request_repaint();
            }
        }));
        Some(Tray { icon: tray })
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub struct Tray;

    impl Tray {
        pub fn set_tooltip(&self, _text: &str) {}
    }

    pub fn create(
        _lang: Lang,
        _tooltip: &str,
        _window: MainWindow,
        _ctx: egui::Context,
        _quit: Arc<AtomicBool>,
    ) -> Option<Tray> {
        None
    }
}

pub use imp::{Tray, create};

/// True where File Sorter can live in the notification area.
pub fn supported() -> bool {
    cfg!(windows)
}
