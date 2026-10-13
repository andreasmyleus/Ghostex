//! The commit review window's state and behaviour (`GitCommitModal` in
//! packages/core-ui/git-commit-modal.tsx (deleted 2026-10-01)); drawing is in `render.rs`.
//!
//! CDXC:Git 2026-09-28 DECISION:
//! User: migrate every React modal to GPUI with GPUI-Kit components, "looking and working exactly as now", so the app runs without CEF. The commit review keeps its two-pane workspace (file rail with Select and Show All, message editor, delete-after toggle; the diff pane with its three display controls), its footer agent select and actions, the Merge to main confirmation, the right-click Copy Path menu, and the same remembered agent (`ghostex.promptAgent.gitCommit`) and diff options (`ghostex.gitCommitModal.diffPreferences.v1`) in client storage.
//! SEE-ALSO: packages/core-ui/git-commit-modal.tsx (deleted 2026-10-01), git-file-diff-modal.tsx (deleted 2026-10-01), changed-files-tree.tsx (deleted 2026-10-01) and changed-files-tree-utils.ts (deleted 2026-10-01), and the `.git-commit-*`, `.git-file-diff-*` rules in packages/core-ui/styles/modals.css, modals-light.css and commands.css (the React twins); apps/desktop/src/app/git_commit_modal_lifecycle.rs (open, commands, diff delivery); apps/desktop/src/app/gx_store/git/review.rs (what the commands do); apps/desktop/src/bin/native_modal_demo/git_commit.rs (standalone preview).
use super::super::create_worktree_modal::trim_prompt_editor_trailing_spaces;
use super::super::native_modal_kit::*;
use super::diff::{GitDiffPrefs, GitDiffViewMode, all_files_diff_draft};
use super::diff_view::{GitDiffPalette, GitDiffSkin, GitDiffView};
use super::focus::{CommitFocus, FocusTarget, RowFocus};
use super::model::*;
use gpui::Focusable as _;
use gpui::{
    App, AppContext as _, ClipboardItem, Context, Entity, FocusHandle, KeyDownEvent, ListAlignment,
    ListState, Subscription, Window, px,
};
use gpui_component::input::{Escape, IndentInline, InputEvent, OutdentInline, TextareaState};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// `APP_MODAL_HOST_GIT_COMMIT_WINDOW_WIDTH` / `_HEIGHT`: the React dialog fills this child window.
pub(crate) const GIT_COMMIT_MODAL_WIDTH: f32 = 1078.0;
pub(crate) const GIT_COMMIT_MODAL_HEIGHT: f32 = 758.0;

/// A configured agent with a launch command (the React modal lists only those).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitCommitAgent {
    pub(crate) agent_id: String,
    pub(crate) name: String,
}

/// `onConfirm` / `onDirectMerge`: the review this answers, the normalized message, the chosen
/// agent, the selected files (only when some are left out) and the options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitCommitConfirm {
    pub(crate) request_id: String,
    pub(crate) message: String,
    pub(crate) agent_id: Option<String>,
    pub(crate) file_paths: Option<Vec<String>>,
    pub(crate) commit_on_new_ref: bool,
    pub(crate) delete_worktree_after: bool,
}

/// What the dialog asks its host to do. The closing commands (`Confirm`, `DirectMerge`,
/// `MultipleCommits`, `Cancel`) are sent after the dialog removed its own window.
pub(crate) enum GitCommitModalCommand {
    /// `confirmSidebarGitCommit`.
    Confirm(GitCommitConfirm),
    /// `confirmSidebarGitDirectMerge`.
    DirectMerge(GitCommitConfirm),
    /// `runSidebarGitMultipleCommits`.
    MultipleCommits {
        request_id: String,
        agent_id: Option<String>,
    },
    /// `cancelSidebarGitCommit`.
    Cancel { request_id: String },
    /// `openSidebarGitChangedFileDiff`: the answer arrives through `receive_file_diff`.
    OpenFileDiff {
        request_id: String,
        file_path: String,
    },
    /// `openSidebarGitChangedFile` with `openLocation`.
    OpenFileLocation {
        request_id: String,
        file_path: String,
    },
    /// The footer agent choice, remembered in `ghostex.promptAgent.gitCommit`.
    PromptAgentChanged(String),
    /// The diff display options, remembered in `ghostex.gitCommitModal.diffPreferences.v1`.
    DiffPrefsChanged(GitDiffPrefs),
    /// Copy Path wrote the clipboard (the copy sound).
    PathCopied,
}

