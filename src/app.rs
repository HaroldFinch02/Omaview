use std::path::PathBuf;
use std::rc::Rc;
use libadwaita::prelude::*;
use libadwaita::Application;

use crate::theme::ThemeManager;
use crate::util::discover_images;
use crate::window::MainWindow;

pub struct OmaviewApp {
    app: Application,
}

impl Default for OmaviewApp {
    fn default() -> Self {
        Self::new()
    }
}

impl OmaviewApp {
    pub fn new() -> Self {
        let app = Application::builder()
            .application_id("org.omarchy.omaview")
            .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
            .build();

        Self { app }
    }

    pub fn run(&self) -> glib::ExitCode {
        let theme_manager = Rc::new(ThemeManager::new());
        let main_win_holder = Rc::new(std::cell::RefCell::new(None::<MainWindow>));

        let theme_clone = theme_manager.clone();
        let win_holder_activate = main_win_holder.clone();

        self.app.connect_activate(move |app| {
            let mut holder = win_holder_activate.borrow_mut();
            if holder.is_none() {
                let win = MainWindow::new(app, theme_clone.clone());
                *holder = Some(win);
            }
            if let Some(win) = holder.as_ref() {
                win.show_home();
                win.window().present();
            }
        });

        let theme_cmd = theme_manager.clone();
        let win_holder_cmd = main_win_holder.clone();

        self.app.connect_command_line(move |app, cmdline| {
            let cwd = cmdline.cwd().unwrap_or_else(|| PathBuf::from("."));
            let raw_args = cmdline.arguments();
            let mut resolved_paths = Vec::new();

            for arg in raw_args.into_iter().skip(1) {
                let p = PathBuf::from(arg);
                let full = if p.is_absolute() { p } else { cwd.join(p) };
                resolved_paths.push(full);
            }

            let mut holder = win_holder_cmd.borrow_mut();
            if holder.is_none() {
                let win = MainWindow::new(app, theme_cmd.clone());
                *holder = Some(win);
            }

            if let Some(win) = holder.as_ref() {
                if resolved_paths.is_empty() {
                    win.show_home();
                } else {
                    let (images, initial_idx) = discover_images(&resolved_paths);
                    if images.is_empty() {
                        win.show_home();
                    } else {
                        win.load_initial_paths(images, initial_idx);
                        win.show_viewer();
                    }
                }
                win.window().present();
            }

            0.into()
        });

        self.app.run()
    }
}
