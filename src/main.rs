//! snapit: a lightweight macOS menu bar screenshot and annotation tool.

mod app;
mod core;
mod mac;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

fn main() {
    let mtm = MainThreadMarker::new().expect("snapit must start on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    // Menu bar only: no Dock icon, no app menu (LSUIElement also says so).
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app::start(mtm);
    app.run();
}
