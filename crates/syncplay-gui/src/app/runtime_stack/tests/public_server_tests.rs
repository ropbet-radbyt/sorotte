use super::*;

#[test]
fn gui_client_core_chat_session_runtime_adapter_normalizes_public_server_refresh_rows() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1")
        .expect("client-core chat adapter should bootstrap");

    let refreshed = GuiSessionRuntimeAdapter::refresh_public_servers(
        &mut adapter,
        vec![
            (" Primary ".to_owned(), " syncplay.pl:8999 ".to_owned()),
            ("Duplicate".to_owned(), "SYNCPLAY.PL:8999".to_owned()),
            (" ".to_owned(), "backup.example:9000".to_owned()),
            ("Invalid".to_owned(), " :9000 ".to_owned()),
            ("IPv6".to_owned(), "[::1]:8999".to_owned()),
        ],
        Some("fr"),
    )
    .expect("public-server refresh should normalize rows");

    assert_eq!(
        refreshed,
        vec![
            ("Primary".to_owned(), "syncplay.pl:8999".to_owned()),
            ("IPv6".to_owned(), "[::1]:8999".to_owned()),
        ]
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_uses_lookup_public_server_refresh_source() {
    let refreshed =
        GuiClientCoreChatSessionRuntimeAdapter::refreshed_public_server_rows_from_lookup(&|name| {
            match name {
                "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS" => Some(
                    r#"[[" Gui Primary ", " syncplay.pl:8999 "], ["Duplicate", "SYNCPLAY.PL:8999"]]"#
                        .to_owned(),
                ),
                _ => None,
            }
        })
        .expect("lookup-backed public-server refresh should parse")
        .expect("lookup-backed public-server refresh should produce rows");

    assert_eq!(
        refreshed,
        vec![("Gui Primary".to_owned(), "syncplay.pl:8999".to_owned())]
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_uses_file_lookup_public_server_refresh_source() {
    let refreshed = GuiClientCoreChatSessionRuntimeAdapter::refreshed_public_server_rows_from_sources(
        &|name| match name {
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH" => Some("public-servers.txt".to_owned()),
            "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS" => {
                Some(r#"[["Inline", "inline.example:9000"]]"#.to_owned())
            }
            _ => None,
        },
        &|path| {
            if path == "public-servers.txt" {
                Ok(
                    r#"[[" File Primary ", " file.example:8999 "], ["Duplicate", "FILE.EXAMPLE:8999"]]"#
                        .to_owned(),
                )
            } else {
                Err("unexpected path".to_owned())
            }
        },
    )
    .expect("file-backed public-server refresh should parse")
    .expect("file-backed public-server refresh should produce rows");

    assert_eq!(
        refreshed,
        vec![("File Primary".to_owned(), "file.example:8999".to_owned())]
    );
}

#[test]
fn gui_client_core_chat_session_runtime_adapter_rejects_invalid_lookup_public_server_refresh_source()
 {
    let error =
        GuiClientCoreChatSessionRuntimeAdapter::refreshed_public_server_rows_from_lookup(&|name| {
            (name == "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS")
                .then_some("not-a-serialized-public-server-list".to_owned())
        })
        .expect_err("invalid lookup-backed public-server refresh should fail");

    assert!(
        error.contains("SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS"),
        "error should identify the invalid lookup source"
    );
}
