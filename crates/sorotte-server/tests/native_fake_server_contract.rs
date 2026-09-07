// This consumer talks to the actual server executable. It intentionally imports
// only the independent fake's wire projection, never its runtime state machine.
#[allow(dead_code)]
mod support;

#[path = "../../sorotte-gui/src/bin/sorotte-gui-native-smoke/native_smoke_runner/fake_server_protocol.rs"]
mod fake_server_protocol;

use std::{
    io::{BufRead, BufReader},
    net::{TcpListener, TcpStream},
    time::Duration,
};

use serde_json::{Value, json};
use support::{ServerProcess, reserve_ipv4_port};

#[test]
fn native_fake_counter_contract_matches_recorded_real_server_conversation() {
    let conversation: Value = serde_json::from_str(include_str!(
        "../../../fixtures/native-harness/fake-server-conversation.json"
    ))
    .unwrap();
    let port = reserve_ipv4_port();
    let mut server = ServerProcess::spawn(&[
        "--port".to_owned(),
        port.to_string(),
        "--ipv4-only".to_owned(),
        "--interface-ipv4".to_owned(),
        "127.0.0.1".to_owned(),
    ]);
    let mut peer = server.wait_for_ipv4(port);
    let username = conversation["username"].as_str().unwrap();
    peer.hello(username, conversation["room"].as_str().unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let fake_writer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut fake_stream, _) = listener.accept().unwrap();
    fake_writer
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut fake_reader = BufReader::new(fake_writer);

    for request in conversation["exchanges"].as_array().unwrap() {
        let counter = fake_server_protocol::validated_client_ignore_counter(request).unwrap();
        fake_server_protocol::write_playlist_echo_counter_ack(&mut fake_stream, counter).unwrap();
        let mut line = String::new();
        fake_reader.read_line(&mut line).unwrap();
        let fake: Value = serde_json::from_str(&line).unwrap();
        peer.write_json_line(&request.to_string());
        if request.pointer("/State/playstate/doSeek") != Some(&json!(true)) {
            // The real server piggybacks acknowledgements on its next state
            // publication. The small fake may acknowledge immediately, but it
            // must preserve the same exact counter. Arm a forced publication
            // instead of mistaking a duration-only wait for a protocol boundary.
            let paused = request
                .pointer("/State/playstate/paused")
                .cloned()
                .unwrap_or(json!(true));
            peer.write_json_line(
                &json!({"State":{"playstate":{"position":0.0,"paused":paused,"doSeek":true}}})
                    .to_string(),
            );
        }
        let observed = peer.read_until(|message| {
            serde_json::to_value(message)
                .unwrap()
                .pointer("/State/ignoringOnTheFly/client")
                == Some(&json!(counter.unwrap()))
        });
        let actual = serde_json::to_value(observed).unwrap();
        if let Some(server_counter) = actual.pointer("/State/ignoringOnTheFly/server") {
            peer.write_json_line(
                &json!({"State":{"ignoringOnTheFly":{"server":server_counter}}}).to_string(),
            );
        }
        assert_eq!(
            actual.pointer("/State/ignoringOnTheFly/client"),
            fake.pointer("/State/ignoringOnTheFly/client"),
            "actual server must acknowledge the exact client counter, including seek and counter-only frames"
        );
        if let Some((paused, _)) =
            fake_server_protocol::validated_client_playstate_transition(request).unwrap()
        {
            assert_eq!(
                actual.pointer("/State/playstate/paused"),
                Some(&json!(paused))
            );
            // setBy is display attribution. It does not replace the frame's
            // transport acknowledgement or independently establish authority.
            assert_eq!(
                actual.pointer("/State/playstate/setBy"),
                Some(&json!(username))
            );
        }
    }
}
