#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let _ = fast_task::ui::app::run();
}

/// Browser entry point: wasm-bindgen runs `main` when the module loads.
#[cfg(target_arch = "wasm32")]
fn main() {
    fast_task::web::start();
}
