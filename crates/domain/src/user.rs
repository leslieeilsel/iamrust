use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{AttachmentId, DomainError, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Online,
    Away,
    Busy,
    Invisible,
    #[default]
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProfileVisibility {
    Everyone,
    #[default]
    Friends,
    Nobody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfilePrivacySettings {
    pub gender_visibility: ProfileVisibility,
    pub birthday_visibility: ProfileVisibility,
    pub region_visibility: ProfileVisibility,
    pub presence_visibility: ProfileVisibility,
    pub read_receipts_enabled: bool,
}

impl Default for ProfilePrivacySettings {
    fn default() -> Self {
        Self {
            gender_visibility: ProfileVisibility::Friends,
            birthday_visibility: ProfileVisibility::Friends,
            region_visibility: ProfileVisibility::Friends,
            presence_visibility: ProfileVisibility::Friends,
            read_receipts_enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: UserId,
    pub username: String,
    pub nickname: String,
    pub avatar_url: Option<Url>,
    #[serde(default)]
    pub avatar_attachment_id: Option<AttachmentId>,
    pub signature: String,
    #[serde(default)]
    pub gender: Option<String>,
    #[serde(default)]
    pub birthday: Option<NaiveDate>,
    #[serde(default)]
    pub region: Option<String>,
    pub presence: Presence,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserProfileUpdate {
    pub nickname: String,
    pub signature: String,
    pub avatar_url: Option<Url>,
    pub avatar_attachment_id: Option<AttachmentId>,
    pub gender: Option<String>,
    pub birthday: Option<NaiveDate>,
    pub region: Option<String>,
    pub presence: Presence,
}

impl UserProfile {
    pub fn update_public_fields(&mut self, update: UserProfileUpdate) -> Result<(), DomainError> {
        let UserProfileUpdate {
            nickname,
            signature,
            avatar_url,
            avatar_attachment_id,
            gender,
            birthday,
            region,
            presence,
        } = update;
        let nickname_count = nickname.trim().chars().count();
        if !(1..=48).contains(&nickname_count) {
            return Err(DomainError::Validation {
                field: "nickname",
                reason: "invalid_length",
            });
        }
        if signature.chars().count() > 160 {
            return Err(DomainError::Validation {
                field: "signature",
                reason: "invalid_length",
            });
        }
        let gender = normalize_optional(gender, 32, "gender")?;
        let region = normalize_optional(region, 96, "region")?;
        if birthday.is_some_and(|date| date > Utc::now().date_naive()) {
            return Err(DomainError::Validation {
                field: "birthday",
                reason: "future_date",
            });
        }
        self.nickname = nickname.trim().to_owned();
        self.signature = signature;
        self.avatar_url = avatar_url;
        self.avatar_attachment_id = avatar_attachment_id;
        self.gender = gender;
        self.birthday = birthday;
        self.region = region;
        self.presence = presence;
        Ok(())
    }
}

fn normalize_optional(
    value: Option<String>,
    max_length: usize,
    field: &'static str,
) -> Result<Option<String>, DomainError> {
    let value = value.map(|value| value.trim().to_owned());
    if value
        .as_ref()
        .is_some_and(|value| value.chars().count() > max_length)
    {
        return Err(DomainError::Validation {
            field,
            reason: "invalid_length",
        });
    }
    Ok(value.filter(|value| !value.is_empty()))
}