pub(crate) type GitCommitModalHost = Rc<dyn Fn(GitCommitModalCommand, &mut App)>;

pub(crate) struct GitCommitModalConfig {
    pub(crate) draft: GitCommitDraft,
    pub(crate) agents: Vec<GitCommitAgent>,
    /// The host's resolved prompt agent: the remembered choice, else Settings' default prompt
    /// agent, else the first agent (`resolvePromptAgentModalSelection`).
    pub(crate) prompt_agent_id: Option<String>,
    pub(crate) diff_prefs: GitDiffPrefs,
    pub(crate) palette: ModalPalette,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InlineDiffMode {
    All,
    File,
}

pub(crate) struct GpuiGitCommitModalWindow {
    pub(super) host: GitCommitModalHost,
    pub(super) palette: ModalPalette,
    pub(super) diff_palette: GitDiffPalette,
    pub(super) draft: GitCommitDraft,
    pub(super) agents: Vec<GitCommitAgent>,
    pub(super) selected_agent_id: String,
    tree: Vec<ChangedFilesTreeNode>,
    collapsed: HashSet<String>,
    pub(super) rows: Rc<Vec<ChangedFilesTreeRow>>,
    pub(super) tree_list: ListState,
    pub(super) excluded: HashSet<String>,
    pub(super) editing_files: bool,
    pub(super) delete_worktree_after: bool,
    pub(super) merge_confirm_open: bool,
    /// The confirmation's focus trap and its two buttons; Radix focuses Cancel when it opens.
    pub(super) merge_trap_focus: FocusHandle,
    pub(super) merge_cancel_focus: FocusHandle,
    pub(super) merge_confirm_focus: FocusHandle,
    pub(super) focus: CommitFocus,
    pub(super) inline_mode: InlineDiffMode,
    diff_cache: HashMap<String, GitFileDiffDraft>,
    all_loading_path: Option<String>,
    pub(super) selected_diff_path: Option<String>,
    loading_diff_path: Option<String>,
    pub(super) prefs: GitDiffPrefs,
    pub(super) diff: GitDiffView,
    pub(super) message: Entity<TextareaState>,
    pub(super) agent_select: ModalSelect,
    pub(super) focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl GpuiGitCommitModalWindow {
    pub(crate) fn new(
        config: GitCommitModalConfig,
        host: GitCommitModalHost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = config.draft;
        let message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Leave empty to auto-generate")
                .default_value(draft.initial_message())
        });
        let subscriptions = vec![cx.subscribe_in(
            &message,
            window,
            |_this: &mut Self, _input, event: &InputEvent, _window, cx| {
                if matches!(
                    event,
                    InputEvent::Change | InputEvent::Focus | InputEvent::Blur
                ) {
                    cx.notify();
                }
            },
        )];
        let focus_handle = cx.focus_handle();
        // `autoFocus` on the message editor; without it the dialog frame takes the keys.
        if draft.show_commit_message {
            message.update(cx, |message, cx| message.focus(window, cx));
        } else {
            focus_handle.focus(window, cx);
        }
        let selected_agent_id = config
            .prompt_agent_id
            .filter(|id| config.agents.iter().any(|agent| &agent.agent_id == id))
            .or_else(|| config.agents.first().map(|agent| agent.agent_id.clone()))
            .unwrap_or_default();
        let tree = build_changed_files_tree(&draft.changed_files);
        let collapsed = HashSet::new();
        let rows = Rc::new(flatten_changed_files_tree(&tree, &collapsed));
        let tree_list = ListState::new(rows.len(), ListAlignment::Top, px(200.0));
        let initial_diff_path = draft.changed_files.first().map(|file| file.path.clone());
        let mut this = Self {
            host,
            palette: config.palette,
            diff_palette: GitDiffPalette::resolve(&config.palette, GitDiffSkin::CommitReview),
            delete_worktree_after: draft.delete_worktree_after_default,
            draft,
            agents: config.agents,
            selected_agent_id,
            tree,
            collapsed,
            rows,
            tree_list,
            excluded: HashSet::new(),
            editing_files: false,
            merge_confirm_open: false,
            merge_trap_focus: cx.focus_handle(),
            merge_cancel_focus: cx.focus_handle().tab_stop(true),
            merge_confirm_focus: cx.focus_handle().tab_stop(true),
            focus: CommitFocus::new(cx),
            inline_mode: InlineDiffMode::File,
            diff_cache: HashMap::new(),
            all_loading_path: None,
            selected_diff_path: initial_diff_path.clone(),
            loading_diff_path: initial_diff_path.clone(),
            prefs: config.diff_prefs,
            diff: GitDiffView::new(window, cx),
            message,
            agent_select: ModalSelect::new(),
            focus_handle,
            _subscriptions: subscriptions,
        };
        this.diff.focus_on_select(this.focus_handle.clone(), cx);
        if let Some(path) = initial_diff_path {
            this.request_file_diff(path, cx);
        }
        this.sync_diff_view();
        this
    }

