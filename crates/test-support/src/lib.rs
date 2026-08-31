use chrono::Utc;
use iamrust_domain::{Presence, UserId, UserProfile};

pub fn user(username: &str) -> UserProfile {
    UserProfile {
        id: UserId::new(),
        username: username.to_owned(),
        nickname: username.to_owned(),
        avatar_url: None,
        avatar_attachment_id: None,
        signature: String::new(),
        gender: None,
        birthday: None,
        region: None,
        presence: Presence::Offline,
        last_seen_at: Some(Utc::now()),
    }
}
