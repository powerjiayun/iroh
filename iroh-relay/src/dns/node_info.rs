//! Support for handling DNS resource records for dialing by [`NodeId`].
//!
//! Dialing by [`NodeId`] is supported by iroh nodes publishing [Pkarr] records to DNS
//! servers or the Mainline DHT.  This module supports creating and parsing these records.
//!
//! DNS records are published under the following names:
//!
//! `_iroh.<z32-node-id>.<origin-domain> TXT`
//!
//! - `_iroh` is the record name as defined by [`IROH_TXT_NAME`].
//!
//! - `<z32-node-id>` is the [z-base-32] encoding of the [`NodeId`].
//!
//! - `<origin-domain>` is the domain name of the publishing DNS server,
//!   [`N0_DNS_NODE_ORIGIN_PROD`] is the server operated by number0 for production.
//!   [`N0_DNS_NODE_ORIGIN_STAGING`] is the server operated by number0 for testing.
//!
//! - `TXT` is the DNS record type.
//!
//! The returned TXT records must contain a string value of the form `key=value` as defined
//! in [RFC1464].  The following attributes are defined:
//!
//! - `relay=<url>`: The home [`RelayUrl`] of this node.
//!
//! - `addr=<addr> <addr>`: A space-separated list of sockets addresses for this iroh node.
//!   Each address is an IPv4 or IPv6 address with a port.
//!
//! [Pkarr]: https://app.pkarr.org
//! [z-base-32]: https://philzimmermann.com/docs/human-oriented-base-32-encoding.txt
//! [RFC1464]: https://www.rfc-editor.org/rfc/rfc1464
//! [`RelayUrl`]: iroh_base::RelayUrl
//! [`N0_DNS_NODE_ORIGIN_PROD`]: crate::dns::N0_DNS_NODE_ORIGIN_PROD
//! [`N0_DNS_NODE_ORIGIN_STAGING`]: crate::dns::N0_DNS_NODE_ORIGIN_STAGING

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::{self, Display},
    hash::Hash,
    net::SocketAddr,
    str::FromStr,
};

use anyhow::{anyhow, Result};
use hickory_resolver::{proto::ProtoError, Name};
use iroh_base::{NodeAddr, NodeId, RelayUrl, SecretKey};
use tracing::warn;
use url::Url;

use crate::{defaults::timeouts::DNS_TIMEOUT, dns::DnsResolver};

/// The DNS name for the iroh TXT record.
pub const IROH_TXT_NAME: &str = "_iroh";

/// Extension methods for [`NodeId`] to encode to and decode from [`z32`],
/// which is the encoding used in [`pkarr`] domain names.
pub trait NodeIdExt {
    /// Encodes a [`NodeId`] in [`z-base-32`] encoding.
    ///
    /// [z-base-32]: https://philzimmermann.com/docs/human-oriented-base-32-encoding.txt
    fn to_z32(&self) -> String;

    /// Parses a [`NodeId`] from [`z-base-32`] encoding.
    ///
    /// [z-base-32]: https://philzimmermann.com/docs/human-oriented-base-32-encoding.txt
    fn from_z32(s: &str) -> Result<NodeId>;
}

impl NodeIdExt for NodeId {
    fn to_z32(&self) -> String {
        z32::encode(self.as_bytes())
    }

    fn from_z32(s: &str) -> Result<NodeId> {
        let bytes = z32::decode(s.as_bytes()).map_err(|_| anyhow!("invalid z32"))?;
        let bytes: &[u8; 32] = &bytes.try_into().map_err(|_| anyhow!("not 32 bytes long"))?;
        let node_id = NodeId::from_bytes(bytes)?;
        Ok(node_id)
    }
}