    fn request_file_diff(&self, file_path: String, cx: &mut App) {
        (self.host)(
            GitCommitModalCommand::OpenFileDiff {
                request_id: self.draft.request_id.clone(),
                file_path,
            },
            cx,
        );
    }

    /// A `gitFileDiff` answer for the open review: cached, and drawn when it is the file shown.
    pub(crate) fn receive_file_diff(&mut self, draft: GitFileDiffDraft, cx: &mut Context<Self>) {
        let path = draft.file_path.clone();
        self.diff_cache.insert(path.clone(), draft);
        if self.loading_diff_path.as_deref() == Some(path.as_str()) {
            self.loading_diff_path = None;
        }
        if self.all_loading_path.as_deref() == Some(path.as_str()) {
            self.all_loading_path = None;
        }
        self.load_next_all_files_diff(cx);
        self.sync_diff_view();
        cx.notify();
    }

    /// Show All asks for the missing patches one at a time, in list order.
    fn load_next_all_files_diff(&mut self, cx: &mut App) {
        if self.inline_mode != InlineDiffMode::All || self.all_loading_path.is_some() {
            return;
        }
        let Some(path) = self
            .draft
            .changed_files
            .iter()
            .find(|file| !self.diff_cache.contains_key(&file.path))
            .map(|file| file.path.clone())
        else {
            return;
        };
        self.all_loading_path = Some(path.clone());
        self.request_file_diff(path, cx);
    }

    pub(super) fn selected_diff_draft(&self) -> Option<&GitFileDiffDraft> {
        self.selected_diff_path
            .as_ref()
            .and_then(|path| self.diff_cache.get(path))
    }

    pub(super) fn is_selected_diff_loading(&self) -> bool {
        self.inline_mode == InlineDiffMode::File
            && self.selected_diff_path.is_some()
            && self.loading_diff_path == self.selected_diff_path
            && self.selected_diff_draft().is_none()
    }

    /// The patch the right pane draws: every file's for Show All, else the selected file's.
    fn active_patch(&self) -> Option<String> {
        match self.inline_mode {
            InlineDiffMode::All => {
                all_files_diff_draft(&self.draft.changed_files, &self.diff_cache)
                    .map(|draft| draft.patch)
            }
            InlineDiffMode::File => self.selected_diff_draft().map(|draft| draft.patch.clone()),
        }
    }

