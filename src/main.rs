mod albums;
mod app;
mod crop_bar;
mod filmstrip;
mod home;
mod image_loader;
mod image_ops;
mod metadata;
mod shortcuts;
mod theme;
mod toolbar;
mod util;
mod viewport;
mod window;

use app::OmaviewApp;

fn main() -> glib::ExitCode {
    glib::set_prgname(Some("omaview"));
    glib::set_application_name("Omaview");
    let _ = gtk4::init();
    let app = OmaviewApp::new();
    app.run()
}
