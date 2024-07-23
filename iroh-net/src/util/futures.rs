//! TODO(matheus23): DOCS

#[cfg(not(feature = "wasm"))]
pub use futures_util::stream::BoxStream;
#[cfg(feature = "wasm")]
pub use futures_util::stream::LocalBoxStream as BoxStream;

#[cfg(not(feature = "wasm"))]
pub use futures_lite::future::Boxed as BoxFuture;
#[cfg(feature = "wasm")]
pub use futures_lite::future::BoxedLocal as BoxFuture;
