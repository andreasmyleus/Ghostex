//! Local overlays on top of the last daemon state.
//!
//! An overlay is a local-first edit the UI shows before the daemon confirms it. Overlays are
//! never written into the server rows and never renumber the revision: inventing `revision + 1`
//! for a local edit made the next real delta look stale and dropped it.

use std::collections::{BTreeMap, BTreeSet};

use ghostex_gx_protocol::{LifecycleState, PresentationSession, SessionActivity};
use serde::{Deserialize, Serialize};

/// How long a renamed session shows its new name before the daemon has to have published it.
pub const RENAME_PATCH_TTL_MS: u64 = 8_000;

/// An optimistic patch of one session. Only the fields that are `Some` are overlaid.
///
/// A patch always expires: the request it anticipates can fail silently, and an overlay that
/// never ends would hide the daemon's state for good. Build one with [`SessionPatch::lifecycle`]
/// or [`SessionPatch::activity`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SessionPatch {
    pub lifecycle_state: Option<LifecycleState>,
    pub activity: Option<SessionActivity>,
    pub is_pinned: Option<bool>,
    pub is_parked: Option<bool>,
    pub is_favorite: Option<bool>,
    /// Three states, not two. `None` is "this patch says nothing about the tag"; `Some(None)` is
    /// "the tag is cleared", which is what `/api/updateSession` is told with an explicit `null`
    /// and what a presentation row then models as an ABSENT field. Collapsing the two made a
    /// clear look like a no-op.
    pub session_tag: Option<Option<String>>,
    /// The name the user just gave the session, shown until the daemon publishes it (a gxserver
    /// that shows a pending rename does so within one round trip) or the patch expires.
    pub title: Option<String>,
    /// Host time after which the patch is dropped, checked on every tick against `now_ms`.
    pub expires_at_ms: u64,
}

impl SessionPatch {
    /// Shows `lifecycle_state` until the daemon reports it, reports something newer, or the patch
    /// expires.
    pub fn lifecycle(lifecycle_state: LifecycleState, expires_at_ms: u64) -> Self {
        Self {
            lifecycle_state: Some(lifecycle_state),
            ..Self::empty(expires_at_ms)
        }
    }

    /// A patch that says nothing yet. Build on it with the `with_` methods.
    pub fn empty(expires_at_ms: u64) -> Self {
        Self {
            lifecycle_state: None,
            activity: None,
            is_pinned: None,
            is_parked: None,
            is_favorite: None,
            session_tag: None,
            title: None,
            expires_at_ms,
        }
    }

    /// Shows `activity` until the daemon reports it, reports something newer, or the patch expires.
    pub fn activity(activity: SessionActivity, expires_at_ms: u64) -> Self {
        Self {
            activity: Some(activity),
            ..Self::empty(expires_at_ms)
        }
    }

    pub fn with_lifecycle(mut self, lifecycle_state: LifecycleState) -> Self {
        self.lifecycle_state = Some(lifecycle_state);
        self
    }

    pub fn with_activity(mut self, activity: SessionActivity) -> Self {
        self.activity = Some(activity);
        self
    }

    pub fn with_pinned(mut self, is_pinned: bool) -> Self {
        self.is_pinned = Some(is_pinned);
        self
    }

    pub fn with_parked(mut self, is_parked: bool) -> Self {
        self.is_parked = Some(is_parked);
        self
    }

    pub fn with_favorite(mut self, is_favorite: bool) -> Self {
        self.is_favorite = Some(is_favorite);
        self
    }

    /// CDXC:SessionTitles 2026-10-12 DECISION:
    /// User: "when i rename a session please apply the name optimistically for 8 seconds while the cli itself updates the name". The row shows the new name the moment Rename is confirmed; a gxserver with the pending-rename rule publishes it within one round trip and takes over (it owns the 8-second deadline, the revert and the failed attempt), and on an older daemon the patch itself ends after the same 8 seconds.
    /// SEE-ALSO: server/src/presentation/session_attributes.rs `pending_session_rename_title`.
    pub fn rename(title: String, now_ms: u64) -> Self {
        Self::empty(now_ms.saturating_add(RENAME_PATCH_TTL_MS)).with_title(title)
    }

    pub fn with_title(mut self, title: String) -> Self {
        self.title = Some(title);
        self
    }

