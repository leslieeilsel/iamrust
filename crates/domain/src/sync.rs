use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{ConversationId, DomainError, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    MessageCreated,
    MessageUpdated,
    ConversationUpdated,
    FriendshipUpdated,
    GroupMembershipUpdated,
    ReadPositionUpdated,
    DraftUpdated,
    PresenceUpdated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncEvent {
    pub id: Uuid,
    pub cursor: u64,
    pub kind: EventKind,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct SyncState {
    cursor: u64,
    seen: HashSet<Uuid>,
}

impl SyncState {
    pub const fn cursor(&self) -> u64 {
        self.cursor
    }

    pub fn apply(&mut self, event: &SyncEvent) -> Result<bool, DomainError> {
        if self.seen.contains(&event.id) {
            return Ok(false);
        }
        if event.cursor <= self.cursor {
            return Err(DomainError::StaleCursor);
        }
        self.seen.insert(event.id);
        self.cursor = event.cursor;
        Ok(true)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnreadCounters {
    per_conversation: BTreeMap<ConversationId, u32>,
}

impl UnreadCounters {
    pub fn increment(&mut self, conversation_id: ConversationId, sender: UserId, me: UserId) {
        if sender != me {
            let count = self.per_conversation.entry(conversation_id).or_default();
            *count = count.saturating_add(1);
        }
    }

    pub fn mark_read(&mut self, conversation_id: ConversationId) {
        self.per_conversation.remove(&conversation_id);
    }

    pub fn conversation(&self, conversation_id: ConversationId) -> u32 {
        self.per_conversation
            .get(&conversation_id)
            .copied()
            .unwrap_or_default()
    }

    pub fn total(&self) -> u32 {
        self.per_conversation.values().copied().sum()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;

    #[test]
    fn sync_is_monotonic_and_deduplicated() {
        let id = Uuid::now_v7();
        let event = SyncEvent {
            id,
            cursor: 1,
            kind: EventKind::MessageCreated,
            payload: json!({}),
            created_at: Utc::now(),
        };
        let mut state = SyncState::default();
        assert_eq!(state.apply(&event), Ok(true));
        assert_eq!(state.apply(&event), Ok(false));
        let stale = SyncEvent {
            id: Uuid::now_v7(),
            ..event
        };
        assert_eq!(state.apply(&stale), Err(DomainError::StaleCursor));
    }

    proptest! {
        #[test]
        fn strictly_increasing_cursors_are_applied_exactly_once(
            increments in prop::collection::vec(1_u16..=1_000, 1..200)
        ) {
            let mut state = SyncState::default();
            let mut cursor = 0_u64;
            for increment in increments {
                cursor += u64::from(increment);
                let event = SyncEvent {
                    id: Uuid::now_v7(),
                    cursor,
                    kind: EventKind::MessageCreated,
                    payload: json!({}),
                    created_at: Utc::now(),
                };
                prop_assert_eq!(state.apply(&event), Ok(true));
                prop_assert_eq!(state.apply(&event), Ok(false));
                prop_assert_eq!(state.cursor(), cursor);
            }
        }

        #[test]
        fn stale_cursors_never_move_state_forward(first in 1_u64..1_000_000, delta in 0_u64..1_000_000) {
            let mut state = SyncState::default();
            let accepted = SyncEvent {
                id: Uuid::now_v7(),
                cursor: first.saturating_add(delta).saturating_add(1),
                kind: EventKind::MessageCreated,
                payload: json!({}),
                created_at: Utc::now(),
            };
            state.apply(&accepted).unwrap();
            let before = state.cursor();
            let stale = SyncEvent { id: Uuid::now_v7(), cursor: first, ..accepted };
            prop_assert_eq!(state.apply(&stale), Err(DomainError::StaleCursor));
            prop_assert_eq!(state.cursor(), before);
        }
    }
}
