use super::*;

fn resources() -> Arc<NetworkResources> {
    NetworkResources::new(ServerResourceLimits {
        queued_bytes_per_peer: 1024,
        queued_bytes_total: 1536,
        ..ServerResourceLimits::default()
    })
}

#[tokio::test]
async fn reliable_frame_budget_includes_both_delimiter_bytes_at_the_exact_boundary() {
    let resources = resources();
    let (sender, mut receiver) = client_event_queue_with_budget(
        ServerOutboundBackpressureMetrics::default(),
        resources.peer_budget(),
    );
    assert_eq!(
        sender.send_reliable_line("x".repeat(1022)).await,
        ClientEventSendOutcome::Sent
    );
    assert_eq!(resources.snapshot().queued_bytes, 1024);
    assert_eq!(receiver.receive_protocol_line().await.unwrap().len(), 1022);
    assert_eq!(resources.snapshot().queued_bytes, 0);
    assert_eq!(
        sender.send_reliable_line("x".repeat(1023)).await,
        ClientEventSendOutcome::Overloaded
    );
    assert_eq!(resources.snapshot().queued_bytes, 0);
}

#[tokio::test]
async fn queued_bytes_follow_coalescing_write_ownership_and_receiver_drop() {
    let resources = resources();
    let metrics = ServerOutboundBackpressureMetrics::default();
    let (sender, mut receiver) =
        client_event_queue_with_budget(metrics.clone(), resources.peer_budget());
    assert_eq!(
        sender.send_periodic_state("a".repeat(600)).await,
        ClientEventSendOutcome::Sent
    );
    assert_eq!(resources.snapshot().queued_bytes, 602);
    assert_eq!(
        sender.send_periodic_state("b".repeat(100)).await,
        ClientEventSendOutcome::Coalesced
    );
    assert_eq!(resources.snapshot().queued_bytes, 102);
    assert_eq!(
        sender.send_reliable_line("receipt".repeat(100)).await,
        ClientEventSendOutcome::Sent
    );
    assert_eq!(
        sender.send_periodic_state("c".repeat(300)).await,
        ClientEventSendOutcome::DroppedPeriodic
    );
    assert_eq!(resources.snapshot().queued_bytes, 804);
    let item = receiver.protocol_lines.recv().await.unwrap();
    let held_write = receiver.resolve_event(item).await;
    assert_eq!(
        resources.snapshot().queued_bytes,
        804,
        "dequeue is not completion of a write"
    );
    drop(receiver);
    assert_eq!(
        resources.snapshot().queued_bytes,
        102,
        "sender's tail must not retain discarded bytes"
    );
    drop(held_write);
    assert_eq!(resources.snapshot().queued_bytes, 0);
    assert_eq!(
        sender.send_reliable_line("after close".into()).await,
        ClientEventSendOutcome::Closed
    );
    assert_eq!(resources.snapshot().queued_bytes, 0);
    assert_eq!(metrics.snapshot().queue_depth, 0);
}

#[tokio::test]
async fn rejected_periodic_growth_preserves_prior_state_and_reliable_order() {
    let resources = resources();
    let (sender, mut receiver) = client_event_queue_with_budget(
        ServerOutboundBackpressureMetrics::default(),
        resources.peer_budget(),
    );
    assert_eq!(
        sender.send_periodic_state("old".repeat(200)).await,
        ClientEventSendOutcome::Sent
    );
    assert_eq!(
        sender.send_periodic_state("large".repeat(210)).await,
        ClientEventSendOutcome::DroppedPeriodic
    );
    assert_eq!(resources.snapshot().queued_bytes, 602);
    assert_eq!(
        sender.send_reliable_line("authoritative".into()).await,
        ClientEventSendOutcome::Sent
    );
    assert_eq!(
        receiver.receive_protocol_line().await.unwrap(),
        "old".repeat(200)
    );
    assert_eq!(
        receiver.receive_protocol_line().await.unwrap(),
        "authoritative"
    );
    assert_eq!(resources.snapshot().queued_bytes, 0);
}

#[tokio::test]
async fn global_byte_overload_isolated_to_sender_and_releases_capacity() {
    let resources = resources();
    let (first, first_rx) = client_event_queue_with_budget(
        ServerOutboundBackpressureMetrics::default(),
        resources.peer_budget(),
    );
    let (second, mut second_rx) = client_event_queue_with_budget(
        ServerOutboundBackpressureMetrics::default(),
        resources.peer_budget(),
    );
    assert_eq!(
        first.send_reliable_line("a".repeat(1000)).await,
        ClientEventSendOutcome::Sent
    );
    assert_eq!(
        second.send_reliable_line("b".repeat(600)).await,
        ClientEventSendOutcome::Overloaded
    );
    assert!(second_rx.overload_queue_depth_for_test().is_some());
    assert!(first_rx.overload_queue_depth_for_test().is_none());
    assert_eq!(resources.snapshot().queued_bytes, 1002);
    second_rx.close_and_record_discarded().await;
    drop(first_rx);
    assert_eq!(resources.snapshot().queued_bytes, 0);
    let (healthy, mut healthy_rx) = client_event_queue_with_budget(
        ServerOutboundBackpressureMetrics::default(),
        resources.peer_budget(),
    );
    assert_eq!(
        healthy.send_reliable_line("normal".into()).await,
        ClientEventSendOutcome::Sent
    );
    assert_eq!(healthy_rx.receive_protocol_line().await.unwrap(), "normal");
}