    /// `None` clears the tag, which is the `null` `/api/updateSession` takes.
    pub fn with_session_tag(mut self, session_tag: Option<String>) -> Self {
        self.session_tag = Some(session_tag);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.lifecycle_state.is_none()
            && self.activity.is_none()
            && self.is_pinned.is_none()
            && self.is_parked.is_none()
            && self.is_favorite.is_none()
            && self.session_tag.is_none()
            && self.title.is_none()
    }

    /// The session as the UI should show it.
    pub fn apply_to(&self, server: &PresentationSession) -> PresentationSession {
        let mut session = server.clone();
        if let Some(lifecycle) = &self.lifecycle_state {
            session.lifecycle_state = lifecycle.clone();
        }
        if let Some(activity) = &self.activity {
            session.activity = activity.clone();
            // `attention` is present only while the activity is attention; a stale payload would
            // keep drawing the attention state after a local acknowledgement.
            if *activity != SessionActivity::Attention {
                session.attention = None;
            }
        }
        if let Some(is_pinned) = self.is_pinned {
            session.is_pinned = is_pinned;
        }
        if let Some(is_parked) = self.is_parked {
            session.is_parked = is_parked;
        }
        if let Some(is_favorite) = self.is_favorite {
            session.is_favorite = is_favorite;
        }
        if let Some(session_tag) = &self.session_tag {
            session.session_tag = session_tag.clone();
        }
        if let Some(title) = &self.title {
            session.title = title.clone();
            session.display_title = Some(title.clone());
            session.display_title_tooltip = Some(title.clone());
            session.primary_title = Some(title.clone());
            session.terminal_title = None;
        }
        session
    }
}

/// What the daemon row says now, compared with a stored patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PatchVerdict {
    /// The daemon still says what it said when the patch was made: keep overlaying.
    Pending,
    /// The daemon row says what the patch says: the patch has nothing left to do.
    CaughtUp,
    /// The daemon row changed to something else: it is newer than the patch, and the patch must
    /// not hide it.
    ServerMovedOn,
}

/// A patch plus the daemon values it was made against.
///
/// CDXC:StateSync 2026-09-19 WHY:
/// A patch must never hide daemon state that is newer than it. Example: the user acknowledges attention (patch activity to idle), the agent starts working again and raises a second attention event before the acknowledgement lands; comparing only "does the row equal the patch" would keep showing idle until the patch expired. Recording the base and dropping the patch as soon as the row leaves that base keeps the overlay strictly about the one transition it anticipated. The attention event id is part of the base because a new attention event has the same activity value as the old one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredPatch {
    pub(crate) patch: SessionPatch,
    base_lifecycle_state: LifecycleState,
    base_activity: SessionActivity,
    base_attention_event_id: Option<String>,
    base_is_pinned: bool,
    base_is_parked: bool,
    base_is_favorite: bool,
    base_session_tag: Option<String>,
    base_title: String,
}

fn attention_event_id(session: &PresentationSession) -> Option<&str> {
    session
        .attention
        .as_ref()
        .and_then(|attention| attention.event_id.as_deref())
}

impl StoredPatch {
    pub(crate) fn new(patch: SessionPatch, server: &PresentationSession) -> Self {
        Self {
            patch,
            base_lifecycle_state: server.lifecycle_state.clone(),
            base_activity: server.activity.clone(),
            base_attention_event_id: attention_event_id(server).map(str::to_string),
            base_is_pinned: server.is_pinned,
            base_is_parked: server.is_parked,
            base_is_favorite: server.is_favorite,
            base_session_tag: server.session_tag.clone(),
            base_title: server.title.clone(),
        }
    }

