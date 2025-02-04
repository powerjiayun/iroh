/// Delay used to perform staggered dns queries.
pub(crate) const DNS_STAGGERING_MS: &[u64] = &[200, 300];

#[cfg(test)]
pub(crate) mod tests {
    use iroh_relay::dns::{default_resolver, DnsResolver};

    /// Get a DNS resolver suitable for testing.
    pub fn resolver() -> &'static DnsResolver {
        default_resolver()
    }
}
