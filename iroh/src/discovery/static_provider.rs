//! A static node discovery to manually add node addressing information.
//!
//! Often an application might get node addressing information out-of-band in an
//! application-specific way.  [`NodeTicket`]'s are one common way used to achieve this.
//! This "static" addressing information is often only usable for a limited time so needs to
//! be able to be removed again once know it is no longer useful.
//!
//! This is where the [`StaticProvider`] is useful: it allows applications to add and
//! retract node addressing information that is otherwise out-of-band to iroh.
//!
//! [`NodeTicket`]: https://docs.rs/iroh-base/latest/iroh_base/ticket/struct.NodeTicket

use std::{
    collections::{btree_map::Entry, BTreeMap},
    sync::{Arc, RwLock},
};

use iroh_base::{NodeAddr, NodeId};
use n0_future::{
    stream::{self, StreamExt},
    time::SystemTime,
};

use super::{Discovery, DiscoveryItem, NodeData, UserData};

/// A static node discovery to manually add node addressing information.
///
/// Often an application might get node addressing information out-of-band in an
/// application-specific way.  [`NodeTicket`]'s are one common way used to achieve this.
/// This "static" addressing information is often only usable for a limited time so needs to
/// be able to be removed again once know it is no longer useful.
///
/// This is where the [`StaticProvider`] is useful: it allows applications to add and
/// retract node addressing information that is otherwise out-of-band to iroh.
///
/// # Examples
///
/// ```rust
/// use iroh::{discovery::static_provider::StaticProvider, Endpoint, NodeAddr};
/// use iroh_base::SecretKey;
///
/// # #[tokio::main]
/// # async fn main() -> anyhow::Result<()> {
/// // Create the discovery service and endpoint.
/// let discovery = StaticProvider::new();
///
/// let _ep = Endpoint::builder()
///     .add_discovery({
///         let discovery = discovery.clone();
///         move |_| Some(discovery)
///     })
///     .bind()
///     .await?;
///
/// /// Sometime later add a RelayUrl for a fake NodeId.
/// let key = SecretKey::from_bytes(&[0u8; 32]); // Do not use fake secret keys!
/// discovery.add_node_addr(NodeAddr {
///     node_id: key.public(),
///     relay_url: Some("https://example.com".parse()?),
///     direct_addresses: Default::default(),
/// });
///
/// # Ok(())
/// # }
/// ```
///
/// [`NodeTicket`]: https://docs.rs/iroh-base/latest/iroh_base/ticket/struct.NodeTicket
#[derive(Debug, Default, Clone)]
#[repr(transparent)]
pub struct StaticProvider {
    nodes: Arc<RwLock<BTreeMap<NodeId, NodeInfo>>>,
}

#[derive(Debug)]
struct NodeInfo {
    data: NodeData,
    last_updated: SystemTime,
}

impl StaticProvider {
    /// The provenance string for this discovery implementation.
    ///
    /// This is mostly used for debugging information and allows understanding the origin of
    /// addressing information used by an iroh [`Endpoint`].
    ///
    /// [`Endpoint`]: crate::Endpoint
    pub const PROVENANCE: &'static str = "static_discovery";

