//! The grip along the top of an Agents pane while the workspace is split: drag it to move the
//! pane's session to another pane, click it for the pane menu.

use gpui::AnyElement;
use gpui::AppContext as _;
use gpui::FontWeight;
use gpui::InteractiveElement as _;
use gpui::IntoElement;
use gpui::ParentElement as _;
use gpui::Render;
use gpui::StatefulInteractiveElement as _;
use gpui::Styled as _;
use gpui::Window;
use gpui::div;
use gpui::prelude::FluentBuilder as _;
use gpui::px;
use gpui_component::ElementExt as _;

use crate::app::actions::{CloseAgentsPane, MergeAllTabsForPane};
use crate::app::context_menu::GpuiContextMenu;
use crate::app::helpers::*;
use crate::app::model::*;
use crate::*;

/// The strip's height. Every pane of a split keeps it, so moving focus between panes never
/// resizes a terminal grid.
const WORKSPACE_PANE_GRIP_HEIGHT: f32 = 10.0;
const WORKSPACE_PANE_GRIP_GROUP: &str = "ghostex-gpui-workspace-pane-grip";

/// What follows the pointer while a pane's session is dragged by its grip.
pub(crate) struct WorkspacePaneDragPreview {
    title: String,
}

impl Render for WorkspacePaneDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .h(px(28.0))
            .max_w(px(240.0))
            .px(px(10.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(workspace_drop_feedback_border_color())
            .bg(workspace_tab_drag_preview_color())
            .shadow_md()
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(titlebar_active_text_color())
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(self.title.clone()),
            )
    }
}

impl GhostexGpuiApp {
    /// CDXC:Workarea 2026-09-23 DECISION:
    /// User: a small bar at the top of the active session pane opens a menu to close it or to merge all agent sessions, and dragging it moves the session to another pane (an edge splits there, the middle moves it in). It exists only while the workspace is split, since one pane has nowhere to go; every pane of the split keeps the strip's height so focusing a pane never resizes its terminal, and only the focused pane draws its handle (the others show it on hover).
    pub(crate) fn render_workspace_pane_grip(
        &self,
        leaf: &WorkspaceLeaf,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        if self.agents_workspace.leaf_order().len() <= 1 {
            return None;
        }
        let pane_id = leaf.pane_id;
        let session_id = leaf.tab_group.active_session_id();
        let focused = self.agents_workspace.focused_pane == pane_id;
        let view = cx.entity().downgrade();
        let grip_bounds =
            std::rc::Rc::new(std::cell::Cell::new(None::<gpui::Bounds<gpui::Pixels>>));
        let menu_bounds = grip_bounds.clone();
        let title = session_id
            .map(|session_id| self.agents_workspace_tab_display_title(session_id))
            .unwrap_or_default();
        Some(
            div()
                .id(format!("ghostex-gpui-workspace-pane-grip-{}", pane_id.0))
                .on_prepaint(move |bounds, _, _| grip_bounds.set(Some(bounds)))
                .group(WORKSPACE_PANE_GRIP_GROUP)
                .flex_shrink_0()
                .w_full()
                .h(px(WORKSPACE_PANE_GRIP_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .cursor_grab()
                // A chat pane's strip is the chat's own surface, so it reads as part of the pane
                // and, under the header, as part of the band above it. Under glass the chat is
                // clear too, so the strip stays clear rather than drawing an opaque band.
                .when(
                    session_id.is_some_and(|session_id| {
                        self.agents_chat_mode_sessions.contains(&session_id)
                    }),
                    |grip| grip.bg(glass_clear(gpui_session_chat_background_color())),
                )
                .child(
                    div()
                        .w(px(36.0))
                        .h(px(4.0))
                        .rounded(px(2.0))
                        .bg(titlebar_icon_color())
                        .when(!focused, |pill| {
                            pill.opacity(0.0)
                                .group_hover(WORKSPACE_PANE_GRIP_GROUP, |pill| pill.opacity(0.5))
                        }),
                )
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.focus_agents_pane(pane_id, cx);
                    this.show_workspace_pane_grip_menu(pane_id, menu_bounds.get(), window, cx);
                }))
                .when_some(session_id, |grip, session_id| {
                    grip.on_drag(
                        DraggedWorkspaceTab {
                            source_pane_id: pane_id,
                            session_id,
                        },
                        move |_dragged, _offset, _window, cx| {
                            let _ = view.update(cx, |this, cx| this.begin_workspace_tab_drag(cx));
                            let title = title.clone();
                            cx.new(|_| WorkspacePaneDragPreview { title })
                        },
                    )
                })
                .into_any_element(),
        )
    }

    /// Hung below the grip and toggled by it: a click on the grip while its menu is open closes it.
    fn show_workspace_pane_grip_menu(
        &self,
        pane_id: WorkspacePaneId,
        grip: Option<gpui::Bounds<gpui::Pixels>>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let menu = GpuiContextMenu::new()
            .menu(
                "Close Pane",
                Box::new(CloseAgentsPane { pane_id: pane_id.0 }),
            )
            .menu(
                "Merge All Panes",
                Box::new(MergeAllTabsForPane { pane_id: pane_id.0 }),
            );
        match grip {
            Some(grip) => menu.toggle_below(grip, window, cx),
            None => menu.show(window.mouse_position(), window, cx),
        }
    }

    /// Close Pane from the grip menu: the sessions it held keep running in the neighbouring pane.
    pub(crate) fn close_agents_pane(
        &mut self,
        pane_id: WorkspacePaneId,
        cx: &mut gpui::Context<Self>,
    ) {
        if self
            .agents_workspace
            .close_pane_keeping_sessions(pane_id)
            .is_none()
        {
            return;
        }
        self.focus_shell_target(
            ShellFocusTarget::AgentsPane(self.agents_workspace.focused_pane),
            cx,
        );
        self.workspace_drop_feedback = None;
        self.workspace_split_drag = None;
        self.workspace_split_layout_metrics.clear();
        self.persist_shell_layout_state();
        cx.notify();
    }
}