    pub(crate) fn verdict(&self, server: &PresentationSession) -> PatchVerdict {
        let mut pending = false;
        if let Some(lifecycle) = &self.patch.lifecycle_state {
            if *lifecycle != server.lifecycle_state {
                if server.lifecycle_state != self.base_lifecycle_state {
                    return PatchVerdict::ServerMovedOn;
                }
                pending = true;
            }
        }
        if let Some(activity) = &self.patch.activity {
            if *activity != server.activity {
                let same_base = server.activity == self.base_activity
                    && attention_event_id(server) == self.base_attention_event_id.as_deref();
                if !same_base {
                    return PatchVerdict::ServerMovedOn;
                }
                pending = true;
            }
        }
        // Every field the patch names asks the same two questions, in the same order: does the
        // row already say it, and if not, has the row left the value this patch predicted FROM.
        // A field added here without its base is a field whose overlay can never be dropped by a
        // daemon that moved on, so the bases are recorded together with the patch and read
        // together here.
        for (named, server_value, base) in [
            (self.patch.is_pinned, server.is_pinned, self.base_is_pinned),
            (self.patch.is_parked, server.is_parked, self.base_is_parked),
            (
                self.patch.is_favorite,
                server.is_favorite,
                self.base_is_favorite,
            ),
        ] {
            if let Some(value) = named {
                if value != server_value {
                    if server_value != base {
                        return PatchVerdict::ServerMovedOn;
                    }
                    pending = true;
                }
            }
        }
        if let Some(session_tag) = &self.patch.session_tag {
            if *session_tag != server.session_tag {
                if server.session_tag != self.base_session_tag {
                    return PatchVerdict::ServerMovedOn;
                }
                pending = true;
            }
        }
        if let Some(title) = &self.patch.title {
            if *title != server.title {
                if server.title != self.base_title {
                    return PatchVerdict::ServerMovedOn;
                }
                pending = true;
            }
        }
        if pending {
            PatchVerdict::Pending
        } else {
            PatchVerdict::CaughtUp
        }
    }
}

/// A set or map keyed by `(project_id, session_id)` that can be probed with `&str` parts, so the
/// hot lookups allocate nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectSessionMap<T> {
    by_project: BTreeMap<String, BTreeMap<String, T>>,
}

impl<T> Default for ProjectSessionMap<T> {
    fn default() -> Self {
        Self {
            by_project: BTreeMap::new(),
        }
    }
}

impl<T> ProjectSessionMap<T> {
    pub(crate) fn get(&self, project_id: &str, session_id: &str) -> Option<&T> {
        self.by_project.get(project_id)?.get(session_id)
    }

    pub(crate) fn get_mut(&mut self, project_id: &str, session_id: &str) -> Option<&mut T> {
        self.by_project.get_mut(project_id)?.get_mut(session_id)
    }

    pub(crate) fn contains(&self, project_id: &str, session_id: &str) -> bool {
        self.get(project_id, session_id).is_some()
    }

    pub(crate) fn insert(&mut self, project_id: &str, session_id: &str, value: T) -> Option<T> {
        self.by_project
            .entry(project_id.to_string())
            .or_default()
            .insert(session_id.to_string(), value)
    }

    pub(crate) fn remove(&mut self, project_id: &str, session_id: &str) -> Option<T> {
        let sessions = self.by_project.get_mut(project_id)?;
        let removed = sessions.remove(session_id);
        if sessions.is_empty() {
            self.by_project.remove(project_id);
        }
        removed
    }

    pub(crate) fn remove_project(&mut self, project_id: &str) -> BTreeMap<String, T> {
        self.by_project.remove(project_id).unwrap_or_default()
    }

    pub(crate) fn project(&self, project_id: &str) -> Option<&BTreeMap<String, T>> {
        self.by_project.get(project_id)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &str, &T)> {
        self.by_project.iter().flat_map(|(project_id, sessions)| {
            sessions
                .iter()
                .map(move |(session_id, value)| (project_id.as_str(), session_id.as_str(), value))
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.by_project.values().map(BTreeMap::len).sum()
    }
}

/// Every local overlay of one machine.
///
/// CDXC:StateSync 2026-09-19 WHY:
/// Overlays are kept beside the daemon rows instead of being written into them, and they never renumber the revision. The TypeScript runtime patched the snapshot in place, so any unrelated upsert of the same session wiped an optimistic value before the daemon had caught up; and an invented `revision + 1` once made the next real delta look stale.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Overlays {
    /// Sessions closed locally. Keeps a later hydrate from putting the row back while the daemon's
    /// transition catches up.
    pub(crate) hidden_sessions: ProjectSessionMap<()>,
    /// Projects removed locally (close to recent) before the daemon's removal arrives.
    pub(crate) hidden_projects: BTreeSet<String>,
    pub(crate) session_patches: ProjectSessionMap<StoredPatch>,
}

impl Overlays {
    pub(crate) fn is_session_hidden(&self, project_id: &str, session_id: &str) -> bool {
        self.hidden_projects.contains(project_id)
            || self.hidden_sessions.contains(project_id, session_id)
    }

    pub(crate) fn clear_project(&mut self, project_id: &str) {
        self.hidden_projects.remove(project_id);
        self.hidden_sessions.remove_project(project_id);
        self.session_patches.remove_project(project_id);
    }
}
