use super::*;

#[test]
fn gui_persisted_config_runtime_owner_projects_live_python_peer_chat_interop() {
    let result = match live_python_interop::run_live_python_peer_connect_flow() {
        Ok(result) => result,
        Err(error) if live_python_interop::live_python_interop_prerequisites_missing(&error) => {
            eprintln!(
                "live Python GUI interop chat test skipped due to missing local prerequisites"
            );
            return;
        }
        Err(error) => {
            panic!("live Python GUI interop chat flow should succeed, got: {error}")
        }
    };

    assert_eq!(
        result.room_name,
        live_python_interop::LIVE_PYTHON_INTEROP_ROOM
    );
    assert!(result.local_user_present);
    assert!(result.peer_user_present);
    assert!(result.room_switch_observed);
    assert!(result.room_rejoin_observed);
    assert!(result.peer_disconnect_observed);
    assert!(result.peer_reconnect_observed);
    assert_eq!(
        result.gui_playlist,
        vec![
            live_python_interop::LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_ONE.to_owned(),
            live_python_interop::LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_TWO.to_owned(),
        ]
    );
    assert_eq!(result.gui_playlist_index, Some(1));
    assert_eq!(
        result.peer_playlist,
        vec![
            live_python_interop::LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_ONE.to_owned(),
            live_python_interop::LIVE_PYTHON_INTEROP_PEER_PLAYLIST_ENTRY_TWO.to_owned(),
        ]
    );
    assert_eq!(result.peer_playlist_index, Some(1));
    assert!(result.gui_chat_messages.iter().any(|message| {
        message.sender == live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_USERNAME
            && message.message == live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE
    }));
    assert!(result.gui_chat_messages.iter().any(|message| {
        message.sender == live_python_interop::LIVE_PYTHON_INTEROP_PEER_USERNAME
            && message.message == live_python_interop::LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE
    }));
    assert!(result.gui_chat_messages.iter().any(|message| {
        message.sender == live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_USERNAME
            && message.message
                == live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_RECONNECT_CHAT_MESSAGE
    }));
    assert!(result.gui_chat_messages.iter().any(|message| {
        message.sender == live_python_interop::LIVE_PYTHON_INTEROP_PEER_USERNAME
            && message.message
                == live_python_interop::LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE
    }));
    assert!(result.peer_chat_messages.iter().any(|message| {
        message.sender == live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_USERNAME
            && message.message == live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE
    }));
    assert!(result.peer_chat_messages.iter().any(|message| {
        message.sender == live_python_interop::LIVE_PYTHON_INTEROP_PEER_USERNAME
            && message.message == live_python_interop::LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE
    }));
    assert!(result.peer_chat_messages.iter().any(|message| {
        message.sender == live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_USERNAME
            && message.message
                == live_python_interop::LIVE_PYTHON_INTEROP_LOCAL_RECONNECT_CHAT_MESSAGE
    }));
    assert!(result.peer_chat_messages.iter().any(|message| {
        message.sender == live_python_interop::LIVE_PYTHON_INTEROP_PEER_USERNAME
            && message.message
                == live_python_interop::LIVE_PYTHON_INTEROP_PEER_RECONNECT_CHAT_MESSAGE
    }));
    assert!(result.widget_count > 0);
}

#[test]
fn gui_persisted_config_runtime_owner_projects_live_python_peer_detached_connect_interop() {
    let result = match live_python_interop::run_live_python_peer_detached_public_server_connect_flow(
    ) {
        Ok(result) => result,
        Err(error) if live_python_interop::live_python_interop_prerequisites_missing(&error) => {
            eprintln!(
                "live Python GUI detached-connect test skipped due to missing local prerequisites"
            );
            return;
        }
        Err(error) => {
            panic!(
                "live Python GUI detached public-server connect flow should succeed, got: {error}"
            )
        }
    };

    assert_eq!(
        result.room_name,
        live_python_interop::LIVE_PYTHON_INTEROP_ROOM
    );
    assert!(result.local_user_present);
    assert!(result.peer_user_present);
    assert!(!result.local_user_ready);
    assert!(!result.peer_user_ready);
    assert!(result.widget_count > 0);
}

#[test]
fn gui_persisted_config_runtime_owner_projects_live_python_peer_controlled_room_interop() {
    let result = match live_python_interop::run_live_python_peer_controlled_room_flow() {
        Ok(result) => result,
        Err(error) if live_python_interop::live_python_interop_prerequisites_missing(&error) => {
            eprintln!(
                "live Python GUI controlled-room test skipped due to missing local prerequisites"
            );
            return;
        }
        Err(error) => {
            panic!("live Python GUI controlled-room flow should succeed, got: {error}")
        }
    };

    assert_eq!(
        result.room_name,
        live_python_interop::LIVE_PYTHON_INTEROP_CONTROLLED_ROOM
    );
    assert!(result.local_user_present);
    assert!(result.peer_user_present);
    assert!(result.local_user_controller);
    assert!(!result.peer_user_controller);
    assert!(!result.peer_local_controller);
    assert!(result.can_manage_playlist);
    assert!(result.widget_count > 0);
}