/// The information that can be published about a node in discovery services.
///
/// This includes an optional [`RelayUrl`] and a set of direct addresses.
///
/// The fields are not public, but there are setter and getter functions for all fields
/// contained in the struct.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct NodeData {
    /// URL of the home relay of this node.
    pub relay_url: Option<RelayUrl>,
    /// Direct addresses where this node can be reached.
    pub direct_addresses: BTreeSet<SocketAddr>,
    /// Optional user-defined [`UserData`] for this node.
    pub user_data: Option<UserData>,
}

impl NodeData {
    /// Creates a new [`NodeData`] with a relay URL and a set of direct addresses.
    pub fn new(relay_url: Option<RelayUrl>, direct_addresses: BTreeSet<SocketAddr>) -> Self {
        Self {
            relay_url,
            direct_addresses,
            user_data: None,
        }
    }

    /// Sets the relay URL.
    pub fn with_relay_url(mut self, relay_url: impl Into<RelayUrl>) -> Self {
        self.relay_url = Some(relay_url.into());
        self
    }

    /// Sets the direct addresses.
    pub fn with_direct_addrs(mut self, direct_addrs: BTreeSet<SocketAddr>) -> Self {
        self.direct_addresses = direct_addrs;
        self
    }

    /// Sets the user data.
    pub fn with_user_data(mut self, user_data: UserData) -> Self {
        self.user_data = Some(user_data);
        self
    }

    /// Removes all direct addresses.
    pub fn clear_direct_addresses(&mut self) {
        self.direct_addresses = Default::default();
    }

    /// Converts into a [`NodeAddr`].
    pub fn into_node_addr(self, node_id: NodeId) -> NodeAddr {
        NodeAddr {
            node_id,
            relay_url: self.relay_url,
            direct_addresses: self.direct_addresses,
        }
    }
}

impl From<NodeAddr> for NodeData {
    fn from(node_addr: NodeAddr) -> Self {
        Self {
            relay_url: node_addr.relay_url,
            direct_addresses: node_addr.direct_addresses,
            user_data: None,
        }
    }
}

// User defined data that can be published and resolved in Discovery.
///
/// Below the hood this is a utf-8 String that is less than or equal to 253 bytes.
///
/// Iroh does not keep track of or examine user defined data.
///
/// TODO: more explanation of how to get the user defined data of a node.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct UserData(String);

/// The max byte length allowed for user defined data.
pub const USER_DATA_MAX_LENGTH: usize = 255;

/// Error returned when an input value is too long for [`UserDefinedData`].
#[derive(Debug, thiserror::Error)]
#[error("User-defined data exceeds max length")]
pub struct MaxLengthExceededError;

impl TryFrom<String> for UserData {
    type Error = MaxLengthExceededError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.as_bytes().len() > USER_DATA_MAX_LENGTH {
            Err(MaxLengthExceededError)
        } else {
            Ok(Self(value))
        }
    }
}

impl FromStr for UserData {
    type Err = MaxLengthExceededError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.as_bytes().len() > USER_DATA_MAX_LENGTH {
            Err(MaxLengthExceededError)
        } else {
            Ok(Self(s.to_string()))
        }
    }
}

impl fmt::Display for UserData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for UserData {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Information about the iroh node contained in an [`IROH_TXT_NAME`] TXT resource record.
#[derive(derive_more::Debug, Clone, Eq, PartialEq)]
pub struct NodeInfo {
    /// The [`NodeId`].
    pub node_id: NodeId,
    /// The information published about the node.
    pub data: NodeData,
}

impl From<TxtAttrs<IrohAttr>> for NodeInfo {
    fn from(attrs: TxtAttrs<IrohAttr>) -> Self {
        (&attrs).into()
    }
}

impl From<&TxtAttrs<IrohAttr>> for NodeInfo {
    fn from(attrs: &TxtAttrs<IrohAttr>) -> Self {
        let node_id = attrs.node_id();
        let attrs = attrs.attrs();
        let relay_url = attrs
            .get(&IrohAttr::Relay)
            .into_iter()
            .flatten()
            .next()
            .and_then(|s| Url::parse(s).ok());
        let direct_addresses = attrs
            .get(&IrohAttr::Addr)
            .into_iter()
            .flatten()
            .filter_map(|s| SocketAddr::from_str(s).ok())
            .collect();
        let user_data = attrs
            .get(&IrohAttr::UserData)
            .into_iter()
            .flatten()
            .next()
            .and_then(|s| UserData::from_str(s).ok());
        let data = NodeData {
            relay_url: relay_url.map(Into::into),
            direct_addresses,
            user_data,
        };
        Self { node_id, data }
    }
}

impl From<NodeInfo> for NodeAddr {
    fn from(value: NodeInfo) -> Self {
        value.data.into_node_addr(value.node_id)
    }
}

impl NodeInfo {
    /// Creates a new [`NodeInfo`] from its parts.
    pub fn new(node_id: NodeId, data: NodeData) -> Self {
        Self { node_id, data }
    }

