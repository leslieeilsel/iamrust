export type UserId = string;
export type ConversationId = string;
export type MessageId = string;

export type Presence = 'online' | 'away' | 'busy' | 'invisible' | 'offline';
export type ProfileVisibility = 'everyone' | 'friends' | 'nobody';

export interface ProfilePrivacySettings {
  gender_visibility: ProfileVisibility;
  birthday_visibility: ProfileVisibility;
  region_visibility: ProfileVisibility;
  presence_visibility: ProfileVisibility;
  read_receipts_enabled: boolean;
}

export interface UserProfile {
  id: UserId;
  username: string;
  nickname: string;
  avatar_url: string | null;
  avatar_attachment_id: string | null;
  signature: string;
  gender: string | null;
  birthday: string | null;
  region: string | null;
  presence: Presence;
  last_seen_at: string | null;
}

export interface ConversationMember {
  user_id: UserId;
  role: 'member' | 'administrator' | 'owner';
  nickname: string | null;
  muted_until: string | null;
  joined_at: string;
}

export type ConversationKind =
  { kind: 'direct'; peer_user_id: UserId } | { kind: 'group'; group_id: string };

export interface Conversation {
  id: ConversationId;
  kind: ConversationKind;
  name: string;
  avatar_url: string | null;
  avatar_attachment_id: string | null;
  members: Record<UserId, ConversationMember>;
  muted: boolean;
  pinned: boolean;
  created_at: string;
  updated_at: string;
}

export interface GroupAnnouncement {
  id: string;
  conversation_id: ConversationId;
  author_id: UserId;
  content: string;
  read_by: UserId[];
  created_at: string;
  updated_at: string;
}

export interface GroupFileItem {
  message_id: MessageId;
  sender_id: UserId;
  attachment: Attachment;
  created_at: string;
}

export interface GroupJoinRequest {
  id: string;
  conversation_id: ConversationId;
  applicant_id: UserId;
  message: string;
  status: 'pending' | 'accepted' | 'rejected';
  reviewed_by: UserId | null;
  created_at: string;
  updated_at: string;
}

export interface GroupPollOption {
  id: string;
  label: string;
  voter_ids: UserId[];
}

export interface GroupPoll {
  id: string;
  conversation_id: ConversationId;
  creator_id: UserId;
  question: string;
  multiple_choice: boolean;
  options: GroupPollOption[];
  closes_at: string | null;
  created_at: string;
}

export interface Attachment {
  id: string;
  kind: 'image' | 'file' | 'audio' | 'video';
  file_name: string;
  mime_type: string;
  byte_size: number;
  sha256: string | null;
  storage_key: string;
  thumbnail_key: string | null;
}

export interface ForwardedMessage {
  sender_id: UserId;
  sender_name: string;
  content: MessageContent;
  created_at: string;
}

export interface Sticker {
  id: string;
  owner_id: UserId;
  attachment: Attachment;
  name: string;
  shortcut: string | null;
  created_at: string;
}

export type MessageContent =
  | { type: 'text'; data: { text: string } }
  | { type: 'image'; data: { attachment: Attachment } }
  | { type: 'file'; data: { attachment: Attachment } }
  | { type: 'audio'; data: { attachment: Attachment; duration_ms: number } }
  | { type: 'sticker'; data: { attachment: Attachment; name: string } }
  | {
      type: 'forward_bundle';
      data: { title: string; messages: ForwardedMessage[] };
    }
  | { type: 'system'; data: { text: string } };

export type MessageStatus = 'pending' | 'sent' | 'delivered' | 'read' | 'failed' | 'recalled';

export interface Message {
  id: MessageId;
  client_message_id: MessageId;
  conversation_id: ConversationId;
  sender_id: UserId;
  sequence: number | null;
  content: MessageContent;
  status: MessageStatus;
  reply_to: MessageId | null;
  mentions: UserId[];
  mention_all: boolean;
  created_at: string;
  server_created_at: string | null;
  edited_at: string | null;
}

export interface FriendRequest {
  id: string;
  sender_id: UserId;
  recipient_id: UserId;
  message: string;
  status: 'pending' | 'accepted' | 'rejected' | 'cancelled';
  created_at: string;
  updated_at: string;
}

export interface FriendSettings {
  user_id: UserId;
  remark: string | null;
  group: string | null;
  share_presence: boolean;
  allow_files: boolean;
}

export interface SessionResponse {
  access_expires_at: string;
  refresh_expires_at: string;
  profile: UserProfile;
  device_id: string;
}

export interface SecondFactorStatus {
  enabled: boolean;
  recovery_codes_remaining: number;
}

