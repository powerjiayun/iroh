#[cfg(not(feature = "wasm"))]
mod native;
#[cfg(feature = "wasm")]
mod wasm;

#[cfg(not(feature = "wasm"))]
pub use native::*;
#[cfg(feature = "wasm")]
pub use wasm::*;