    fn to_attrs(&self) -> TxtAttrs<IrohAttr> {
        self.into()
    }

    /// Parses a [`NodeInfo`] from a TXT records lookup.
    pub fn from_txt_lookup(lookup: super::TxtLookup) -> Result<Self> {
        let attrs = TxtAttrs::from_txt_lookup(lookup)?;
        Ok(attrs.into())
    }

    /// Parses a [`NodeInfo`] from a [`pkarr::SignedPacket`].
    pub fn from_pkarr_signed_packet(packet: &pkarr::SignedPacket) -> Result<Self> {
        let attrs = TxtAttrs::from_pkarr_signed_packet(packet)?;
        Ok(attrs.into())
    }

    /// Creates a [`pkarr::SignedPacket`].
    ///
    /// This constructs a DNS packet and signs it with a [`SecretKey`].
    pub fn to_pkarr_signed_packet(
        &self,
        secret_key: &SecretKey,
        ttl: u32,
    ) -> Result<pkarr::SignedPacket> {
        self.to_attrs().to_pkarr_signed_packet(secret_key, ttl)
    }

    /// Converts into a list of `{key}={value}` strings.
    pub fn to_txt_strings(&self) -> Vec<String> {
        self.to_attrs().to_txt_strings().collect()
    }
}

/// Parses a [`NodeId`] from iroh DNS name.
///
/// Takes a [`hickory_resolver::proto::rr::Name`] DNS name and expects the first label to be
/// [`IROH_TXT_NAME`] and the second label to be a z32 encoded [`NodeId`]. Ignores
/// subsequent labels.
fn node_id_from_hickory_name(name: &hickory_resolver::proto::rr::Name) -> Option<NodeId> {
    if name.num_labels() < 2 {
        return None;
    }
    let mut labels = name.iter();
    let label = std::str::from_utf8(labels.next().expect("num_labels checked")).ok()?;
    if label != IROH_TXT_NAME {
        return None;
    }
    let label = std::str::from_utf8(labels.next().expect("num_labels checked")).ok()?;
    let node_id = NodeId::from_z32(label).ok()?;
    Some(node_id)
}

/// The attributes supported by iroh for [`IROH_TXT_NAME`] DNS resource records.
///
/// The resource record uses the lower-case names.
#[derive(
    Debug, strum::Display, strum::AsRefStr, strum::EnumString, Hash, Eq, PartialEq, Ord, PartialOrd,
)]
#[strum(serialize_all = "kebab-case")]
pub(super) enum IrohAttr {
    /// URL of home relay.
    Relay,
    /// Direct address.
    Addr,
    /// User-defined data
    UserData,
}

/// Attributes parsed from [`IROH_TXT_NAME`] TXT records.
///
/// This struct is generic over the key type. When using with [`String`], this will parse
/// all attributes. Can also be used with an enum, if it implements [`FromStr`] and
/// [`Display`].
#[derive(Debug)]
pub(super) struct TxtAttrs<T> {
    node_id: NodeId,
    attrs: BTreeMap<T, Vec<String>>,
}

