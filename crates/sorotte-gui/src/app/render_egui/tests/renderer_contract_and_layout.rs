use super::*;

#[test]
fn gui_widget_egui_renderer_rebuilds_widget_tree_from_renderer_contract() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp::default());
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
    assert_eq!(compact.column_width, 340.0);
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
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
    assert_eq!(compact.row_count, 3);

    let medium =
        GuiWidgetEguiRenderer::plan_responsive_columns(820.0, 12.0, 360.0, 3, spans.clone());
    assert_eq!(medium.column_count, 2);
    assert_eq!(medium.row_count, 2);

    let wide = GuiWidgetEguiRenderer::plan_responsive_columns(1280.0, 12.0, 360.0, 3, spans);
    assert_eq!(wide.column_count, 3);
    assert_eq!(wide.row_count, 1);
}

#[test]
fn gui_widget_egui_renderer_room_dashboard_breakpoints_stack_balance_and_widen() {
    assert_eq!(
        GuiWidgetEguiRenderer::room_dashboard_layout_for_width(520.0),
        super::super::GuiRoomDashboardLayout::Narrow
    );
    assert_eq!(
        GuiWidgetEguiRenderer::room_dashboard_layout_for_width(760.0),
        super::super::GuiRoomDashboardLayout::Medium
    );
    assert_eq!(
        GuiWidgetEguiRenderer::room_dashboard_layout_for_width(900.0),
        super::super::GuiRoomDashboardLayout::Wide
    );
    assert_eq!(
        GuiWidgetEguiRenderer::room_dashboard_layout_for_width(1440.0),
        super::super::GuiRoomDashboardLayout::Wide
    );
    assert_eq!(
        GuiWidgetEguiRenderer::room_dashboard_layout_for_width(4096.0),
        super::super::GuiRoomDashboardLayout::Wide
    );
}

#[test]
fn gui_widget_egui_renderer_room_dashboard_keeps_inset_inside_viewport() {
    assert_eq!(
        GuiWidgetEguiRenderer::room_dashboard_content_width(0.0),
        0.0
    );
    assert_eq!(
        GuiWidgetEguiRenderer::room_dashboard_content_width(820.0),
        796.0
    );
    assert_eq!(
        GuiWidgetEguiRenderer::room_dashboard_content_width(1600.0),
        1576.0
    );
}

#[test]
fn gui_widget_egui_renderer_room_dashboard_row_groups_align_wide_top_panels() {
    assert_eq!(
        GuiWidgetEguiRenderer::room_dashboard_row_groups_for_width(520.0),
        vec![vec![
            "main-window:connection",
            "main-window:playlist-column",
            "main-window:chat-panel",
        ]]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::room_dashboard_row_groups_for_width(760.0),
        vec![
            vec!["main-window:connection", "main-window:playlist-column"],
            vec!["main-window:chat-panel"],
        ]
    );
    assert_eq!(
        GuiWidgetEguiRenderer::room_dashboard_row_groups_for_width(920.0),
        vec![
            vec!["main-window:connection", "main-window:playlist-column"],
            vec!["main-window:chat-panel"],
        ]
    );
}

#[test]
fn gui_widget_egui_renderer_plugins_surface_uses_bounded_rail_on_wide_widths() {
    assert_eq!(
        GuiWidgetEguiRenderer::plugins_surface_split_for_width(520.0),
        None
    );

    let (medium_rail, medium_detail) =
        GuiWidgetEguiRenderer::plugins_surface_split_for_width(900.0)
            .expect("medium plugins surface should split");
    assert_eq!(medium_rail, 220.0);
    assert_eq!(medium_detail, 668.0);

    let (wide_rail, wide_detail) = GuiWidgetEguiRenderer::plugins_surface_split_for_width(1600.0)
        .expect("wide plugins surface should split");
    assert_eq!(wide_rail, 280.0);
    assert_eq!(wide_detail, 1308.0);
}

#[test]
fn gui_widget_egui_renderer_form_label_width_stays_inside_available_row() {
    assert_eq!(GuiWidgetEguiRenderer::form_label_width(560.0, 160.0), 160.0);
    assert_eq!(GuiWidgetEguiRenderer::form_label_width(240.0, 160.0), 96.0);
    assert_eq!(GuiWidgetEguiRenderer::form_label_width(60.0, 160.0), 60.0);
}

#[test]
fn gui_widget_egui_renderer_dark_semantic_palette_keeps_status_pairs_readable() {
    fn linear_component(value: u8) -> f32 {
        let channel = f32::from(value) / 255.0;
        if channel <= 0.03928 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(color: egui::Color32) -> f32 {
        (0.2126 * linear_component(color.r()))
            + (0.7152 * linear_component(color.g()))
            + (0.0722 * linear_component(color.b()))
    }

    fn contrast_ratio(a: egui::Color32, b: egui::Color32) -> f32 {
        let light = luminance(a).max(luminance(b));
        let dark = luminance(a).min(luminance(b));
        (light + 0.05) / (dark + 0.05)
    }

    let palette = GuiWidgetEguiRenderer::palette_for_dark_mode(true);
    assert!(contrast_ratio(palette.neutral_text, egui::Color32::from_rgb(38, 45, 54)) >= 4.5);
    assert!(contrast_ratio(palette.info_text, palette.info_bg) >= 4.5);
    assert!(contrast_ratio(palette.success_text, palette.success_bg) >= 4.5);
    assert!(contrast_ratio(palette.warning_text, palette.warning_bg) >= 4.5);
    assert!(contrast_ratio(palette.controlled_text, palette.controlled_bg) >= 4.5);
}
