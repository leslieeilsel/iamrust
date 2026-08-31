use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{AttachmentId, ConversationId, DomainError, GroupId, UserId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ConversationKind {
    Direct { peer_user_id: UserId },
    Group { group_id: GroupId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberRole {
    Member,
    Administrator,
    Owner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMember {
    pub user_id: UserId,
    pub role: MemberRole,
    pub nickname: Option<String>,
    pub muted_until: Option<DateTime<Utc>>,
    pub joined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: ConversationId,
    pub kind: ConversationKind,
    pub name: String,
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub avatar_attachment_id: Option<AttachmentId>,
    pub members: BTreeMap<UserId, ConversationMember>,
    pub muted: bool,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Conversation {
    pub fn direct(a: UserId, b: UserId, now: DateTime<Utc>) -> Result<Self, DomainError> {
        if a == b {
            return Err(DomainError::SelfTarget);
        }
        let mut members = BTreeMap::new();
        for user_id in [a, b] {
            members.insert(
                user_id,
                ConversationMember {
                    user_id,
                    role: MemberRole::Member,
                    nickname: None,
                    muted_until: None,
                    joined_at: now,
                },
            );
        }
        Ok(Self {
            id: ConversationId::new(),
            kind: ConversationKind::Direct { peer_user_id: b },
            name: String::new(),
            avatar_url: None,
            avatar_attachment_id: None,
            members,
            muted: false,
            pinned: false,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn group(
        owner_id: UserId,
        member_ids: impl IntoIterator<Item = UserId>,
        name: String,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        if name.trim().is_empty() || name.chars().count() > 80 {
            return Err(DomainError::Validation {
                field: "group_name",
                reason: "invalid_length",
            });
        }
        let mut members = BTreeMap::new();
        members.insert(
            owner_id,
            ConversationMember {
                user_id: owner_id,
                role: MemberRole::Owner,
                nickname: None,
                muted_until: None,
                joined_at: now,
            },
        );
        for user_id in member_ids {
            members.entry(user_id).or_insert(ConversationMember {
                user_id,
                role: MemberRole::Member,
                nickname: None,
                muted_until: None,
                joined_at: now,
            });
        }
        if members.len() < 2 {
            return Err(DomainError::Validation {
                field: "members",
                reason: "group_requires_two_members",
            });
        }
        Ok(Self {
            id: ConversationId::new(),
            kind: ConversationKind::Group {
                group_id: GroupId::new(),
            },
            name: name.trim().to_owned(),
            avatar_url: None,
            avatar_attachment_id: None,
            members,
            muted: false,
            pinned: false,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn can_read(&self, user_id: UserId) -> bool {
        self.members.contains_key(&user_id)
    }

    pub fn can_send(&self, user_id: UserId, now: DateTime<Utc>) -> bool {
        self.members
            .get(&user_id)
            .is_some_and(|member| member.muted_until.is_none_or(|until| until <= now))
    }

    pub fn remove_member(
        &mut self,
        actor: UserId,
        target: UserId,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let actor_role = self
            .members
            .get(&actor)
            .map(|member| member.role)
            .ok_or(DomainError::Forbidden)?;
        let target_role = self
            .members
            .get(&target)
            .map(|member| member.role)
            .ok_or(DomainError::NotFound)?;
        if target_role == MemberRole::Owner || actor_role <= target_role {
            return Err(DomainError::Forbidden);
        }
        self.members.remove(&target);
        self.updated_at = now;
        Ok(())
    }

    pub fn transfer_ownership(
        &mut self,
        actor: UserId,
        target: UserId,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if self.members.get(&actor).map(|member| member.role) != Some(MemberRole::Owner) {
            return Err(DomainError::Forbidden);
        }
        if !self.members.contains_key(&target) {
            return Err(DomainError::NotFound);
        }
        self.members.get_mut(&actor).expect("actor exists").role = MemberRole::Member;
        self.members.get_mut(&target).expect("target exists").role = MemberRole::Owner;
        self.updated_at = now;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    #[test]
    fn group_permissions_prevent_removing_owner() {
        let owner = UserId::new();
        let admin = UserId::new();
        let mut group =
            Conversation::group(owner, [admin], "Rust room".to_owned(), Utc::now()).unwrap();
        group.members.get_mut(&admin).unwrap().role = MemberRole::Administrator;
        assert_eq!(
            group.remove_member(admin, owner, Utc::now()),
            Err(DomainError::Forbidden)
        );
    }
}
