//! XDAW desktop application.

mod app;
mod arr_strip;
mod arrangement;
mod arrec;
mod browser;
mod capture;
mod clip_editor;
mod devices;
mod dialogs;
mod filedrop;
mod mixer;
mod runtime;
mod script;
mod session;
mod theme;
mod timeline;
mod transport;

fn main() -> eframe::Result {
    let open = std::env::args().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]).with_min_inner_size([900.0, 600.0]).with_title("XDAW"),
        ..Default::default()
    };
    eframe::run_native("XDAW", options, Box::new(move |cc| Ok(Box::new(app::XdawApp::new(cc, open)))))
}
