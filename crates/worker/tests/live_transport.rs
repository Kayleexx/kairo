#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use kairo_runtime::StreamRelay;
use kairo_worker::live_transport::{EdgeIdentity, LiveEndpoint};

#[tokio::test]
async fn relays_chunks_before_the_producer_finishes() {
    let server = LiveEndpoint::bind().expect("server should bind");
    let client = LiveEndpoint::bind().expect("client should bind");
    let addr = server.local_addr().expect("server should have an address");

    let identity = EdgeIdentity {
        run_id: "run-1",
        edge_id: "step-a-to-step-b",
        epoch: 7,
    };

    let (producer, source) = StreamRelay::bounded(256 * 1024);
    let (sink, consumer) = StreamRelay::bounded(256 * 1024);
    let serve = tokio::spawn(async move { server.serve_relay(identity, source).await });
    let fetch = tokio::spawn(async move { client.fetch_relay(addr, identity, sink).await });

    producer
        .send(vec![1; 64 * 1024])
        .await
        .expect("send first chunk");
    let first = consumer.next().await.expect("receive first chunk");
    assert_eq!(first.as_deref().map(<[u8]>::len), Some(64 * 1024));
    producer
        .send(vec![2; 64 * 1024])
        .await
        .expect("send second chunk");
    producer.close();
    let second = consumer.next().await.expect("receive second chunk");
    assert_eq!(second.as_deref().map(<[u8]>::len), Some(64 * 1024));
    assert_eq!(consumer.next().await.expect("clean eof"), None);
    let sent = serve.await.expect("server task").expect("server relay");
    let received = fetch.await.expect("client task").expect("client relay");
    assert_eq!(sent.bytes, 128 * 1024);
    assert_eq!(received.bytes, 128 * 1024);
    assert!(sent.first_byte.is_some());
    assert!(received.first_byte.is_some());
}

#[tokio::test]
async fn rejects_a_fetch_for_a_different_edge_than_what_is_being_offered() {
    let server = LiveEndpoint::bind().expect("server should bind");
    let client = LiveEndpoint::bind().expect("client should bind");
    let addr = server.local_addr().expect("server should have an address");

    let offered = EdgeIdentity {
        run_id: "run-1",
        edge_id: "step-a-to-step-b",
        epoch: 7,
    };
    let requested = EdgeIdentity {
        run_id: "run-1",
        edge_id: "step-a-to-step-b",
        // a stale epoch -- the same run/edge, but a fenced-out caller.
        epoch: 6,
    };

    let (_producer, source) = StreamRelay::bounded(64 * 1024);
    let (sink, _consumer) = StreamRelay::bounded(64 * 1024);
    let serve = tokio::spawn(async move { server.serve_relay(offered, source).await });

    let result = client.fetch_relay(addr, requested, sink).await;
    assert!(
        result.is_err(),
        "a fenced/mismatched fetch must be rejected"
    );

    // the server side observes the same rejection, not a silent success.
    assert!(serve.await.expect("serve task should not panic").is_err());
}

#[tokio::test]
async fn relay_capacity_backpressures_the_transport_sender() {
    let (producer, source) = StreamRelay::bounded(4);
    producer
        .send(vec![1_u8; 4])
        .await
        .expect("first chunk should fill the relay");
    let blocked = {
        let producer = &producer;
        async move { producer.send(vec![2_u8; 4]).await }
    };
    tokio::pin!(blocked);
    let waiter = producer.wait_for_blocked_sends(1);
    tokio::pin!(waiter);
    tokio::select! {
        result = &mut blocked => panic!("full relay accepted a chunk: {result:?}"),
        result = &mut waiter => result.expect("send should block at capacity"),
    }
    assert_eq!(source.metrics().peak_buffered_bytes, 4);
    assert_eq!(
        source.next().await.expect("first chunk"),
        Some(vec![1_u8; 4])
    );
    blocked
        .await
        .expect("consumer capacity should resume the sender");
    assert_eq!(
        source.next().await.expect("second chunk"),
        Some(vec![2_u8; 4])
    );
}
