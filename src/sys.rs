#[cfg(not(target_arch = "wasm32"))]
mod stdlib;
#[cfg(not(target_arch = "wasm32"))]
pub use stdlib::*;

#[cfg(target_arch = "wasm32")]
mod wasm;
#[cfg(target_arch = "wasm32")]
pub use wasm::*;