export interface SecondFactorSetupResponse {
  secret: string;
  otpauth_uri: string;
  expires_at: string;
}

export interface RecoveryCodesResponse {
  recovery_codes: string[];
}

export interface QrLoginStartResponse {
  challenge_id: string;
  secret: string;
  qr_payload: string;
  expires_at: string;
}

export interface QrLoginPollResponse {
  status: 'pending' | 'ready';
  session: SessionResponse | null;
}

export interface DeviceInfo {
  id: string;
  name: string;
  platform: string;
  app_version: string;
  last_seen_at: string;
  current: boolean;
}

export interface BootstrapResponse {
  profile: UserProfile;
  profile_privacy: ProfilePrivacySettings;
  friends: UserProfile[];
  friend_settings: FriendSettings[];
  friend_requests: FriendRequest[];
  friend_request_profiles: UserProfile[];
  conversations: Conversation[];
  conversation_states: ConversationState[];
  cursor: number;
  server_features: Record<string, unknown>;
}

export interface Page<T> {
  items: T[];
  next_cursor: string | null;
}

export interface SyncEvent {
  id: string;
  cursor: number;
  kind:
    | 'message_created'
    | 'message_updated'
    | 'conversation_updated'
    | 'friendship_updated'
    | 'group_membership_updated'
    | 'read_position_updated'
    | 'draft_updated'
    | 'presence_updated';
  payload: Record<string, unknown>;
  created_at: string;
}

export interface SyncResponse {
  events: SyncEvent[];
  next_cursor: number;
  has_more: boolean;
}

export interface MessageAck {
  client_message_id: MessageId;
  message_id: MessageId;
  sequence: number;
  server_time: string;
}

export interface MessageReaction {
  emoji: string;
  user_ids: UserId[];
}

export interface MessageDetails {
  message: Message;
  reactions: MessageReaction[];
  delivered_to: UserId[];
  read_by: UserId[];
  favorited: boolean;
  expires_at: string | null;
}

export interface TranslateMessageResponse {
  source_language: string | null;
  target_language: string;
  translated_text: string;
}

export interface TranscribeMessageResponse {
  text: string;
  language: string | null;
}

export interface ScheduledMessageInfo {
  schedule_id: string;
  conversation_id: ConversationId;
  content: MessageContent;
  reply_to: MessageId | null;
  mentions: UserId[];
  mention_all: boolean;
  scheduled_for: string;
  expires_in_seconds: number | null;
}

export interface ScheduledMessageResponse {
  schedule_id: string;
  scheduled_for: string;
}

export interface PersonalDataExport {
  generated_at: string;
  email: string;
  profile: UserProfile;
  privacy: ProfilePrivacySettings;
  friend_ids: UserId[];
  friend_requests: FriendRequest[];
  conversations: Conversation[];
  messages: Message[];
}

export type ConnectionState = 'connecting' | 'online' | 'offline' | 'syncing' | 'failed';
export type AppSection = 'conversations' | 'contacts' | 'search' | 'settings';
export type ThemePreference = 'system' | 'light' | 'dark' | 'high-contrast';
export type SendShortcut = 'enter' | 'mod-enter';

export interface AppSettings {
  theme: ThemePreference;
  compactMode: boolean;
  fontScale: number;
  language: 'zh-CN' | 'en-US';
  sendShortcut: SendShortcut;
  notifications: boolean;
  notificationSound: boolean;
  notificationPreview: boolean;
  privacyMode: boolean;
  crashReporting: boolean;
  doNotDisturbEnabled: boolean;
  doNotDisturbStart: string;
  doNotDisturbEnd: string;
  closeBehavior: 'tray' | 'quit';
  keepCacheOnLogout: boolean;
  autostart: boolean;
  globalShortcutEnabled: boolean;
  downloadDirectory: string;
  localDatabaseEncryption: boolean;
}

export interface ConversationMeta {
  unread: number;
  lastMessage: Message | null;
  draft: string;
  hidden: boolean;
  manuallyUnread: boolean;
  lastReadSequence: number;
  label: string | null;
}

export interface ConversationState {
  conversation_id: ConversationId;
  pinned: boolean;
  muted: boolean;
  hidden: boolean;
  manually_unread: boolean;
  last_read_sequence: number;
  unread_count: number;
  draft: string;
  label: string | null;
}

export interface PendingUpload {
  localId: string;
  file: File;
  previewUrl: string | null;
  progress: number;
  status: 'ready' | 'uploading' | 'failed' | 'completed';
  error: string | null;
}
