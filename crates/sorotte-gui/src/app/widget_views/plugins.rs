use super::*;

impl SorotteGuiShellAppState {
    pub(crate) fn plugins_widget_tree(&self) -> GuiWidgetNode {
        let stream_support_selected = self.selected_plugin == GuiPluginSelection::StreamSupport;
        let media_matching_selected = self.selected_plugin == GuiPluginSelection::MediaMatching;
        let plex_selected = self.selected_plugin == GuiPluginSelection::Plex;
        let plugin_list = GuiWidgetNode::branch(
            "plugins:list",
            "Plugins",
            GuiWidgetKind::Panel,
            vec![
                GuiWidgetNode::leaf(
                    "plugins:list:stream-support",
                    "Stream Support",
                    GuiWidgetKind::ListItem,
                    Some(self.stream_helper.health.label().to_owned()),
                    true,
                    stream_support_selected,
                ),
                GuiWidgetNode::leaf(
                    "plugins:list:media-matching",
                    "Media Matching",
                    GuiWidgetKind::ListItem,
                    Some(self.media_match.health.label().to_owned()),
                    true,
                    media_matching_selected,
                ),
                GuiWidgetNode::leaf(
                    "plugins:list:plex",
                    "Plex",
                    GuiWidgetKind::ListItem,
                    Some(self.plex.status.clone()),
                    true,
                    plex_selected,
                ),
            ],
        );

        let selected_detail = match self.selected_plugin {
            GuiPluginSelection::StreamSupport => self.stream_support_plugin_detail_widget_tree(),
            GuiPluginSelection::MediaMatching => self.media_matching_plugin_detail_widget_tree(),
            GuiPluginSelection::Plex => self.plex_plugin_detail_widget_tree(),
        };
        let detail = GuiWidgetNode::branch(
            "plugins:details",
            "Plugin Details",
            GuiWidgetKind::Panel,
            vec![selected_detail.with_span(2)],
        );
        GuiWidgetNode::layout(
            "plugins-root",
            "Plugins",
            GuiLayoutMode::ResponsiveColumns {
                min_column_width: 260.0,
                max_columns: 3,
            },
            vec![plugin_list, detail.with_span(2)],
        )
    }

