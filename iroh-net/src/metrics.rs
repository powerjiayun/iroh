//! Co-locating all of the iroh-net metrics structs
pub use crate::magicsock::Metrics as MagicsockMetrics;
#[cfg(feature = "native")]
pub use crate::netcheck::Metrics as NetcheckMetrics;
#[cfg(feature = "native")]
pub use crate::portmapper::Metrics as PortmapMetrics;
pub use crate::relay::Metrics as RelayMetrics;