impl From<&NodeInfo> for TxtAttrs<IrohAttr> {
    fn from(info: &NodeInfo) -> Self {
        let mut attrs = vec![];
        if let Some(relay_url) = &info.data.relay_url {
            attrs.push((IrohAttr::Relay, relay_url.to_string()));
        }
        for addr in &info.data.direct_addresses {
            attrs.push((IrohAttr::Addr, addr.to_string()));
        }
        Self::from_parts(info.node_id, attrs.into_iter())
    }
}

impl<T: FromStr + Display + Hash + Ord> TxtAttrs<T> {
    /// Creates [`TxtAttrs`] from a node id and an iterator of key-value pairs.
    pub(super) fn from_parts(node_id: NodeId, pairs: impl Iterator<Item = (T, String)>) -> Self {
        let mut attrs: BTreeMap<T, Vec<String>> = BTreeMap::new();
        for (k, v) in pairs {
            attrs.entry(k).or_default().push(v);
        }
        Self { attrs, node_id }
    }

    /// Creates [`TxtAttrs`] from a node id and an iterator of "{key}={value}" strings.
    pub(super) fn from_strings(
        node_id: NodeId,
        strings: impl Iterator<Item = String>,
    ) -> Result<Self> {
        let mut attrs: BTreeMap<T, Vec<String>> = BTreeMap::new();
        for s in strings {
            let mut parts = s.split('=');
            let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
                continue;
            };
            let Ok(attr) = T::from_str(key) else {
                continue;
            };
            attrs.entry(attr).or_default().push(value.to_string());
        }
        Ok(Self { attrs, node_id })
    }

    async fn lookup(resolver: &DnsResolver, name: Name) -> Result<Self> {
        let name = ensure_iroh_txt_label(name)?;
        let lookup = resolver.lookup_txt(name, DNS_TIMEOUT).await?;
        let attrs = Self::from_txt_lookup(lookup)?;
        Ok(attrs)
    }

    /// Looks up attributes by [`NodeId`] and origin domain.
    pub(super) async fn lookup_by_id(
        resolver: &DnsResolver,
        node_id: &NodeId,
        origin: &str,
    ) -> Result<Self> {
        let name = node_domain(node_id, origin)?;
        TxtAttrs::lookup(resolver, name).await
    }

    /// Looks up attributes by DNS name.
    pub(super) async fn lookup_by_name(resolver: &DnsResolver, name: &str) -> Result<Self> {
        let name = Name::from_str(name)?;
        TxtAttrs::lookup(resolver, name).await
    }

    /// Returns the parsed attributes.
    pub(super) fn attrs(&self) -> &BTreeMap<T, Vec<String>> {
        &self.attrs
    }

    /// Returns the node id.
    pub(super) fn node_id(&self) -> NodeId {
        self.node_id
    }

    /// Parses a [`pkarr::SignedPacket`].
    pub(super) fn from_pkarr_signed_packet(packet: &pkarr::SignedPacket) -> Result<Self> {
        use pkarr::dns::{
            rdata::RData,
            {self},
        };
        let pubkey = packet.public_key();
        let pubkey_z32 = pubkey.to_z32();
        let node_id = NodeId::from(*pubkey.verifying_key());
        let zone = dns::Name::new(&pubkey_z32)?;
        let inner = packet.packet();
        let txt_data = inner.answers.iter().filter_map(|rr| match &rr.rdata {
            RData::TXT(txt) => match rr.name.without(&zone) {
                Some(name) if name.to_string() == IROH_TXT_NAME => Some(txt),
                Some(_) | None => None,
            },
            _ => None,
        });

        let txt_strs = txt_data.filter_map(|s| String::try_from(s.clone()).ok());
        Self::from_strings(node_id, txt_strs)
    }

    /// Parses a TXT records lookup.
    pub(super) fn from_txt_lookup(lookup: super::TxtLookup) -> Result<Self> {
        let queried_node_id = node_id_from_hickory_name(lookup.0.query().name())
            .ok_or_else(|| anyhow!("invalid DNS answer: not a query for _iroh.z32encodedpubkey"))?;

        let strings = lookup.0.as_lookup().record_iter().filter_map(|record| {
            match node_id_from_hickory_name(record.name()) {
                // Filter out only TXT record answers that match the node_id we searched for.
                Some(n) if n == queried_node_id => match record.data().as_txt() {
                    Some(txt) => Some(txt.to_string()),
                    None => {
                        warn!(
                            ?queried_node_id,
                            data = ?record.data(),
                            "unexpected record type for DNS discovery query"
                        );
                        None
                    }
                },
                Some(answered_node_id) => {
                    warn!(
                        ?queried_node_id,
                        ?answered_node_id,
                        "unexpected node ID answered for DNS query"
                    );
                    None
                }
                None => {
                    warn!(
                        ?queried_node_id,
                        name = ?record.name(),
                        "unexpected answer record name for DNS query"
                    );
                    None
                }
            }
        });

        Self::from_strings(queried_node_id, strings)
    }

    fn to_txt_strings(&self) -> impl Iterator<Item = String> + '_ {
        self.attrs
            .iter()
            .flat_map(move |(k, vs)| vs.iter().map(move |v| format!("{k}={v}")))
    }

    /// Creates a [`pkarr::SignedPacket`]
    ///
    /// This constructs a DNS packet and signs it with a [`SecretKey`].
    pub(super) fn to_pkarr_signed_packet(
        &self,
        secret_key: &SecretKey,
        ttl: u32,
    ) -> Result<pkarr::SignedPacket> {
        let packet = self.to_pkarr_dns_packet(ttl)?;
        let keypair = pkarr::Keypair::from_secret_key(&secret_key.to_bytes());
        let signed_packet = pkarr::SignedPacket::from_packet(&keypair, &packet)?;
        Ok(signed_packet)
    }

    fn to_pkarr_dns_packet(&self, ttl: u32) -> Result<pkarr::dns::Packet<'static>> {
        use pkarr::dns::{self, rdata};
        let name = dns::Name::new(IROH_TXT_NAME)?.into_owned();

        let mut packet = dns::Packet::new_reply(0);
        for s in self.to_txt_strings() {
            let mut txt = rdata::TXT::new();
            txt.add_string(&s)?;
            let rdata = rdata::RData::TXT(txt.into_owned());
            packet.answers.push(dns::ResourceRecord::new(
                name.clone(),
                dns::CLASS::IN,
                ttl,
                rdata,
            ));
        }
        Ok(packet)
    }
}