    pub(super) fn sync_diff_view(&mut self) {
        let patch = self.active_patch();
        self.diff.set_patch(patch.as_deref(), self.prefs);
    }

    /// `activeDiffStats`.
    pub(super) fn active_diff_stat(&self) -> GitFileStat {
        match self.inline_mode {
            InlineDiffMode::All => summarize_changed_files(&self.draft.changed_files),
            InlineDiffMode::File => self
                .selected_diff_draft()
                .map(|draft| GitFileStat {
                    additions: draft.additions.unwrap_or(0),
                    deletions: draft.deletions.unwrap_or(0),
                })
                .unwrap_or_default(),
        }
    }

    pub(super) fn active_diff_label(&self) -> Option<String> {
        match self.inline_mode {
            InlineDiffMode::All => Some("All files".to_string()),
            InlineDiffMode::File => self.selected_diff_path.clone(),
        }
    }

    pub(super) fn selected_files(&self) -> impl Iterator<Item = &GitChangedFile> {
        self.draft
            .changed_files
            .iter()
            .filter(|file| !self.excluded.contains(&file.path))
    }

    pub(super) fn selected_count(&self) -> usize {
        self.selected_files().count()
    }

    pub(super) fn all_selected(&self) -> bool {
        let total = self.draft.changed_files.len();
        total > 0 && self.selected_count() == total
    }

    fn none_selected(&self) -> bool {
        !self.draft.changed_files.is_empty() && self.selected_count() == 0
    }

    /// `canConfirm`: a commit needs at least one selected file.
    pub(super) fn can_confirm(&self) -> bool {
        !self.draft.show_commit_message || !self.none_selected()
    }

    pub(super) fn can_run_direct_merge(&self) -> bool {
        self.can_confirm() && !self.selected_agent_id.is_empty()
    }

    fn selected_file_paths(&self) -> Option<Vec<String>> {
        let files = &self.draft.changed_files;
        let selected: Vec<String> = self
            .selected_files()
            .map(|file| file.path.clone())
            .collect();
        (!files.is_empty() && selected.len() != files.len()).then_some(selected)
    }

    /// CDXC:Git 2026-05-28-07:47:
    /// Commit messages should not carry trailing spaces at line ends. Normalize pasted text in the textarea and normalize again before confirm, while a fully blank message still reaches native as empty so auto-generation works.
    fn trimmed_message(&self, cx: &App) -> String {
        trim_prompt_editor_trailing_spaces(&self.message.read(cx).value())
            .trim()
            .to_string()
    }

    fn agent_id(&self) -> Option<String> {
        (!self.selected_agent_id.is_empty()).then(|| self.selected_agent_id.clone())
    }

    fn confirm_payload(&self, commit_on_new_ref: bool, cx: &App) -> GitCommitConfirm {
        GitCommitConfirm {
            request_id: self.draft.request_id.clone(),
            message: self.trimmed_message(cx),
            agent_id: self.agent_id(),
            file_paths: self.selected_file_paths(),
            commit_on_new_ref,
            delete_worktree_after: self.delete_worktree_after,
        }
    }

    pub(super) fn confirm(
        &mut self,
        commit_on_new_ref: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_confirm() {
            return;
        }
        let payload = self.confirm_payload(commit_on_new_ref, cx);
        window.remove_window();
        (self.host)(GitCommitModalCommand::Confirm(payload), cx);
    }

