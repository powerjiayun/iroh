#![allow(missing_docs)]

use std::path::Path;

use anyhow::{Context, Result};
use iroh_metrics::inc;
use pkarr::SignedPacket;
use redb::{backends::InMemoryBackend, Database, ReadableTable, TableDefinition};
use tracing::info;

use crate::{metrics::Metrics, util::PublicKeyBytes};

pub type SignedPacketsKey = [u8; 32];
const SIGNED_PACKETS_TABLE: TableDefinition<&SignedPacketsKey, &[u8]> =
    TableDefinition::new("signed-packets-1");

#[derive(Debug)]
pub struct SignedPacketStore {
    db: Database,
}

impl SignedPacketStore {
    pub fn persistent(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        info!("loading packet database from {}", path.to_string_lossy());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create database directory at {}",
                    path.to_string_lossy()
                )
            })?;
        }
        let db = Database::builder()
            .create(path)
            .context("failed to open packet database")?;
        Self::open(db)
    }

    pub fn in_memory() -> Result<Self> {
        info!("using in-memory packet database");
        let db = Database::builder().create_with_backend(InMemoryBackend::new())?;
        Self::open(db)
    }

    pub fn open(db: Database) -> Result<Self> {
        let write_tx = db.begin_write()?;
        {
            let _table = write_tx.open_table(SIGNED_PACKETS_TABLE)?;
        }
        write_tx.commit()?;
        Ok(Self { db })
    }

    pub fn upsert(&self, packet: SignedPacket) -> Result<bool> {
        let key = PublicKeyBytes::from_signed_packet(&packet);

        // Optimistic path, only take a read transaction
        let tx = self.db.begin_read()?;
        {
            let table = tx.open_table(SIGNED_PACKETS_TABLE)?;
            if let Some(existing) = get_packet(&table, &key)? {
                if existing.more_recent_than(&packet) {
                    return Ok(false);
                }
            }
        }
        drop(tx);

        // We need to write, now take a write transaction
        let tx = self.db.begin_write()?;
        let mut replaced = false;
        {
            let mut table = tx.open_table(SIGNED_PACKETS_TABLE)?;
            // Check again, in case sth happened in between
            if let Some(existing) = get_packet(&table, &key)? {
                if existing.more_recent_than(&packet) {
                    return Ok(false);
                } else {
                    replaced = true;
                }
            }
            let value = packet.as_bytes();
            table.insert(key.as_bytes(), &value[..])?;
        }
        tx.commit()?;

        if replaced {
            inc!(Metrics, store_packets_updated);
        } else {
            inc!(Metrics, store_packets_inserted);
        }
        Ok(true)
    }

    pub fn get(&self, key: &PublicKeyBytes) -> Result<Option<SignedPacket>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(SIGNED_PACKETS_TABLE)?;
        get_packet(&table, key)
    }

    pub fn remove(&self, key: &PublicKeyBytes) -> Result<bool> {
        let tx = self.db.begin_write()?;
        let updated = {
            let mut table = tx.open_table(SIGNED_PACKETS_TABLE)?;
            let did_remove = table.remove(key.as_bytes())?.is_some();
            #[allow(clippy::let_and_return)]
            did_remove
        };
        tx.commit()?;
        if updated {
            inc!(Metrics, store_packets_removed)
        }
        Ok(updated)
    }
}

fn get_packet(
    table: &impl ReadableTable<&'static SignedPacketsKey, &'static [u8]>,
    key: &PublicKeyBytes,
) -> Result<Option<SignedPacket>> {
    let Some(row) = table.get(key.as_ref())? else {
        return Ok(None);
    };
    let packet = SignedPacket::from_bytes(&row.value().to_vec().into())?;
    Ok(Some(packet))
}

#[cfg(test)]
mod tests {
    use iroh::dns::node_info::NodeInfo;
    use iroh::key::SecretKey;

    #[test]
    fn test_packet_size() {
        let key = SecretKey::generate();
        let node_id = key.public();
        let node_info = NodeInfo::new(
            node_id,
            Some("https://my-relay.com".parse().unwrap()),
            [
                "127.0.0.1:1245".parse().unwrap(),
                "127.0.0.1:1246".parse().unwrap(),
                "127.0.0.1:1247".parse().unwrap(),
                "127.0.0.1:1248".parse().unwrap(),
                "[::]:1241".parse().unwrap(),
                "[::]:1242".parse().unwrap(),
                "[::]:1243".parse().unwrap(),
                "[::]:1244".parse().unwrap(),
            ]
            .into_iter()
            .collect(),
        );
        let ttl = 1024;
        let packet = node_info.to_pkarr_signed_packet(&key, ttl).unwrap();

        assert_eq!(packet.as_bytes().len(), 450);
    }
}