    /// Creates a new static discovery instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a static discovery instance from node addresses.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::{net::SocketAddr, str::FromStr};
    ///
    /// use iroh::{discovery::static_provider::StaticProvider, Endpoint, NodeAddr};
    ///
    /// # fn get_addrs() -> Vec<NodeAddr> {
    /// #     Vec::new()
    /// # }
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// // get addrs from somewhere
    /// let addrs = get_addrs();
    ///
    /// // create a StaticProvider from the list of addrs.
    /// let discovery = StaticProvider::from_node_addrs(addrs);
    /// // create an endpoint with the discovery
    /// let endpoint = Endpoint::builder()
    ///     .add_discovery(|_| Some(discovery))
    ///     .bind()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_node_addrs(infos: impl IntoIterator<Item = impl Into<NodeAddr>>) -> Self {
        let res = Self::default();
        for info in infos {
            res.add_node_addr(info, None);
        }
        res
    }

    /// Sets node addressing information for the given node ID.
    ///
    /// This will completely overwrite any existing info for the node.
    pub fn set_node_addr(
        &self,
        node_addr: impl Into<NodeAddr>,
        user_data: Option<UserData>,
    ) -> Option<NodeAddr> {
        let last_updated = SystemTime::now();
        let node_addr: NodeAddr = node_addr.into();
        let node_id = node_addr.node_id;
        let data = NodeData::from(node_addr).with_user_data(user_data);
        let mut guard = self.nodes.write().expect("poisoned");
        let previous = guard.insert(node_id, NodeInfo { data, last_updated });
        previous.map(|x| x.data.into_node_addr(node_id))
    }

    /// Augments node addressing information for the given node ID.
    ///
    /// The provided addressing information is combined with the existing info in the static
    /// provider.  Any new direct addresses are added to those already present while the
    /// relay URL is overwritten.
    pub fn add_node_addr(&self, info: impl Into<NodeAddr>, user_data: Option<UserData>) {
        let node_addr: NodeAddr = info.into();
        let last_updated = SystemTime::now();
        let mut guard = self.nodes.write().expect("poisoned");
        match guard.entry(node_addr.node_id) {
            Entry::Occupied(mut entry) => {
                let existing = entry.get_mut();
                existing
                    .data
                    .direct_addresses
                    .extend(node_addr.direct_addresses);
                existing.data.relay_url = node_addr.relay_url;
                existing.data.user_data = user_data;
                existing.last_updated = last_updated;
            }
            Entry::Vacant(entry) => {
                entry.insert(NodeInfo {
                    data: NodeData::from(node_addr).with_user_data(user_data),
                    last_updated,
                });
            }
        }
    }

    /// Returns node addressing information for the given node ID.
    pub fn get_node_addr(&self, node_id: NodeId) -> Option<NodeAddr> {
        let guard = self.nodes.read().expect("poisoned");
        let info = guard.get(&node_id)?;
        Some(info.data.clone().into_node_addr(node_id))
    }

    /// Returns the optional user-defined data for the given node ID.
    pub fn get_user_data(&self, node_id: NodeId) -> Option<UserData> {
        let guard = self.nodes.read().expect("poisoned");
        let info = guard.get(&node_id)?;
        info.data.user_data.clone()
    }

    /// Removes all node addressing information for the given node ID.
    ///
    /// Any removed information is returned.
    pub fn remove_node_addr(&self, node_id: NodeId) -> Option<NodeAddr> {
        let mut guard = self.nodes.write().expect("poisoned");
        let info = guard.remove(&node_id)?;
        Some(info.data.into_node_addr(node_id))
    }
}

impl Discovery for StaticProvider {
    fn publish(&self, _data: &NodeData) {}

    fn resolve(
        &self,
        _endpoint: crate::Endpoint,
        node_id: NodeId,
    ) -> Option<n0_future::stream::Boxed<anyhow::Result<super::DiscoveryItem>>> {
        let guard = self.nodes.read().expect("poisoned");
        let info = guard.get(&node_id);
        match info {
            Some(node_info) => {
                let last_updated = node_info
                    .last_updated
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .expect("time drift")
                    .as_micros() as u64;
                let item = DiscoveryItem::from_node_data(
                    node_id,
                    node_info.data.clone(),
                    Self::PROVENANCE,
                    Some(last_updated),
                );
                Some(stream::iter(Some(Ok(item))).boxed())
            }
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;
    use iroh_base::SecretKey;
    use testresult::TestResult;

    use super::*;
    use crate::Endpoint;

    #[tokio::test]
    async fn test_basic() -> TestResult {
        let discovery = StaticProvider::new();

        let _ep = Endpoint::builder()
            .add_discovery({
                let discovery = discovery.clone();
                move |_| Some(discovery)
            })
            .bind()
            .await?;

        let key = SecretKey::from_bytes(&[0u8; 32]);
        let addr = NodeAddr {
            node_id: key.public(),
            relay_url: Some("https://example.com".parse()?),
            direct_addresses: Default::default(),
        };
        let user_data = Some("foobar".parse().unwrap());
        discovery.add_node_addr(addr.clone(), user_data);

        let back = discovery.get_node_addr(key.public()).context("no addr")?;

        assert_eq!(back, addr);
        let user_data = discovery.get_user_data(key.public());
        assert_eq!(user_data.map(|d| d.to_string()), Some("foobar".to_string()));

        let removed = discovery
            .remove_node_addr(key.public())
            .context("nothing removed")?;
        assert_eq!(removed, addr);
        let res = discovery.get_node_addr(key.public());
        assert!(res.is_none());
        let user_data = discovery.get_user_data(key.public());
        assert!(user_data.is_none());

        Ok(())
    }
}