    pub(super) fn multiple_commits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_confirm() {
            return;
        }
        let command = GitCommitModalCommand::MultipleCommits {
            request_id: self.draft.request_id.clone(),
            agent_id: self.agent_id(),
        };
        window.remove_window();
        (self.host)(command, cx);
    }

    pub(super) fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request_id = self.draft.request_id.clone();
        window.remove_window();
        (self.host)(GitCommitModalCommand::Cancel { request_id }, cx);
    }

    pub(super) fn open_merge_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_run_direct_merge() {
            return;
        }
        self.close_popups();
        self.merge_confirm_open = true;
        self.merge_cancel_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn close_merge_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.merge_confirm_open = false;
        self.refocus(window, cx);
        cx.notify();
    }

    pub(super) fn confirm_direct_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_run_direct_merge() {
            return;
        }
        self.merge_confirm_open = false;
        let payload = self.confirm_payload(false, cx);
        window.remove_window();
        (self.host)(GitCommitModalCommand::DirectMerge(payload), cx);
    }

    /// Back to the message editor (or the frame) after a popup or the confirmation closes.
    fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.draft.show_commit_message {
            self.message
                .update(cx, |message, cx| message.focus(window, cx));
        } else {
            self.focus_handle.focus(window, cx);
        }
    }

    pub(super) fn open_inline_file_diff(&mut self, path: String, cx: &mut Context<Self>) {
        self.inline_mode = InlineDiffMode::File;
        self.selected_diff_path = Some(path.clone());
        self.loading_diff_path = Some(path.clone());
        self.request_file_diff(path, cx);
        self.sync_diff_view();
        cx.notify();
    }

    pub(super) fn show_all_file_diffs(&mut self, cx: &mut Context<Self>) {
        self.inline_mode = InlineDiffMode::All;
        self.loading_diff_path = None;
        self.load_next_all_files_diff(cx);
        self.sync_diff_view();
        cx.notify();
    }

    pub(super) fn toggle_editing_files(&mut self, cx: &mut Context<Self>) {
        self.editing_files = !self.editing_files;
        cx.notify();
    }

    pub(super) fn toggle_file(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.excluded.remove(path) {
            self.excluded.insert(path.to_string());
        }
        cx.notify();
    }

    pub(super) fn toggle_all_files(&mut self, cx: &mut Context<Self>) {
        if self.all_selected() {
            self.excluded = self
                .draft
                .changed_files
                .iter()
                .map(|file| file.path.clone())
                .collect();
        } else {
            self.excluded.clear();
        }
        cx.notify();
    }

    pub(super) fn toggle_directory(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(path) {
            self.collapsed.insert(path.to_string());
        }
        self.rows = Rc::new(flatten_changed_files_tree(&self.tree, &self.collapsed));
        self.tree_list.reset(self.rows.len());
        cx.notify();
    }

    pub(super) fn toggle_delete_worktree_after(&mut self, cx: &mut Context<Self>) {
        self.delete_worktree_after = !self.delete_worktree_after;
        cx.notify();
    }

    pub(super) fn set_diff_prefs(&mut self, prefs: GitDiffPrefs, cx: &mut Context<Self>) {
        self.prefs = prefs;
        (self.host)(GitCommitModalCommand::DiffPrefsChanged(prefs), cx);
        self.sync_diff_view();
        cx.notify();
    }

    /// `setGitCommitPromptAgentId(undefined)` after the Default Prompt Agent changed: the new
    /// default, else the first agent.
    pub(crate) fn reset_prompt_agent(
        &mut self,
        default_agent_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        self.selected_agent_id = self
            .agents
            .iter()
            .find(|agent| Some(agent.agent_id.as_str()) == default_agent_id)
            .or_else(|| self.agents.first())
            .map(|agent| agent.agent_id.clone())
            .unwrap_or_default();
        cx.notify();
    }

    pub(super) fn selected_agent_index(&self) -> Option<usize> {
        self.agents
            .iter()
            .position(|agent| agent.agent_id == self.selected_agent_id)
    }

    pub(super) fn toggle_agent_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.agent_select.toggle(self.selected_agent_index());
        if self.agent_select.open {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn choose_agent(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.agent_select.close();
        if let Some(agent) = self.agents.get(index) {
            self.selected_agent_id = agent.agent_id.clone();
            (self.host)(
                GitCommitModalCommand::PromptAgentChanged(agent.agent_id.clone()),
                cx,
            );
        }
        self.refocus(window, cx);
        cx.notify();
    }

    pub(super) fn dismiss_agent_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.agent_select.close();
        self.refocus(window, cx);
        cx.notify();
    }

    fn close_popups(&mut self) {
        self.agent_select.close();
    }

    /// CDXC:Git 2026-06-08-09:41:
    /// Commit review file rows need a right-click Copy path action. Keep the menu local to file rows so directory expansion and normal left-click diff preview behavior stay unchanged.
    pub(super) fn copy_file_path(&mut self, path: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(path));
        (self.host)(GitCommitModalCommand::PathCopied, cx);
    }

    pub(super) fn open_file_location(&mut self, path: String, cx: &mut Context<Self>) {
        (self.host)(
            GitCommitModalCommand::OpenFileLocation {
                request_id: self.draft.request_id.clone(),
                file_path: path,
            },
            cx,
        );
    }

    /// Plain-text paste into the message drops trailing spaces per line (`handleMessagePaste`);
    /// a paste that changes nothing is left to the editor. The message field's paste hook, so it
    /// gets the field's own clipboard read, made off the UI thread (CDXC:Clipboard in
    /// helpers/clipboard_read.rs).
    pub(super) fn paste_message(
        &mut self,
        item: &ClipboardItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(text) = item.text() else {
            return false;
        };
        let trimmed = trim_prompt_editor_trailing_spaces(&text);
        if text.is_empty() || trimmed == text {
            return false;
        }
        self.message.update(cx, |message, cx| {
            message.replace(trimmed, window, cx);
        });
        cx.notify();
        true
    }

    /// A browser textarea hands Tab and Shift+Tab to focus navigation; gpui-component's editor
    /// would indent the message instead.
    pub(super) fn on_indent_action(
        &mut self,
        _: &IndentInline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.message.read(cx).focus_handle(cx).is_focused(window) {
            cx.stop_propagation();
            window.focus_next(cx);
        }
    }

    pub(super) fn on_outdent_action(
        &mut self,
        _: &OutdentInline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.message.read(cx).focus_handle(cx).is_focused(window) {
            cx.stop_propagation();
            window.focus_prev(cx);
        }
    }

    /// The editor turns Escape into its own action; the dialog's Escape runs here first.
    pub(super) fn on_escape_action(
        &mut self,
        _: &Escape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.escape(window, cx);
    }

    /// Escape closes the innermost open layer: the agent list, the confirmation, then the dialog.
    /// (An open file menu takes Escape itself.)
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.agent_select.open {
            self.dismiss_agent_select(window, cx);
        } else if self.merge_confirm_open {
            self.close_merge_confirm(window, cx);
        } else {
            self.cancel(window, cx);
        }
    }

    pub(super) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        if key == "escape" {
            self.escape(window, cx);
            cx.stop_propagation();
            return;
        }
        if self.merge_confirm_open {
            // The focused button answers Enter and Space; Tab walks the pair inside the trap.
            if matches!(key, "enter" | "space") {
                if self.merge_confirm_focus.is_focused(window) {
                    self.confirm_direct_merge(window, cx);
                } else {
                    self.close_merge_confirm(window, cx);
                }
                cx.stop_propagation();
            }
            return;
        }
        if self.agent_select.open {
            match self.agent_select.handle_key(key, self.agents.len()) {
                ModalSelectKey::Consumed => {
                    if !self.agent_select.open {
                        self.refocus(window, cx);
                    }
                    cx.notify();
                }
                ModalSelectKey::Choose(index) => self.choose_agent(index, window, cx),
                ModalSelectKey::Ignored => return,
            }
            cx.stop_propagation();
            return;
        }
        if self.activate_focused(key, window, cx) {
            cx.stop_propagation();
        }
    }

    /// Enter or Space on the focused control, the way a browser activates a focused button (both
    /// keys), a checkbox (Space only) or a select trigger (Enter, Space and the arrows open it).
    fn activate_focused(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let button = matches!(key, "enter" | "space");
        if let Some(row) = self.focus.focused_row(window) {
            match row {
                RowFocus::Directory(path) if button => self.toggle_directory(&path, cx),
                RowFocus::Open(path) if button => self.open_inline_file_diff(path, cx),
                RowFocus::Include(path) if key == "space" => self.toggle_file(&path, cx),
                _ => return false,
            }
            return true;
        }
        let Some(target) = self.focus.focused_target(window) else {
            return false;
        };
        let prefs = self.prefs;
        match target {
            FocusTarget::IncludeAll if key == "space" => self.toggle_all_files(cx),
            FocusTarget::DeleteAfter if key == "space" => self.toggle_delete_worktree_after(cx),
            FocusTarget::Agent if matches!(key, "enter" | "space" | "up" | "down") => {
                self.toggle_agent_select(window, cx)
            }
            _ if !button => return false,
            FocusTarget::SelectFiles => self.toggle_editing_files(cx),
            FocusTarget::ShowAll => self.show_all_file_diffs(cx),
            FocusTarget::ViewMode => self.set_diff_prefs(
                GitDiffPrefs {
                    view_mode: match prefs.view_mode {
                        GitDiffViewMode::Split => GitDiffViewMode::Unified,
                        GitDiffViewMode::Unified => GitDiffViewMode::Split,
                    },
                    ..prefs
                },
                cx,
            ),
            // Disabled while side by side forces wrapping.
            FocusTarget::LineWrap if prefs.view_mode == GitDiffViewMode::Split => {}
            FocusTarget::LineWrap => self.set_diff_prefs(
                GitDiffPrefs {
                    line_wrap: !prefs.line_wrap,
                    ..prefs
                },
                cx,
            ),
            FocusTarget::Whitespace => self.set_diff_prefs(
                GitDiffPrefs {
                    hide_whitespace: !prefs.hide_whitespace,
                    ..prefs
                },
                cx,
            ),
            FocusTarget::Cancel => self.cancel(window, cx),
            FocusTarget::Merge => self.open_merge_confirm(window, cx),
            FocusTarget::NewBranch => self.confirm(true, window, cx),
            FocusTarget::Multiple => self.multiple_commits(window, cx),
            FocusTarget::Confirm => self.confirm(false, window, cx),
            FocusTarget::IncludeAll | FocusTarget::DeleteAfter | FocusTarget::Agent => {
                return false;
            }
        }
        true
    }

    /// Preview hooks for the standalone demo binary.
    #[allow(dead_code)] // used by src/bin/native_modal_demo/git_commit.rs only
    pub(crate) fn preview_state(
        &mut self,
        state: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match state {
            "select" => {
                self.editing_files = true;
                if let Some(file) = self.draft.changed_files.get(1) {
                    self.excluded.insert(file.path.clone());
                }
            }
            "all" => self.show_all_file_diffs(cx),
            "confirm" => self.open_merge_confirm(window, cx),
            "agents" => self.toggle_agent_select(window, cx),
            "split" => {
                let prefs = GitDiffPrefs {
                    view_mode: GitDiffViewMode::Split,
                    ..self.prefs
                };
                self.set_diff_prefs(prefs, cx);
            }
            "wrap" => {
                let prefs = GitDiffPrefs {
                    line_wrap: true,
                    ..self.prefs
                };
                self.set_diff_prefs(prefs, cx);
            }
            "collapsed" => {
                if let Some(row) = self.rows.iter().find(|row| row.directory.is_some()) {
                    let path = row.path.clone();
                    self.toggle_directory(&path, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }
}

impl ModalCornerClose for GpuiGitCommitModalWindow {
    fn close_from_corner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel(window, cx);
    }
}