    fn plex_plugin_detail_widget_tree(&self) -> GuiWidgetNode {
        let mut children = vec![
            GuiWidgetNode::layout(
                "plugins:plex:status",
                "Plex Status",
                GuiLayoutMode::KeyValueGrid {
                    min_pair_width: 260.0,
                },
                self.plex_plugin_status_rows(),
            ),
            GuiWidgetNode::layout(
                "plugins:plex:actions",
                "Plex Actions",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 150.0,
                },
                vec![
                    self.plex_account_action_node(),
                    GuiWidgetNode::leaf(
                        "plugins:plex:refresh-servers",
                        "Refresh Servers",
                        GuiWidgetKind::Button,
                        None,
                        self.plex_plugin_action_enabled("refresh-servers"),
                        false,
                    ),
                    self.plex_sync_action_node(),
                ],
            ),
        ];
        if !self.plex.servers.is_empty() {
            children.push(GuiWidgetNode::layout(
                "plugins:plex:servers",
                "Plex Servers",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 190.0,
                },
                self.plex
                    .servers
                    .iter()
                    .enumerate()
                    .map(|(index, server)| {
                        GuiWidgetNode::leaf(
                            format!("plugins:plex:server:{index}"),
                            server.name.clone(),
                            GuiWidgetKind::Button,
                            Some(format!(
                                "{} · route: {} · {}",
                                Self::plex_server_scope_label(server),
                                server.connection_kind.label(),
                                server.uri
                            )),
                            true,
                            server.selected,
                        )
                        .with_tooltip(format!(
                            "{}\n{}\n{} route\n{}",
                            server.name,
                            Self::plex_server_scope_label(server),
                            server.connection_kind.label(),
                            server.uri
                        ))
                    })
                    .collect(),
            ));
        }

        GuiWidgetNode::branch("plugins:plex", "Plex", GuiWidgetKind::Panel, children)
    }

    fn media_matching_plugin_detail_widget_tree(&self) -> GuiWidgetNode {
        let mut children = vec![
            GuiWidgetNode::layout(
                "plugins:media-matching:status",
                "Media Matching Status",
                GuiLayoutMode::KeyValueGrid {
                    min_pair_width: 260.0,
                },
                self.media_matching_plugin_status_rows(),
            ),
            GuiWidgetNode::layout(
                "plugins:media-matching:settings",
                "Media Matching Settings",
                GuiLayoutMode::KeyValueGrid {
                    min_pair_width: 260.0,
                },
                self.media_matching_plugin_settings_rows(),
            ),
            GuiWidgetNode::layout(
                "plugins:media-matching:actions",
                "Media Matching Actions",
                GuiLayoutMode::ButtonWrap {
                    min_button_width: 150.0,
                },
                vec![
                    GuiWidgetNode::leaf(
                        "plugins:media-matching:install",
                        "Install Tools",
                        GuiWidgetKind::Button,
                        None,
                        self.media_matching_plugin_action_enabled("install"),
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "plugins:media-matching:import-ffmpeg",
                        "Import ffmpeg",
                        GuiWidgetKind::Button,
                        None,
                        self.media_matching_plugin_action_enabled("import"),
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "plugins:media-matching:import-ffprobe",
                        "Import ffprobe",
                        GuiWidgetKind::Button,
                        None,
                        self.media_matching_plugin_action_enabled("import"),
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "plugins:media-matching:import-fpcalc",
                        "Import fpcalc",
                        GuiWidgetKind::Button,
                        None,
                        self.media_matching_plugin_action_enabled("import"),
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "plugins:media-matching:open-location",
                        "Open Install Location",
                        GuiWidgetKind::Button,
                        None,
                        self.media_match.open_install_location_available,
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "plugins:media-matching:recheck",
                        "Recheck Tools",
                        GuiWidgetKind::Button,
                        None,
                        self.media_matching_plugin_action_enabled("recheck"),
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "plugins:media-matching:rebuild-index",
                        "Rebuild Index",
                        GuiWidgetKind::Button,
                        None,
                        self.media_matching_plugin_action_enabled("index"),
                        false,
                    ),
                    GuiWidgetNode::leaf(
                        "plugins:media-matching:clear-cache",
                        "Clear Cache",
                        GuiWidgetKind::Button,
                        None,
                        self.media_matching_plugin_action_enabled("cache"),
                        false,
                    ),
                ],
            ),
        ];
        if self.media_match_remediation.active {
            children.push(GuiWidgetNode::layout(
                "plugins:media-matching:remediation",
                "Media Matching Progress",
                GuiLayoutMode::KeyValueGrid {
                    min_pair_width: 260.0,
                },
                self.media_matching_plugin_remediation_rows(),
            ));
        }
        GuiWidgetNode::branch(
            "plugins:media-matching",
            "Media Matching",
            GuiWidgetKind::Panel,
            children,
        )
    }

    fn media_matching_plugin_status_rows(&self) -> Vec<GuiWidgetNode> {
        let mut rows = vec![
            GuiWidgetNode::leaf(
                "plugins:media-matching:title",
                "Title",
                GuiWidgetKind::Status,
                Some(self.media_match_status_title().to_owned()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:media-matching:summary",
                "Summary",
                GuiWidgetKind::Status,
                Some(self.media_match_status_summary()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:media-matching:health",
                "Health",
                GuiWidgetKind::Status,
                Some(self.media_match.health.label().to_owned()),
                true,
                false,
            ),
        ];
        for (id, label, value) in [
            (
                "plugins:media-matching:install-location",
                "Install Location",
                self.media_match.install_location.clone(),
            ),
            (
                "plugins:media-matching:ffmpeg-status",
                "ffmpeg",
                self.media_match.ffmpeg_status.clone(),
            ),
            (
                "plugins:media-matching:ffprobe-status",
                "ffprobe",
                self.media_match.ffprobe_status.clone(),
            ),
            (
                "plugins:media-matching:fpcalc-status",
                "fpcalc",
                self.media_match.fpcalc_status.clone(),
            ),
            (
                "plugins:media-matching:cache-status",
                "Cache",
                self.media_match.cache_status.clone(),
            ),
            (
                "plugins:media-matching:background-status",
                "Background",
                self.media_match.background_status.clone(),
            ),
            (
                "plugins:media-matching:current-decision",
                "Current File",
                self.media_match.current_decision.clone(),
            ),
            (
                "plugins:media-matching:last-evidence",
                "Last Evidence",
                self.media_match.last_evidence.clone(),
            ),
        ] {
            if let Some(value) = value {
                rows.push(GuiWidgetNode::leaf(
                    id,
                    label,
                    GuiWidgetKind::Status,
                    Some(value),
                    true,
                    false,
                ));
            }
        }
        rows
    }

    fn media_matching_plugin_settings_rows(&self) -> Vec<GuiWidgetNode> {
        let fingerprinting_enabled = self.media_match.settings.fingerprinting_enabled;
        let background_warmup_enabled = self.media_match.settings.background_warmup_enabled;
        let runtime_tolerance_enabled = self.media_match.settings.runtime_tolerance_enabled;
        let strong_policy = self.media_match.settings.autoplay_policy
            == sorotte_media_match::MediaMatchAutoplayPolicy::AllowStrongSameMedia;
        vec![
            GuiWidgetNode::leaf(
                "plugins:media-matching:setting:fingerprinting",
                "Fingerprinting",
                GuiWidgetKind::Checkbox,
                Some(bool_label(fingerprinting_enabled).to_owned()),
                self.pending_operation.is_none(),
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:media-matching:setting:background-warmup",
                "Background Warmup",
                GuiWidgetKind::Checkbox,
                Some(bool_label(background_warmup_enabled).to_owned()),
                self.pending_operation.is_none(),
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:media-matching:setting:runtime-tolerance",
                "Runtime Tolerance",
                GuiWidgetKind::Checkbox,
                Some(bool_label(runtime_tolerance_enabled).to_owned()),
                self.pending_operation.is_none(),
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:media-matching:policy:diagnostics",
                "Diagnostics Only",
                GuiWidgetKind::Button,
                Some(self.media_match_autoplay_policy_summary()),
                self.pending_operation.is_none(),
                !strong_policy,
            ),
            GuiWidgetNode::leaf(
                "plugins:media-matching:policy:strong",
                "Allow Strong Same-Media",
                GuiWidgetKind::Button,
                Some(self.media_match_autoplay_policy_summary()),
                self.pending_operation.is_none(),
                strong_policy,
            ),
        ]
    }

    fn media_matching_plugin_remediation_rows(&self) -> Vec<GuiWidgetNode> {
        let mut rows = vec![
            GuiWidgetNode::leaf(
                "plugins:media-matching:remediation:label",
                "Operation",
                GuiWidgetKind::Status,
                self.media_match_remediation.label.clone(),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:media-matching:remediation:progress",
                "Progress",
                GuiWidgetKind::Status,
                Some(format!(
                    "{:.0}%",
                    self.media_match_remediation.progress_fraction * 100.0
                )),
                true,
                false,
            ),
        ];
        if let Some(detail) = self.media_match_remediation.detail.clone() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:media-matching:remediation:detail",
                "Detail",
                GuiWidgetKind::Status,
                Some(detail),
                true,
                false,
            ));
        }
        rows
    }

    fn media_matching_plugin_action_enabled(&self, action: &str) -> bool {
        if self.pending_operation.is_some() || self.media_match_remediation.active {
            return false;
        }
        match action {
            "install" => self.media_match.install_supported,
            "import" => self.media_match.integration_supported,
            "recheck" | "cache" => true,
            "index" => {
                self.media_match.settings.fingerprinting_enabled
                    && self.media_match.health == GuiMediaMatchToolHealth::Healthy
            }
            _ => false,
        }
    }

    fn plex_plugin_status_rows(&self) -> Vec<GuiWidgetNode> {
        let mut rows = vec![
            GuiWidgetNode::leaf(
                "plugins:plex:title",
                "Title",
                GuiWidgetKind::Status,
                Some(self.plex_status_title()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:plex:summary",
                "Summary",
                GuiWidgetKind::Status,
                Some(self.plex_status_summary()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:plex:health",
                "Health",
                GuiWidgetKind::Status,
                Some(self.plex_status_health()),
                true,
                false,
            ),
        ];
        if self.plex.authenticated && self.plex.servers.is_empty() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:plex:status:servers",
                "Servers",
                GuiWidgetKind::Status,
                Some("none found".to_owned()),
                true,
                false,
            ));
        } else if self.plex.authenticated && self.selected_plex_server().is_none() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:plex:status:server",
                "Server",
                GuiWidgetKind::Status,
                Some("select a server".to_owned()),
                true,
                false,
            ));
        }
        if let Some(item) = self.plex.current_item.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:plex:status:item",
                "Current Item",
                GuiWidgetKind::Status,
                Some(item.clone()),
                true,
                false,
            ));
        }
        if let Some(last_report) = self.plex.last_report.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:plex:status:last-report",
                "Last Report",
                GuiWidgetKind::Status,
                Some(last_report.clone()),
                true,
                false,
            ));
        }
        if let Some(code) = self.plex.auth_code.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:plex:status:auth-code",
                "Auth Code",
                GuiWidgetKind::Status,
                Some(code.clone()),
                true,
                false,
            ));
        }
        if let Some(url) = self.plex.auth_url.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:plex:status:auth-url",
                "Auth URL",
                GuiWidgetKind::Status,
                Some(url.clone()),
                true,
                false,
            ));
        }
        if let Some(error) = self.plex.last_error.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:plex:status:error",
                "Last Error",
                GuiWidgetKind::Status,
                Some(error.clone()),
                true,
                false,
            ));
        }
        rows
    }

    fn plex_status_title(&self) -> String {
        if self.plex.authenticating {
            "Plex login pending".to_owned()
        } else if !self.plex.authenticated {
            "Connect Plex".to_owned()
        } else if self.selected_plex_server().is_none() {
            "Choose a Plex server".to_owned()
        } else if self.selected_plex_server_reachability()
            == Some(GuiPlexServerReachability::Checking)
        {
            "Checking Plex server".to_owned()
        } else if self.selected_plex_server_reachability()
            == Some(GuiPlexServerReachability::Unreachable)
        {
            "Plex server offline".to_owned()
        } else if self.plex.enabled {
            "Plex sync active".to_owned()
        } else {
            "Plex ready".to_owned()
        }
    }

    fn plex_status_summary(&self) -> String {
        if let Some(error) = self.plex.last_error.as_deref() {
            return error.to_owned();
        }
        if !self.plex.authenticated && !self.plex.authenticating {
            return "Connect your Plex account to report watch progress.".to_owned();
        }
        if self.plex.enabled {
            if let Some(item) = self.plex.current_item.as_ref() {
                return format!("Reporting watch progress for {item}.");
            }
            return self
                .selected_plex_server()
                .map(|server| {
                    format!(
                        "Reporting watch progress to {} ({} over a {} route).",
                        server.name,
                        Self::plex_server_scope_label(server),
                        server.connection_kind.label()
                    )
                })
                .unwrap_or_else(|| "Timeline reporting enabled.".to_owned());
        }
        if self.plex.authenticating {
            return "Finish the browser login; this panel will update automatically.".to_owned();
        }
        match self.selected_plex_server_reachability() {
            Some(GuiPlexServerReachability::Checking) => {
                return format!(
                    "Checking whether the selected {} Plex server is reachable over a {} route.",
                    self.selected_plex_server_scope_label().unwrap_or("owned"),
                    self.selected_plex_server_route_label().unwrap_or("remote")
                );
            }
            Some(GuiPlexServerReachability::Reachable) => {
                return format!(
                    "Selected {} Plex server is reachable over a {} route; sync is ready to enable.",
                    self.selected_plex_server_scope_label().unwrap_or("owned"),
                    self.selected_plex_server_route_label().unwrap_or("remote")
                );
            }
            Some(GuiPlexServerReachability::Unreachable) => {
                return format!(
                    "Selected {} Plex server could not be reached over a {} route. Refresh servers or choose another.",
                    self.selected_plex_server_scope_label().unwrap_or("owned"),
                    self.selected_plex_server_route_label().unwrap_or("remote")
                );
            }
            Some(GuiPlexServerReachability::Unknown) => {
                return format!(
                    "Selected {} Plex server has not been checked over its {} route yet.",
                    self.selected_plex_server_scope_label().unwrap_or("owned"),
                    self.selected_plex_server_route_label().unwrap_or("remote")
                );
            }
            None => {}
        }
        if self.plex.authenticated {
            return "Plex account connected; no server selected.".to_owned();
        }
        "Plex account not connected.".to_owned()
    }

    fn plex_status_health(&self) -> String {
        if self.plex.last_error.is_some() {
            "error".to_owned()
        } else if self.plex.enabled {
            "enabled".to_owned()
        } else if self.plex.authenticating {
            "authenticating".to_owned()
        } else if self.selected_plex_server_reachability()
            == Some(GuiPlexServerReachability::Checking)
        {
            "checking".to_owned()
        } else if self.selected_plex_server_reachability()
            == Some(GuiPlexServerReachability::Unreachable)
        {
            "offline".to_owned()
        } else if self.plex.authenticated {
            "ready".to_owned()
        } else {
            "disconnected".to_owned()
        }
    }

    fn plex_account_action_node(&self) -> GuiWidgetNode {
        if self.plex.authenticating {
            GuiWidgetNode::leaf(
                "plugins:plex:poll-auth",
                "Check Login",
                GuiWidgetKind::Button,
                None,
                self.plex_plugin_action_enabled("poll-auth"),
                false,
            )
        } else if self.plex.authenticated {
            GuiWidgetNode::leaf(
                "plugins:plex:disconnect",
                "Disconnect Plex",
                GuiWidgetKind::Button,
                None,
                self.plex_plugin_action_enabled("disconnect"),
                false,
            )
        } else {
            GuiWidgetNode::leaf(
                "plugins:plex:connect",
                "Connect Plex",
                GuiWidgetKind::Button,
                None,
                self.plex_plugin_action_enabled("connect"),
                false,
            )
        }
    }

    fn plex_sync_action_node(&self) -> GuiWidgetNode {
        GuiWidgetNode::leaf(
            if self.plex.enabled {
                "plugins:plex:disable-sync"
            } else {
                "plugins:plex:enable-sync"
            },
            if self.plex.enabled {
                "Turn Sync Off"
            } else {
                "Turn Sync On"
            },
            GuiWidgetKind::Button,
            None,
            self.plex_plugin_action_enabled("toggle-sync"),
            false,
        )
    }

    fn selected_plex_server(&self) -> Option<&GuiPlexServerRow> {
        self.plex.servers.iter().find(|server| server.selected)
    }

    fn selected_plex_server_reachability(&self) -> Option<GuiPlexServerReachability> {
        self.selected_plex_server()
            .map(|server| server.reachability)
    }

    fn selected_plex_server_route_label(&self) -> Option<&'static str> {
        self.selected_plex_server()
            .map(|server| server.connection_kind.label())
    }

    fn selected_plex_server_scope_label(&self) -> Option<&'static str> {
        self.selected_plex_server()
            .map(Self::plex_server_scope_label)
    }

    fn plex_server_scope_label(server: &GuiPlexServerRow) -> &'static str {
        if server.has_local_connection {
            "local server"
        } else if server.owned {
            "owned server"
        } else {
            "shared server"
        }
    }

    fn plex_plugin_action_enabled(&self, action: &str) -> bool {
        if self.pending_operation.is_some() {
            return false;
        }
        match action {
            "connect" => !self.plex.authenticated && !self.plex.authenticating,
            "poll-auth" => self.plex.authenticating,
            "refresh-servers" => self.plex.authenticated,
            "toggle-sync" => {
                if self.plex.enabled {
                    return true;
                }
                self.plex.authenticated
                    && self.selected_plex_server().is_some()
                    && !matches!(
                        self.selected_plex_server_reachability(),
                        Some(GuiPlexServerReachability::Checking)
                            | Some(GuiPlexServerReachability::Unreachable)
                    )
            }
            "disconnect" => {
                self.plex.authenticated || self.plex.authenticating || self.plex.enabled
            }
            _ => false,
        }
    }

    fn stream_support_plugin_detail_widget_tree(&self) -> GuiWidgetNode {
        let mut children = Vec::new();
        if let Some(alert) = self.stream_support_plugin_alert_widget_tree() {
            children.push(alert);
        }

        children.push(GuiWidgetNode::layout(
            "plugins:stream-support:status",
            "Stream Support Status",
            GuiLayoutMode::KeyValueGrid {
                min_pair_width: 260.0,
            },
            self.stream_support_plugin_status_rows(),
        ));

        children.push(GuiWidgetNode::layout(
            "plugins:stream-support:actions",
            "Stream Support Actions",
            GuiLayoutMode::ButtonWrap {
                min_button_width: 150.0,
            },
            vec![
                GuiWidgetNode::leaf(
                    "plugins:stream-support:install",
                    "Install Helper",
                    GuiWidgetKind::Button,
                    None,
                    self.stream_support_plugin_action_enabled("install"),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "plugins:stream-support:import-downloader",
                    "Import yt-dlp",
                    GuiWidgetKind::Button,
                    None,
                    self.stream_support_plugin_action_enabled("import"),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "plugins:stream-support:import-js-runtime",
                    "Import Deno",
                    GuiWidgetKind::Button,
                    None,
                    self.stream_support_plugin_action_enabled("import"),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "plugins:stream-support:open-location",
                    "Open Install Location",
                    GuiWidgetKind::Button,
                    None,
                    self.stream_helper.open_install_location_available,
                    false,
                ),
                GuiWidgetNode::leaf(
                    "plugins:stream-support:recheck",
                    "Recheck Support",
                    GuiWidgetKind::Button,
                    None,
                    self.stream_support_plugin_action_enabled("recheck"),
                    false,
                ),
                GuiWidgetNode::leaf(
                    "plugins:stream-support:retry",
                    "Retry URL",
                    GuiWidgetKind::Button,
                    None,
                    self.stream_support_plugin_action_enabled("retry"),
                    false,
                ),
            ],
        ));

        GuiWidgetNode::branch(
            "plugins:stream-support",
            "Stream Support",
            GuiWidgetKind::Panel,
            children,
        )
    }

    fn stream_support_plugin_status_rows(&self) -> Vec<GuiWidgetNode> {
        let mut rows = vec![
            GuiWidgetNode::leaf(
                "plugins:stream-support:title",
                "Title",
                GuiWidgetKind::Status,
                Some(self.stream_helper_status_title().to_owned()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:stream-support:summary",
                "Summary",
                GuiWidgetKind::Status,
                Some(self.stream_helper_status_summary()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:stream-support:health",
                "Health",
                GuiWidgetKind::Status,
                Some(self.stream_helper.health.label().to_owned()),
                true,
                false,
            ),
        ];

        if let Some(install_location) = self.stream_helper.install_location.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:stream-support:install-location",
                "Install Location",
                GuiWidgetKind::Status,
                Some(install_location.clone()),
                true,
                false,
            ));
        }
        if let Some(downloader_status) = self.stream_helper.downloader_status.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:stream-support:downloader-status",
                "yt-dlp",
                GuiWidgetKind::Status,
                Some(downloader_status.clone()),
                true,
                false,
            ));
        }
        if let Some(js_runtime_status) = self.stream_helper.js_runtime_status.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:stream-support:js-runtime-status",
                "Deno",
                GuiWidgetKind::Status,
                Some(js_runtime_status.clone()),
                true,
                false,
            ));
        }
        if let Some(target) = self.stream_helper.target.as_ref() {
            rows.push(GuiWidgetNode::leaf(
                "plugins:stream-support:target",
                "Target",
                GuiWidgetKind::Status,
                Some(target.clone()),
                true,
                false,
            ));
        }
        if self.stream_helper_remediation.active {
            rows.push(GuiWidgetNode::leaf(
                "plugins:stream-support:remediation",
                "Remediation",
                GuiWidgetKind::Status,
                self.stream_helper_remediation.label.clone(),
                true,
                false,
            ));
            rows.push(GuiWidgetNode::leaf(
                "plugins:stream-support:remediation-progress",
                "Progress",
                GuiWidgetKind::Status,
                Some(format!(
                    "{:.0}%",
                    self.stream_helper_remediation.progress_fraction * 100.0
                )),
                true,
                false,
            ));
            if let Some(detail) = self.stream_helper_remediation.detail.as_ref() {
                rows.push(GuiWidgetNode::leaf(
                    "plugins:stream-support:remediation-detail",
                    "Remediation Detail",
                    GuiWidgetKind::Status,
                    Some(detail.clone()),
                    true,
                    false,
                ));
            }
        }

        rows
    }

    fn stream_support_plugin_alert_widget_tree(&self) -> Option<GuiWidgetNode> {
        if self.stream_helper.health == GuiStreamHelperHealth::Healthy
            && !self.stream_helper_remediation.active
        {
            return None;
        }

        let level = match self.stream_helper.health {
            GuiStreamHelperHealth::Broken => GuiTransientNotificationLevel::Error,
            GuiStreamHelperHealth::Healthy if self.stream_helper_remediation.active => {
                GuiTransientNotificationLevel::Success
            }
            _ => GuiTransientNotificationLevel::Warning,
        };
        let message = self
            .stream_helper
            .message
            .clone()
            .unwrap_or_else(|| self.stream_helper_status_summary());

        let mut children = vec![
            GuiWidgetNode::leaf(
                "plugins:stream-support:alert:level",
                "Level",
                GuiWidgetKind::Status,
                Some(level.label().to_owned()),
                true,
                false,
            ),
            GuiWidgetNode::leaf(
                "plugins:stream-support:alert:message",
                "Message",
                GuiWidgetKind::Status,
                Some(message),
                true,
                false,
            ),
        ];

        let mut alert_actions = Vec::new();
        if self.stream_helper.install_supported {
            alert_actions.push(GuiWidgetNode::leaf(
                "plugins:stream-support:alert:install",
                "Install Helper",
                GuiWidgetKind::Button,
                None,
                self.stream_support_plugin_action_enabled("install"),
                false,
            ));
        }
        if self.stream_helper.retry_available {
            alert_actions.push(GuiWidgetNode::leaf(
                "plugins:stream-support:alert:retry",
                "Retry URL",
                GuiWidgetKind::Button,
                None,
                self.stream_support_plugin_action_enabled("retry"),
                false,
            ));
        }
        alert_actions.push(GuiWidgetNode::leaf(
            "plugins:stream-support:alert:recheck",
            "Recheck Support",
            GuiWidgetKind::Button,
            None,
            self.stream_support_plugin_action_enabled("recheck"),
            false,
        ));

        children.push(GuiWidgetNode::layout(
            "plugins:stream-support:alert:actions",
            "Alert Actions",
            GuiLayoutMode::ButtonWrap {
                min_button_width: 150.0,
            },
            alert_actions,
        ));

        Some(GuiWidgetNode::branch(
            "plugins:stream-support:alert",
            level.label(),
            GuiWidgetKind::Panel,
            children,
        ))
    }

    fn stream_support_plugin_action_enabled(&self, action: &str) -> bool {
        match action {
            "install" => {
                self.pending_operation.is_none()
                    && !self.stream_helper_remediation.active
                    && self.stream_helper.install_supported
            }
            "import" => {
                self.pending_operation.is_none()
                    && !self.stream_helper_remediation.active
                    && self.stream_helper.integration_supported
            }
            "recheck" => self.pending_operation.is_none() && !self.stream_helper_remediation.active,
            "retry" => {
                self.pending_operation.is_none()
                    && !self.stream_helper_remediation.active
                    && self.stream_helper.retry_available
            }
            _ => false,
        }
    }
}
