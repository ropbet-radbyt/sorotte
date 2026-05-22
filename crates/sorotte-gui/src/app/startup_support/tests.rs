use crate::app::testing::support::TEST_USERNAME;

#[test]
fn gui_client_core_chat_tcp_bootstrap_from_lookup_uses_existing_client_env_keys() {
    let bootstrap = super::gui_client_core_chat_tcp_bootstrap_from_lookup(|name| match name {
        "SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("true".to_owned()),
        "SOROTTE_CLIENT_HOST" => Some("syncplay.example".to_owned()),
        "SOROTTE_CLIENT_PORT" => Some("8995".to_owned()),
        "SOROTTE_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
        "SOROTTE_CLIENT_ROOM" => Some("room-a".to_owned()),
        _ => None,
    })
    .expect("bootstrap lookup should succeed")
    .expect("bootstrap should be enabled");

    assert_eq!(
        bootstrap,
        super::GuiClientCoreChatTcpBootstrap {
            host: "syncplay.example".to_owned(),
            port: 8995,
            username: TEST_USERNAME.to_owned(),
            room: "room-a".to_owned(),
        }
    );
    assert_eq!(bootstrap.host_arg(), "syncplay.example:8995");
}

#[test]
fn gui_client_core_chat_loopback_bootstrap_from_lookup_uses_existing_client_env_keys() {
    let bootstrap = super::gui_client_core_chat_loopback_bootstrap_from_lookup(|name| match name {
        "SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK" => Some("true".to_owned()),
        "SOROTTE_CLIENT_USERNAME" => Some(TEST_USERNAME.to_owned()),
        "SOROTTE_CLIENT_ROOM" => Some("room-a".to_owned()),
        _ => None,
    })
    .expect("bootstrap lookup should succeed")
    .expect("bootstrap should be enabled");

    assert_eq!(
        bootstrap,
        super::GuiClientCoreChatLoopbackBootstrap {
            username: TEST_USERNAME.to_owned(),
            room: "room-a".to_owned(),
        }
    );
}

#[test]
fn gui_client_core_chat_tcp_bootstrap_from_lookup_rejects_invalid_port() {
    let error = super::gui_client_core_chat_tcp_bootstrap_from_lookup(|name| match name {
        "SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" => Some("on".to_owned()),
        "SOROTTE_CLIENT_PORT" => Some("70000".to_owned()),
        _ => None,
    })
    .expect_err("invalid port should be rejected");

    assert_eq!(
        error,
        "SOROTTE_CLIENT_PORT must be a valid TCP port from 1 to 65535.".to_owned()
    );
}