fn ensure_iroh_txt_label(name: Name) -> Result<Name, ProtoError> {
    if name.iter().next() == Some(IROH_TXT_NAME.as_bytes()) {
        Ok(name)
    } else {
        Name::parse(IROH_TXT_NAME, Some(&name))
    }
}

fn node_domain(node_id: &NodeId, origin: &str) -> Result<Name> {
    let domain = format!("{}.{}", NodeId::to_z32(node_id), origin);
    let domain = Name::from_str(&domain)?;
    Ok(domain)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, str::FromStr, sync::Arc};

    use hickory_resolver::{
        lookup::Lookup,
        proto::{
            op::Query,
            rr::{
                rdata::{A, TXT},
                RData, Record, RecordType,
            },
        },
        Name,
    };
    use iroh_base::{NodeId, SecretKey};
    use testresult::TestResult;

    use super::{NodeData, NodeIdExt, NodeInfo};

    #[test]
    fn txt_attr_roundtrip() {
        let node_data = NodeData::new(
            Some("https://example.com".parse().unwrap()),
            ["127.0.0.1:1234".parse().unwrap()].into_iter().collect(),
        );
        let node_id = "vpnk377obfvzlipnsfbqba7ywkkenc4xlpmovt5tsfujoa75zqia"
            .parse()
            .unwrap();
        let expected = NodeInfo::new(node_id, node_data);
        let attrs = expected.to_attrs();
        let actual = NodeInfo::from(&attrs);
        assert_eq!(expected, actual);
    }

    #[test]
    fn signed_packet_roundtrip() {
        let secret_key =
            SecretKey::from_str("vpnk377obfvzlipnsfbqba7ywkkenc4xlpmovt5tsfujoa75zqia").unwrap();
        let node_data = NodeData::new(
            Some("https://example.com".parse().unwrap()),
            ["127.0.0.1:1234".parse().unwrap()].into_iter().collect(),
        );
        let expected = NodeInfo::new(secret_key.public(), node_data);
        let packet = expected.to_pkarr_signed_packet(&secret_key, 30).unwrap();
        let actual = NodeInfo::from_pkarr_signed_packet(&packet).unwrap();
        assert_eq!(expected, actual);
    }

    /// There used to be a bug where uploading a NodeAddr with more than only exactly
    /// one relay URL or one publicly reachable IP addr would prevent connection
    /// establishment.
    ///
    /// The reason was that only the first address was parsed (e.g. 192.168.96.145 in
    /// this example), which could be a local, unreachable address.
    #[test]
    fn test_from_hickory_lookup() -> TestResult {
        let name = Name::from_utf8(
            "_iroh.dgjpkxyn3zyrk3zfads5duwdgbqpkwbjxfj4yt7rezidr3fijccy.dns.iroh.link.",
        )?;
        let query = Query::query(name.clone(), RecordType::TXT);
        let records = [
            Record::from_rdata(
                name.clone(),
                30,
                RData::TXT(TXT::new(vec!["addr=192.168.96.145:60165".to_string()])),
            ),
            Record::from_rdata(
                name.clone(),
                30,
                RData::TXT(TXT::new(vec!["addr=213.208.157.87:60165".to_string()])),
            ),
            // Test a record with mismatching record type (A instead of TXT). It should be filtered out.
            Record::from_rdata(name.clone(), 30, RData::A(A::new(127, 0, 0, 1))),
            // Test a record with a mismatching name
            Record::from_rdata(
                Name::from_utf8(format!(
                    "_iroh.{}.dns.iroh.link.",
                    NodeId::from_str(
                        // Another NodeId
                        "a55f26132e5e43de834d534332f66a20d480c3e50a13a312a071adea6569981e"
                    )?
                    .to_z32()
                ))?,
                30,
                RData::TXT(TXT::new(vec![
                    "relay=https://euw1-1.relay.iroh.network./".to_string()
                ])),
            ),
            // Test a record with a completely different name
            Record::from_rdata(
                Name::from_utf8("dns.iroh.link.")?,
                30,
                RData::TXT(TXT::new(vec![
                    "relay=https://euw1-1.relay.iroh.network./".to_string()
                ])),
            ),
            Record::from_rdata(
                name.clone(),
                30,
                RData::TXT(TXT::new(vec![
                    "relay=https://euw1-1.relay.iroh.network./".to_string()
                ])),
            ),
        ];
        let lookup = Lookup::new_with_max_ttl(query, Arc::new(records));
        let lookup = hickory_resolver::lookup::TxtLookup::from(lookup);

        let node_info = NodeInfo::from_txt_lookup(lookup.into())?;
        let expected_node_info = NodeInfo::new(
            NodeId::from_str("1992d53c02cdc04566e5c0edb1ce83305cd550297953a047a445ea3264b54b18")
                .unwrap(),
            NodeData::new(
                Some("https://euw1-1.relay.iroh.network./".parse()?),
                BTreeSet::from([
                    "192.168.96.145:60165".parse()?,
                    "213.208.157.87:60165".parse()?,
                ]),
            ),
        );
        assert_eq!(node_info, expected_node_info);

        Ok(())
    }
}
