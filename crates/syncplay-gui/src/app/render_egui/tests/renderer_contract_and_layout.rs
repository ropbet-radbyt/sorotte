use super::*;

#[test]
fn gui_widget_egui_renderer_rebuilds_widget_tree_from_renderer_contract() {
    let state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
    let expected_tree = state.shell_widget_tree();
    let mut renderer = GuiWidgetEguiRenderer::default();

    state.render_shell_widgets(&mut renderer);

    assert_eq!(renderer.root(), Some(&expected_tree));
}

#[test]
fn gui_widget_egui_renderer_defaults_editable_fields_to_empty_text() {
    let password_node = GuiWidgetNode::leaf(
        "test:password",
        "Password",
        GuiWidgetKind::PasswordInput,
        None,
        true,
        false,
    );
    let text_node = GuiWidgetNode::leaf(
        "test:text",
        "Text",
        GuiWidgetKind::TextInput,
        Some("value".to_owned()),
        true,
        false,
    );

    assert_eq!(
        GuiWidgetEguiRenderer::editable_text_value(&password_node),
        ""
    );
    assert_eq!(
        GuiWidgetEguiRenderer::editable_text_value(&text_node),
        "value"
    );
}

#[test]
fn gui_widget_egui_renderer_responsive_column_planner_covers_compact_medium_and_wide_widths() {
    let compact = GuiWidgetEguiRenderer::plan_responsive_columns(340.0, 12.0, 360.0, 3, [1, 1, 1]);
    assert_eq!(compact.column_count, 1);
    assert_eq!(compact.row_count, 3);
    assert_eq!(
        compact.rows,
        vec![
            vec![super::super::GuiResponsiveColumnsPlanEntry {
                child_index: 0,
                column: 0,
                span: 1,
            }],
            vec![super::super::GuiResponsiveColumnsPlanEntry {
                child_index: 1,
                column: 0,
                span: 1,
            }],
            vec![super::super::GuiResponsiveColumnsPlanEntry {
                child_index: 2,
                column: 0,
                span: 1,
            }],
        ]
    );

    let medium = GuiWidgetEguiRenderer::plan_responsive_columns(820.0, 12.0, 360.0, 3, [1, 1, 1]);
    assert_eq!(medium.column_count, 2);
    assert_eq!(medium.row_count, 2);
    assert_eq!(
        medium.rows,
        vec![
            vec![
                super::super::GuiResponsiveColumnsPlanEntry {
                    child_index: 0,
                    column: 0,
                    span: 1,
                },
                super::super::GuiResponsiveColumnsPlanEntry {
                    child_index: 1,
                    column: 1,
                    span: 1,
                },
            ],
            vec![super::super::GuiResponsiveColumnsPlanEntry {
                child_index: 2,
                column: 0,
                span: 1,
            }],
        ]
    );

    let wide = GuiWidgetEguiRenderer::plan_responsive_columns(1280.0, 12.0, 360.0, 3, [1, 2, 1, 3]);
    assert_eq!(wide.column_count, 3);
    assert_eq!(wide.row_count, 3);
    assert_eq!(
        wide.rows,
        vec![
            vec![
                super::super::GuiResponsiveColumnsPlanEntry {
                    child_index: 0,
                    column: 0,
                    span: 1,
                },
                super::super::GuiResponsiveColumnsPlanEntry {
                    child_index: 1,
                    column: 1,
                    span: 2,
                },
            ],
            vec![super::super::GuiResponsiveColumnsPlanEntry {
                child_index: 2,
                column: 0,
                span: 1,
            }],
            vec![super::super::GuiResponsiveColumnsPlanEntry {
                child_index: 3,
                column: 0,
                span: 3,
            }],
        ]
    );
}

#[test]
fn gui_widget_egui_renderer_responsive_column_planner_clamps_requested_spans() {
    let plan = GuiWidgetEguiRenderer::plan_responsive_columns(820.0, 12.0, 360.0, 2, [3, 0, 1]);
    assert_eq!(plan.column_count, 2);
    assert_eq!(
        plan.rows,
        vec![
            vec![super::super::GuiResponsiveColumnsPlanEntry {
                child_index: 0,
                column: 0,
                span: 2,
            }],
            vec![
                super::super::GuiResponsiveColumnsPlanEntry {
                    child_index: 1,
                    column: 0,
                    span: 1,
                },
                super::super::GuiResponsiveColumnsPlanEntry {
                    child_index: 2,
                    column: 1,
                    span: 1,
                },
            ],
        ]
    );
}

#[test]
fn gui_widget_egui_renderer_main_window_top_region_scales_across_compact_medium_and_wide_widths() {
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        shared_playlist_enabled: Some(true),
        chat_input_enabled: Some(true),
        player_path: Some("mpv".to_owned()),
        room: Some("Lounge".to_owned()),
        ..StoredClientSettingsMvp::default()
    });
    assert!(
        state.apply(GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "One".to_owned(),
            "Two".to_owned(),
        ]))
    );

    let top_region = state
        .main_window_widget_tree()
        .find("main-window:top-region")
        .expect("main window top region should exist")
        .clone();
    let spans = top_region.children.iter().map(|child| child.column_span);

    let compact =
        GuiWidgetEguiRenderer::plan_responsive_columns(340.0, 12.0, 360.0, 3, spans.clone());
    assert_eq!(compact.column_count, 1);
    assert_eq!(compact.row_count, 2);

    let medium =
        GuiWidgetEguiRenderer::plan_responsive_columns(820.0, 12.0, 360.0, 3, spans.clone());
    assert_eq!(medium.column_count, 2);
    assert_eq!(medium.row_count, 2);

    let wide = GuiWidgetEguiRenderer::plan_responsive_columns(1280.0, 12.0, 360.0, 3, spans);
    assert_eq!(wide.column_count, 3);
    assert_eq!(wide.row_count, 1);
}
