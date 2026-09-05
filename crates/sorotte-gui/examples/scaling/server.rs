use super::{Fixture, metrics};
use serde_json::{Value, json};
use sorotte_server::{
    DirectedOutboundLine, ServerActorHandle, ServerResourceLimits, ServerRuntime,
    run_server_network_loop_until_shutdown,
};
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::watch,
};

fn hello(id: &str) -> String {
    json!({"Hello":{"username":id,"room":{"name":"scaling"},"version":"1.7.5",
        "features":{"sorotteLargeProtocolFramesV1":true,"mediaMatch":true,"sharedPlaylists":true}}})
    .to_string()
}

fn dispatch_summary(output: &[DirectedOutboundLine]) -> Result<Value, String> {
    let mut bytes = 0usize;
    let mut recipients = BTreeSet::new();
    for line in output {
        if line.line.len() > sorotte_protocol::SOROTTE_MAX_PROTOCOL_LINE_BYTES {
            return Err("server emitted an oversized frame".to_owned());
        }
        bytes += line.line.len();
        recipients.insert(&line.client_id);
    }
    Ok(
        json!({"encoded_bytes":bytes,"frames":output.len(),"recipients":recipients.len(),
        "largest_frame_bytes":output.iter().map(|line| line.line.len()).max().unwrap_or(0)}),
    )
}

pub fn model(fixture: Fixture, extra_clone: bool) -> Result<Value, String> {
    let mut runtime = ServerRuntime::new();
    runtime
        .try_set_permanent_rooms((0..fixture.empty_rooms).map(|i| format!("empty-{i:04}")))
        .map_err(|e| format!("empty-room fixture: {e}"))?;
    let file = json!({"Set":{"file":{"name":"fixture.mkv","duration":120.0,"size":123456,
        "benchmarkMetadata":"x".repeat(fixture.metadata_bytes)}}})
    .to_string();
    for i in 0..fixture.roster {
        let id = format!("user-{i:04}");
        runtime
            .handle_line_fanout(&id, &hello(&id))
            .map_err(|e| format!("member {i} join: {e}"))?;
        runtime
            .handle_line_fanout(&id, &file)
            .map_err(|e| format!("member {i} metadata: {e}"))?;
    }
    let (list, list_cost) = metrics::measure(|| {
        let output = runtime
            .handle_line_fanout("user-0000", r#"{"List":null}"#)
            .map_err(|e| e.to_string())?;
        if extra_clone {
            std::hint::black_box(output.clone());
        }
        Ok(output)
    })?;
    let roster: Value = serde_json::from_str(&list.first().ok_or("List response missing")?.line)
        .map_err(|e| e.to_string())?;
    let actual_roster = roster["List"]["scaling"]
        .as_object()
        .ok_or("roster missing")?
        .len();
    if actual_roster != fixture.roster {
        return Err("server roster lost fixture members".to_owned());
    }
    if roster["List"]
        .as_object()
        .ok_or("room inventory missing")?
        .len()
        != fixture.empty_rooms + 1
    {
        return Err("server roster lost fixture empty rooms".to_owned());
    }
    let (update, update_cost) = metrics::measure(|| {
        runtime
            .handle_line_fanout("user-0000", &file)
            .map_err(|e| e.to_string())
    })?;
    let expected_files = (0..fixture.server_playlist_items)
        .map(|i| format!("episode-{i:04}-{}.mkv", "p".repeat(16)))
        .collect::<Vec<_>>();
    let playlist = json!({"Set":{"playlistChange":{"files":expected_files}}}).to_string();
    let (playlist_output, playlist_cost) = metrics::measure(|| {
        runtime
            .handle_line_fanout("user-0000", &playlist)
            .map_err(|e| format!("playlist fixture: {e}"))
    })?;
    let mut accepted_playlist_recipients = BTreeSet::new();
    for line in &playlist_output {
        let value: Value = serde_json::from_str(&line.line).map_err(|e| e.to_string())?;
        if let Some(files) = value["Set"]["playlistChange"]["files"].as_array() {
            if !files
                .iter()
                .map(Value::as_str)
                .eq(expected_files.iter().map(|file| Some(file.as_str())))
            {
                return Err(
                    "server corrected the generated playlist instead of accepting it".to_owned(),
                );
            }
            accepted_playlist_recipients.insert(&line.client_id);
        }
    }
    if accepted_playlist_recipients.len() != fixture.roster {
        return Err("generated playlist did not reach every roster member".to_owned());
    }
    let ((), joins_leaves_cost) = metrics::measure(|| {
        for i in 0..fixture.churn_cycles {
            let id = format!("churn-{i:06}");
            runtime
                .handle_line_fanout(&id, &hello(&id))
                .map_err(|e| e.to_string())?;
            runtime
                .handle_transport_disconnect_fanout(&id)
                .map_err(|e| e.to_string())?;
            if runtime.session(&id).is_some() {
                return Err("disconnected model session retained".to_owned());
            }
        }
        Ok(())
    })?;
    Ok(
        json!({"list":{"allocation":list_cost,"dispatch":dispatch_summary(&list)?},
        "metadata":{"allocation":update_cost,"dispatch":dispatch_summary(&update)?},
        "playlist":{"allocation":playlist_cost,"dispatch":dispatch_summary(&playlist_output)?,"accepted_items":expected_files.len(),"accepted_recipients":accepted_playlist_recipients.len()},
        "joins_leaves":joins_leaves_cost,"retained_members":actual_roster,"empty_rooms":fixture.empty_rooms}),
    )
}

async fn send(stream: &mut (impl AsyncWriteExt + Unpin), line: &str) -> Result<(), String> {
    stream
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    stream.write_all(b"\n").await.map_err(|e| e.to_string())
}

async fn read_until(
    reader: &mut (impl AsyncBufReadExt + Unpin),
    needle: &str,
) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let mut line = String::new();
            if reader
                .read_line(&mut line)
                .await
                .map_err(|e| e.to_string())?
                == 0
            {
                return Err("healthy peer disconnected".to_owned());
            }
            if line.len() > sorotte_protocol::SOROTTE_MAX_PROTOCOL_LINE_BYTES + 2 {
                return Err("oversized network frame".to_owned());
            }
            if line.contains(needle) {
                return Ok(());
            }
        }
    })
    .await
    .map_err(|_| "healthy peer progress deadline exceeded".to_owned())?
}

