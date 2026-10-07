// Shared with the browser client: theme, read-only views, and plain widgets.
pub mod theme;
pub mod view;
pub(crate) mod widgets;

// Desktop app only.
#[cfg(not(target_arch = "wasm32"))]
pub mod app;
#[cfg(not(target_arch = "wasm32"))]
pub mod tasks;

#[cfg(not(target_arch = "wasm32"))]
mod bg;
#[cfg(not(target_arch = "wasm32"))]
mod fuzzy;
#[cfg(not(target_arch = "wasm32"))]
mod info;
#[cfg(not(target_arch = "wasm32"))]
mod keys;
#[cfg(not(target_arch = "wasm32"))]
mod projects;
#[cfg(not(target_arch = "wasm32"))]
mod share;
#[cfg(not(target_arch = "wasm32"))]
mod tags;
