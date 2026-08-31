use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{DomainError, FriendRequestId, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FriendRequestStatus {
    Pending,
    Accepted,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendRequest {
    pub id: FriendRequestId,
    pub sender_id: UserId,
    pub recipient_id: UserId,
    pub message: String,
    pub status: FriendRequestStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl FriendRequest {
    pub fn new(
        sender_id: UserId,
        recipient_id: UserId,
        message: String,
        now: DateTime<Utc>,
    ) -> Result<Self, DomainError> {
        if sender_id == recipient_id {
            return Err(DomainError::SelfTarget);
        }
        if message.chars().count() > 240 {
            return Err(DomainError::Validation {
                field: "verification_message",
                reason: "invalid_length",
            });
        }
        Ok(Self {
            id: FriendRequestId::new(),
            sender_id,
            recipient_id,
            message,
            status: FriendRequestStatus::Pending,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn accept(&mut self, actor: UserId, now: DateTime<Utc>) -> Result<(), DomainError> {
        self.transition(actor, FriendRequestStatus::Accepted, now)
    }

    pub fn reject(&mut self, actor: UserId, now: DateTime<Utc>) -> Result<(), DomainError> {
        self.transition(actor, FriendRequestStatus::Rejected, now)
    }

    fn transition(
        &mut self,
        actor: UserId,
        next: FriendRequestStatus,
        now: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        if actor != self.recipient_id {
            return Err(DomainError::Forbidden);
        }
        if self.status != FriendRequestStatus::Pending {
            return Err(DomainError::InvalidTransition {
                from: "final",
                to: "final",
            });
        }
        self.status = next;
        self.updated_at = now;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Friendship {
    pub lower_user_id: UserId,
    pub upper_user_id: UserId,
    pub created_at: DateTime<Utc>,
}

impl Friendship {
    pub fn new(a: UserId, b: UserId, now: DateTime<Utc>) -> Result<Self, DomainError> {
        if a == b {
            return Err(DomainError::SelfTarget);
        }
        let (lower_user_id, upper_user_id) = if a < b { (a, b) } else { (b, a) };
        Ok(Self {
            lower_user_id,
            upper_user_id,
            created_at: now,
        })
    }

    pub fn contains(&self, user_id: UserId) -> bool {
        self.lower_user_id == user_id || self.upper_user_id == user_id
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    #[test]
    fn only_recipient_can_accept_pending_request() {
        let sender = UserId::new();
        let recipient = UserId::new();
        let mut request =
            FriendRequest::new(sender, recipient, "hello".to_owned(), Utc::now()).unwrap();
        assert_eq!(
            request.accept(sender, Utc::now()),
            Err(DomainError::Forbidden)
        );
        request.accept(recipient, Utc::now()).unwrap();
        assert_eq!(request.status, FriendRequestStatus::Accepted);
        assert!(request.reject(recipient, Utc::now()).is_err());
    }
}