async fn wait_connections(actor: &ServerActorHandle, expected: usize) -> Result<(), String> {
    tokio::time::timeout(Duration::from_secs(10), async {
        while actor.resource_snapshot().active_connections != expected
            || actor.resource_snapshot().queued_bytes != 0
        {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .map_err(|_| "network cleanup did not reach its bounded baseline".to_owned())
}

fn resource(actor: &ServerActorHandle) -> Value {
    let state = actor.resource_snapshot();
    let queue = actor.outbound_backpressure_snapshot();
    json!({"active_connections":state.active_connections,"unauthenticated_connections":state.unauthenticated_connections,
        "address_buckets":state.address_buckets,"queued_bytes":state.queued_bytes,"peak_queued_bytes":state.peak_queued_bytes,
        "queue_depth":queue.queue_depth,"peak_queue_depth":queue.peak_queue_depth,
        "overload_disconnects":queue.overload_disconnects,"dropped_messages":queue.dropped_messages,
        "coalesced_state_updates":queue.coalesced_state_updates})
}

pub async fn network(fixture: Fixture) -> Result<Value, String> {
    let mut runtime = ServerRuntime::new();
    let mut limits = ServerResourceLimits::default();
    limits.active_connections = 16;
    limits.unauthenticated_connections = 8;
    limits.connections_per_address = 16;
    limits.queued_bytes_per_peer = 512 * 1024;
    limits.queued_bytes_total = 4 * 1024 * 1024;
    runtime
        .set_resource_limits(limits)
        .map_err(|e| e.to_string())?;
    let actor = ServerActorHandle::spawn(runtime);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(run_server_network_loop_until_shutdown(
        listener,
        actor.clone(),
        None,
        shutdown_rx,
    ));
    let mut healthy = TcpStream::connect(address)
        .await
        .map_err(|e| e.to_string())?;
    send(&mut healthy, &hello("healthy")).await?;
    let (read, mut write) = healthy.into_split();
    let mut reader = BufReader::new(read);
    read_until(&mut reader, "\"Hello\"").await?;
    let mut slow = TcpStream::connect(address)
        .await
        .map_err(|e| e.to_string())?;
    send(&mut slow, &hello("unreading")).await?;
    // Never drain this peer: it exercises the OS socket and production outbound queue together.
    let bytes = if fixture.name == "large" {
        128 * 1024
    } else {
        16 * 1024
    };
    let updates = if fixture.name == "large" { 256 } else { 64 };
    let mut healthy_latency = Vec::with_capacity(updates);
    for sequence in 0..updates {
        let line = json!({"Set":{"file":{"name":"slow-peer.mkv","benchmarkSequence":sequence,
            "benchmarkMetadata":"s".repeat(bytes)}}})
        .to_string();
        let started = Instant::now();
        send(&mut write, &line).await?;
        read_until(&mut reader, &format!("\"benchmarkSequence\":{sequence}")).await?;
        healthy_latency.push(started.elapsed().as_nanos() as u64);
    }
    let slow_snapshot = resource(&actor);
    drop(slow);
    drop(write);
    drop(reader);
    wait_connections(&actor, 0).await?;
    let handles_before = metrics::os_handles()?;
    let mut checkpoints = Vec::new();
    let mut churn_latency = Vec::with_capacity(fixture.churn_cycles);
    for cycle in 0..fixture.churn_cycles {
        let started = Instant::now();
        let mut stream = TcpStream::connect(address)
            .await
            .map_err(|e| e.to_string())?;
        send(&mut stream, &hello(&format!("reconnect-{cycle:06}"))).await?;
        let mut peer = BufReader::new(stream);
        read_until(&mut peer, "\"Hello\"").await?;
        drop(peer);
        wait_connections(&actor, 0).await?;
        churn_latency.push(started.elapsed().as_nanos() as u64);
        if (cycle + 1) % (fixture.churn_cycles / 8).max(1) == 0 || cycle + 1 == fixture.churn_cycles
        {
            checkpoints.push(json!({"completed_cycles":cycle+1,"resources":resource(&actor),"os_handles":metrics::os_handles()?}));
        }
    }
    let retained = actor.resource_snapshot();
    if retained.active_connections != 0
        || retained.unauthenticated_connections != 0
        || retained.address_buckets != 0
        || retained.queued_bytes != 0
        || retained.peak_queued_bytes > limits.queued_bytes_total
    {
        return Err("network retained-resource invariant failed".to_owned());
    }
    let handles_after = metrics::os_handles()?;
    shutdown_tx.send(true).map_err(|e| e.to_string())?;
    task.await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    actor.shutdown().await.map_err(|e| e.to_string())?;
    Ok(
        json!({"slow_peer":{"updates":updates,"metadata_bytes":bytes,"resources":slow_snapshot,
        "healthy_round_trip_nanoseconds":healthy_latency},"churn_round_trip_nanoseconds":churn_latency,
        "checkpoints":checkpoints,"handles_before_churn":handles_before,"handles_after_churn":handles_after,
        "joined_network_workers":true,"retained_network_workers":0,"retained_connections":0,
        "queue_byte_limit":limits.queued_bytes_total}),
    )
}
