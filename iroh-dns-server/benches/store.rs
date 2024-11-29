use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion};

use iroh::dns::node_info::NodeInfo;
use iroh::key::SecretKey;
use iroh_dns_server::store::SignedPacketStore;

pub fn memory_benchmark(c: &mut Criterion) {
    let key = SecretKey::generate();
    let node_id = key.public();
    let node_info = NodeInfo::new(
        node_id,
        Some("https://my-relay.com".parse().unwrap()),
        ["127.0.0.1:1245".parse().unwrap()].into_iter().collect(),
    );
    let ttl = 1024;
    let packet_0 = node_info.to_pkarr_signed_packet(&key, ttl).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let packet_1 = node_info.to_pkarr_signed_packet(&key, ttl).unwrap();
    assert!(packet_0.timestamp() < packet_1.timestamp());

    c.bench_function("memory_upsert_noop", |b| {
        let store = SignedPacketStore::in_memory().unwrap();
        let res = store.upsert(packet_1.clone());
        assert_eq!(res.unwrap(), true); // first insert
        b.iter(|| {
            let res = store.upsert(packet_0.clone());
            assert_eq!(res.unwrap(), false);
        })
    });

    c.bench_function("memory_upsert_change", |b| {
        let store = SignedPacketStore::in_memory().unwrap();
        let res = store.upsert(packet_1.clone());
        assert_eq!(res.unwrap(), true); // first insert

        b.iter_custom(|iters| {
            let mut total = Duration::default();

            for _i in 0..iters {
                let packet = node_info.to_pkarr_signed_packet(&key, ttl).unwrap();

                // do not measure packet creation
                let start = Instant::now();
                let res = store.upsert(packet.clone());
                total += start.elapsed();
                assert_eq!(res.unwrap(), true);
            }

            total
        })
    });
}

pub fn persistent_benchmark(c: &mut Criterion) {
    let key = SecretKey::generate();
    let node_id = key.public();
    let node_info = NodeInfo::new(
        node_id,
        None,
        ["127.0.0.1:1245".parse().unwrap()].into_iter().collect(),
    );
    let ttl = 1024;
    let packet_0 = node_info.to_pkarr_signed_packet(&key, ttl).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let packet_1 = node_info.to_pkarr_signed_packet(&key, ttl).unwrap();
    assert!(packet_0.timestamp() < packet_1.timestamp());

    c.bench_function("persistent_upsert_noop", |b| {
        let dir = tempfile::tempdir().unwrap();
        let store = SignedPacketStore::persistent(dir.path().join("upsert-noop-bench")).unwrap();
        let res = store.upsert(packet_1.clone());
        assert_eq!(res.unwrap(), true); // first insert
        b.iter(|| {
            let res = store.upsert(packet_0.clone());
            assert_eq!(res.unwrap(), false);
        })
    });

    c.bench_function("persistent_upsert_change", |b| {
        let dir = tempfile::tempdir().unwrap();
        let store = SignedPacketStore::persistent(dir.path().join("upsert-change-bench")).unwrap();
        let res = store.upsert(packet_1.clone());
        assert_eq!(res.unwrap(), true); // first insert

        b.iter_custom(|iters| {
            let mut total = Duration::default();

            for _i in 0..iters {
                let packet = node_info.to_pkarr_signed_packet(&key, ttl).unwrap();

                // do not measure packet creation
                let start = Instant::now();
                let res = store.upsert(packet.clone());
                total += start.elapsed();
                assert_eq!(res.unwrap(), true);
            }

            total
        })
    });
}

criterion_group!(benches, memory_benchmark, persistent_benchmark);
criterion_main!(benches);
