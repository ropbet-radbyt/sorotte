use super::*;

impl GuiWidgetEguiRenderer {
    pub(in crate::app) fn action_for_checkbox_node(
        state: &SorotteGuiShellAppState,
        node: &GuiWidgetNode,
        value: bool,
    ) -> Option<GuiShellAction> {
        if node.id == "main-window:control:autoplay-toggle" {
            return Some(GuiShellAction::AnnounceAutoplayState(value));
        }
        if node.id == "main-window:browser:hide-empty" {
            return Some(GuiShellAction::ToggleMainWindowHideEmptyRooms);
        }
        if node.id == "plugins:media-matching:setting:fingerprinting" {
            return Some(GuiShellAction::SetMediaMatchFingerprintingEnabled(value));
        }
        if node.id == "plugins:media-matching:setting:runtime-tolerance" {
            return Some(GuiShellAction::SetMediaMatchRuntimeToleranceEnabled(value));
        }
        let (section, label, kind) = Self::configuration_control_identity(state, node)?;
        if kind != GuiDialogControlKind::Checkbox {
            return None;
        }
        Some(GuiShellAction::EditConfigurationBool {
            section,
            label,
            value,
        })
    }

    pub(in crate::app) fn actions_for_text_input_node(
        state: &SorotteGuiShellAppState,
        node: &GuiWidgetNode,
        value: &str,
        changed: bool,
        submitted: bool,
    ) -> Option<Vec<GuiShellAction>> {
        if node.id == "main-window:chat-input" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::ApplyGuiDraftRuntimeSnapshot(
                    GuiDraftRuntimeSnapshot {
                        outgoing_chat_message: (!value.is_empty()).then(|| value.to_owned()),
                    },
                ));
            }
            if submitted {
                actions.push(GuiShellAction::BeginLocalChatSend(value.to_owned()));
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:room-input" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::EditConfigurationText {
                    section: "Connection",
                    label: "Room",
                    value: value.to_owned(),
                });
            }
            if submitted && nonempty_room_name_text(value).is_some() {
                actions.push(GuiShellAction::JoinMainWindowRoom(value.to_owned()));
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:user:new" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateNewMainWindowUserDraft(
                    value.to_owned(),
                ));
            }
            if submitted && normalized_editable_text(value).is_some() {
                actions.push(GuiShellAction::CommitNewMainWindowUser);
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "room-history:edit:entries" {
            return changed.then(|| vec![GuiShellAction::UpdateRoomHistoryEdit(value.to_owned())]);
        }

        if node.id == "main-window:playlist-edit:text" {
            return changed.then(|| {
                vec![GuiShellAction::UpdateSharedPlaylistTextEdit(
                    value.to_owned(),
                )]
            });
        }

        if node.id == "main-window:playlist-url-edit:text" {
            return changed.then(|| {
                vec![GuiShellAction::UpdateSharedPlaylistUrlEdit(
                    value.to_owned(),
                )]
            });
        }

        if node.id == "main-window:media-url-edit:text" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateMediaUrlEdit(value.to_owned()));
            }
            if submitted && let Some(target) = normalized_editable_text(value) {
                actions.push(GuiShellAction::RequestMainWindowUserMediaOpen(target));
                actions.push(GuiShellAction::CancelMediaUrlEdit);
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:controlled-room-create:room" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateCreateControlledRoomEdit(
                    value.to_owned(),
                ));
            }
            if submitted {
                let room_name = controlled_room_base_name_legacy_compatible(value);
                if let Some(room_name) = nonempty_room_name_text(&room_name) {
                    actions.push(GuiShellAction::RequestControllerAuth {
                        room: room_name,
                        password: generate_room_password_legacy_compatible(),
                    });
                    actions.push(GuiShellAction::CancelCreateControlledRoomEdit);
                }
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if node.id == "main-window:controller-auth:password" {
            let mut actions = Vec::new();
            if changed {
                actions.push(GuiShellAction::UpdateControllerAuthPasswordEdit(
                    value.to_owned(),
                ));
            }
            if submitted
                && let Some(session) = state.controller_auth_edit_session.as_ref()
                && normalized_editable_text(value).is_some()
            {
                actions.push(GuiShellAction::RequestControllerAuth {
                    room: session.room_name.clone(),
                    password: value.to_owned(),
                });
                actions.push(GuiShellAction::CancelControllerAuthEdit);
            }
            return (!actions.is_empty()).then_some(actions);
        }

        if let Some((section, label, kind)) = Self::configuration_control_identity(state, node) {
            if matches!(
                kind,
                GuiDialogControlKind::TextInput
                    | GuiDialogControlKind::TextArea
                    | GuiDialogControlKind::PasswordInput
                    | GuiDialogControlKind::NumericInput
                    | GuiDialogControlKind::Select
            ) && changed
            {
                return Some(vec![GuiShellAction::EditConfigurationText {
                    section,
                    label,
                    value: value.to_owned(),
                }]);
            }
            return None;
        }

        let mut actions = Vec::new();
        match node.id.as_str() {
            "public-servers:edit:label" => {
                if changed {
                    actions.push(GuiShellAction::UpdatePublicServerEditLabel(
                        value.to_owned(),
                    ));
                }
                if submitted {
                    actions.push(GuiShellAction::CommitPublicServerEdit);
                }
            }
            "public-servers:edit:address" => {
                if changed {
                    actions.push(GuiShellAction::UpdatePublicServerEditAddress(
                        value.to_owned(),
                    ));
                }
                if submitted {
                    actions.push(GuiShellAction::CommitPublicServerEdit);
                }
            }
            "main-window:user-edit:username" => {
                if changed {
                    actions.push(GuiShellAction::UpdateMainWindowUserEdit(value.to_owned()));
                }
                if submitted {
                    actions.push(GuiShellAction::CommitMainWindowUserEdit);
                }
            }
            _ => {}
        }
        (!actions.is_empty()).then_some(actions)
    }

    pub(in crate::app) fn configuration_select_options_for_node(
        state: &SorotteGuiShellAppState,
        node: &GuiWidgetNode,
    ) -> Option<Vec<String>> {
        let (section, label, kind) = Self::configuration_control_identity(state, node)?;
        if kind != GuiDialogControlKind::Select {
            return None;
        }
        Some(match (section, label) {
            ("Readiness", "Unpause Action") => [
                "IfAlreadyReady",
                "IfOthersReady",
                "IfMinUsersReady",
                "Always",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            ("Readiness", "Autoplay Min Users") => {
                let mut options = ["app-default", "0", "1", "2", "3", "4", "5"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if let Some(value) = node.value.as_ref()
                    && !value.is_empty()
                    && !options.iter().any(|option| option == value)
                {
                    options.push(value.clone());
                }
                options
            }
            ("Privacy", "Filename Privacy") | ("Privacy", "Filesize Privacy") => {
                ["SendRaw", "SendHashed", "DoNotSend"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            }
            ("Chat", "Input Position") => ["Top", "Middle", "Bottom"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            ("Chat", "Output Mode") => ["Chatroom", "Scrolling"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            ("System", "Language") => SUPPORTED_LEGACY_RUNTIME_LANGUAGE_TAGS_DISPLAY
                .split('/')
                .map(str::to_owned)
                .collect(),
            ("System", "Update Channel") => {
                ["stable", "dev"].into_iter().map(str::to_owned).collect()
            }
            _ => return None,
        })
    }
}
