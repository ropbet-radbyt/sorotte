use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GuiMenuShortcut {
    pub(super) shortcut: egui::KeyboardShortcut,
    pub(super) action_id: MenuActionId,
}

impl GuiWidgetEguiRenderer {
    pub(super) fn menu_shortcuts() -> [GuiMenuShortcut; 4] {
        [
            GuiMenuShortcut {
                shortcut: egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::G,
                ),
                action_id: MenuActionId::UndoSeek,
            },
            GuiMenuShortcut {
                shortcut: egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::O,
                ),
                action_id: MenuActionId::SetOffset,
            },
            GuiMenuShortcut {
                shortcut: egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O),
                action_id: MenuActionId::OpenMedia,
            },
            GuiMenuShortcut {
                shortcut: egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::G),
                action_id: MenuActionId::Seek,
            },
        ]
    }

    pub(super) fn actions_for_menu_shortcut(
        shortcut: egui::KeyboardShortcut,
    ) -> Vec<GuiShellAction> {
        Self::menu_shortcuts()
            .into_iter()
            .find(|binding| binding.shortcut == shortcut)
            .map(|binding| vec![GuiShellAction::InvokeMenuAction(binding.action_id)])
            .unwrap_or_default()
    }

    pub(super) fn consume_menu_shortcuts(
        ctx: &egui::Context,
        state: &SorotteGuiShellAppState,
    ) -> Vec<GuiShellAction> {
        if state.open_modal.is_some() || ctx.wants_keyboard_input() {
            return Vec::new();
        }

        ctx.input_mut(|input| {
            let mut actions = Vec::new();
            for binding in Self::menu_shortcuts() {
                if input.consume_shortcut(&binding.shortcut) {
                    actions.extend(Self::actions_for_menu_shortcut(binding.shortcut));
                }
            }
            actions
        })
    }
}
