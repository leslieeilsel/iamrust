import { invoke, type InvokeArgs } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import { clearRefreshToken, loadRefreshToken, saveRefreshToken } from './credentials';
import type {
  Attachment,
  BootstrapResponse,
  Conversation,
  ConversationId,
  DeviceInfo,
  FriendRequest,
  FriendSettings,
  GroupAnnouncement,
  GroupFileItem,
  GroupJoinRequest,
  GroupPoll,
  Message,
  MessageAck,
  MessageContent,
  MessageId,
  MessageDetails,
  MessageReaction,
  Page,
  PersonalDataExport,
  ProfilePrivacySettings,
  QrLoginPollResponse,
  QrLoginStartResponse,
  RecoveryCodesResponse,
  ScheduledMessageInfo,
  ScheduledMessageResponse,
  SessionResponse,
  SecondFactorSetupResponse,
  SecondFactorStatus,
  Sticker,
  SyncResponse,
  TranscribeMessageResponse,
  TranslateMessageResponse,
  UserId,
  UserProfile,
} from './types';

const API_BASE = (import.meta.env.VITE_API_URL ?? 'http://127.0.0.1:3780').replace(/\/$/u, '');
const IS_TAURI = '__TAURI_INTERNALS__' in window;

function currentDeviceName(): string {
  return navigator.userAgent.trim().slice(0, 80) || 'I Am Rust Desktop';
}

function currentPlatform(): string {
  return navigator.platform.trim().slice(0, 32) || 'desktop';
}

interface WireSessionResponse extends SessionResponse {
  access_token: string;
  refresh_token: string;
}

interface RemoteResponse {
  status: number;
  body: unknown;
}

interface ApiErrorBody {
  code?: string;
  message_key?: string;
  field?: string | null;
  correlation_id?: string;
  retryable?: boolean;
}

export class ApiClientError extends Error {
  readonly status: number;
  readonly code: string;
  readonly field: string | null;
  readonly retryable: boolean;

  constructor(status: number, body: ApiErrorBody = {}) {
    super(body.message_key ?? `request_failed_${status}`);
    this.name = 'ApiClientError';
    this.status = status;
    this.code = body.code ?? 'unknown';
    this.field = body.field ?? null;
    this.retryable = body.retryable ?? status >= 500;
  }
}

export interface DownloadResult {
  path: string | null;
  file_name: string;
  byte_size: number;
}

class ApiClient {
  private accessToken: string | null = null;
  private refreshToken: string | null = null;
  private refreshPromise: Promise<SessionResponse> | null = null;

  async restore(): Promise<SessionResponse | null> {
    if (IS_TAURI) {
      const response = await invokeRemote('remote_restore');
      if (response.status === 204) return null;
      return unwrapRemote<SessionResponse>(response);
    }
    this.refreshToken = loadRefreshToken();
    if (!this.refreshToken) return null;
    try {
      return await this.refresh();
    } catch (error) {
      if (error instanceof ApiClientError && error.status === 0) throw error;
      this.clearSession();
      return null;
    }
  }

  async register(input: {
    email: string;
    username: string;
    password: string;
    nickname: string;
    device_name: string;
  }): Promise<SessionResponse> {
    const request = {
      ...input,
      device_name: input.device_name.trim().slice(0, 80) || currentDeviceName(),
      platform: currentPlatform(),
      app_version: '0.1.0',
    };
    if (IS_TAURI) {
      return unwrapRemote<SessionResponse>(await invokeRemote('remote_register', { request }));
    }
    const session = await this.request<WireSessionResponse>(
      '/api/v1/auth/register',
      {
        method: 'POST',
        body: JSON.stringify(request),
      },
      false,
    );
    return this.setSession(session);
  }

  async login(
    login: string,
    password: string,
    secondFactorCode?: string,
  ): Promise<SessionResponse> {
    const request = {
      login,
      password,
      second_factor_code: secondFactorCode?.trim() || null,
      device_name: currentDeviceName(),
      platform: currentPlatform(),
      app_version: '0.1.0',
    };
    if (IS_TAURI) {
      return unwrapRemote<SessionResponse>(await invokeRemote('remote_login', { request }));
    }
    const session = await this.request<WireSessionResponse>(
      '/api/v1/auth/login',
      {
        method: 'POST',
        body: JSON.stringify(request),
      },
      false,
    );
    return this.setSession(session);
  }

  async beginQrLogin(): Promise<QrLoginStartResponse> {
    const request = {
      device_name: currentDeviceName(),
      platform: currentPlatform(),
      app_version: '0.1.0',
    };
    if (IS_TAURI) {
      return unwrapRemote<QrLoginStartResponse>(
        await invokeRemote('remote_begin_qr_login', { request }),
      );
    }
    return this.request(
      '/api/v1/auth/qr-login',
      {
        method: 'POST',
        body: JSON.stringify(request),
      },
      false,
    );
  }

  async pollQrLogin(challengeId: string, secret: string): Promise<QrLoginPollResponse> {
    if (IS_TAURI) {
      return unwrapRemote<QrLoginPollResponse>(
        await invokeRemote('remote_poll_qr_login', {
          challengeId,
          request: { secret },
        }),
      );
    }
    const response = await this.request<
      Omit<QrLoginPollResponse, 'session'> & { session: WireSessionResponse | null }
    >(
      `/api/v1/auth/qr-login/${challengeId}/poll`,
      { method: 'POST', body: JSON.stringify({ secret }) },
      false,
    );
    return {
      ...response,
      session: response.session ? this.setSession(response.session) : null,
    };
  }

  async approveQrLogin(challengeId: string, secret: string): Promise<void> {
    await this.request(`/api/v1/auth/qr-login/${challengeId}/approve`, {
      method: 'POST',
      body: JSON.stringify({ secret }),
    });
  }

  async logout(): Promise<void> {
    if (IS_TAURI) {
      unwrapRemote(await invokeRemote('remote_logout'));
      return;
    }
    const token = this.refreshToken;
    try {
      if (token) {
        await this.request(
          '/api/v1/auth/logout',
          {
            method: 'POST',
            body: JSON.stringify({ refresh_token: token }),
          },
          false,
        );
      }
    } finally {
      this.clearSession();
    }
  }

  async requestPasswordReset(email: string): Promise<void> {
    const request = { email };
    if (IS_TAURI) {
      unwrapRemote(await invokeRemote('remote_request_password_reset', { request }));
      return;
    }
    await this.request(
      '/api/v1/auth/password-reset/request',
      { method: 'POST', body: JSON.stringify(request) },
      false,
    );
  }

  async confirmPasswordReset(resetToken: string, newPassword: string): Promise<void> {
    const request = { reset_token: resetToken, new_password: newPassword };
    if (IS_TAURI) {
      unwrapRemote(await invokeRemote('remote_confirm_password_reset', { request }));
      return;
    }
    await this.request(
      '/api/v1/auth/password-reset/confirm',
      { method: 'POST', body: JSON.stringify(request) },
      false,
    );
  }

  async changePassword(currentPassword: string, newPassword: string): Promise<void> {
    await this.request('/api/v1/auth/change-password', {
      method: 'POST',
      body: JSON.stringify({ current_password: currentPassword, new_password: newPassword }),
    });
  }

  async secondFactorStatus(): Promise<SecondFactorStatus> {
    return this.request('/api/v1/me/second-factor');
  }

  async beginSecondFactorSetup(): Promise<SecondFactorSetupResponse> {
    return this.request('/api/v1/me/second-factor', { method: 'POST' });
  }

  async enableSecondFactor(code: string): Promise<RecoveryCodesResponse> {
    return this.request('/api/v1/me/second-factor/enable', {
      method: 'POST',
      body: JSON.stringify({ code }),
    });
  }

  async disableSecondFactor(currentPassword: string, code: string): Promise<void> {
    await this.request('/api/v1/me/second-factor', {
      method: 'DELETE',
      body: JSON.stringify({ current_password: currentPassword, code }),
    });
  }

  async regenerateRecoveryCodes(
    currentPassword: string,
    code: string,
  ): Promise<RecoveryCodesResponse> {
    return this.request('/api/v1/me/second-factor/recovery-codes', {
      method: 'POST',
      body: JSON.stringify({ current_password: currentPassword, code }),
    });
  }

  async devices(): Promise<DeviceInfo[]> {
    return this.request('/api/v1/devices');
  }

  async revokeDevice(deviceId: string): Promise<void> {
    await this.request(`/api/v1/devices/${deviceId}`, { method: 'DELETE' });
  }

  async bootstrap(): Promise<BootstrapResponse> {
    return this.request('/api/v1/bootstrap');
  }

  async updateProfile(input: {
    nickname: string;
    signature: string;
    avatar_url: string | null;
    avatar_attachment_id: string | null;
    gender: string | null;
    birthday: string | null;
    region: string | null;
    presence: UserProfile['presence'];
  }): Promise<UserProfile> {
    return this.request('/api/v1/me', { method: 'PATCH', body: JSON.stringify(input) });
  }

  async updateProfilePrivacy(settings: ProfilePrivacySettings): Promise<ProfilePrivacySettings> {
    return this.request('/api/v1/me/privacy', {
      method: 'PATCH',
      body: JSON.stringify(settings),
    });
  }

  async exportPersonalData(): Promise<PersonalDataExport> {
    return this.request('/api/v1/me/export');
  }

  async deleteAccount(currentPassword: string, confirmation: string): Promise<void> {
    await this.request('/api/v1/me', {
      method: 'DELETE',
      body: JSON.stringify({ current_password: currentPassword, confirmation }),
    });
    if (IS_TAURI) {
      await invokeRemote('remote_logout');
    } else {
      this.clearSession();
    }
  }

  async searchUser(username: string): Promise<UserProfile[]> {
    const search = new URLSearchParams({ username });
    return this.request(`/api/v1/users/search?${search.toString()}`);
  }

  async sendFriendRequest(username: string, message: string): Promise<FriendRequest> {
    return this.request('/api/v1/friend-requests', {
      method: 'POST',
      body: JSON.stringify({ username, message }),
    });
  }

  async decideFriendRequest(
    requestId: string,
    decision: 'accept' | 'reject',
  ): Promise<FriendRequest> {
    return this.request(`/api/v1/friend-requests/${requestId}`, {
      method: 'PATCH',
      body: JSON.stringify({ decision }),
    });
  }

  async updateFriendSettings(
    friendId: UserId,
    settings: Omit<FriendSettings, 'user_id'>,
  ): Promise<FriendSettings> {
    return this.request(`/api/v1/friends/${friendId}`, {
      method: 'PATCH',
      body: JSON.stringify(settings),
    });
  }

  async deleteFriend(friendId: UserId): Promise<void> {
    await this.request(`/api/v1/friends/${friendId}`, { method: 'DELETE' });
  }

  async blockUser(userId: UserId): Promise<void> {
    await this.request(`/api/v1/blocks/${userId}`, { method: 'POST' });
  }

  async unblockUser(userId: UserId): Promise<void> {
    await this.request(`/api/v1/blocks/${userId}`, { method: 'DELETE' });
  }

  async reportUser(userId: UserId, reason: string, details: string): Promise<void> {
    await this.request(`/api/v1/reports/${userId}`, {
      method: 'POST',
      body: JSON.stringify({ reason, details: details.trim() || null }),
    });
  }

  async createDirect(peerUserId: UserId): Promise<Conversation> {
    return this.request('/api/v1/conversations/direct', {
      method: 'POST',
      body: JSON.stringify({ peer_user_id: peerUserId }),
    });
  }

  async createGroup(name: string, memberIds: UserId[]): Promise<Conversation> {
    return this.request('/api/v1/conversations/group', {
      method: 'POST',
      body: JSON.stringify({ name, member_ids: memberIds }),
    });
  }

  async groupSettings(conversationId: ConversationId): Promise<{ mute_all: boolean }> {
    return this.request(`/api/v1/groups/${conversationId}`);
  }

  async updateGroup(
    conversationId: ConversationId,
    input: {
      name?: string;
      avatar_url?: string | null;
      avatar_attachment_id?: string | null;
    },
  ): Promise<Conversation> {
    return this.request(`/api/v1/groups/${conversationId}`, {
      method: 'PATCH',
      body: JSON.stringify(input),
    });
  }

  async addGroupMembers(
    conversationId: ConversationId,
    memberIds: UserId[],
  ): Promise<Conversation> {
    return this.request(`/api/v1/groups/${conversationId}/members`, {
      method: 'POST',
      body: JSON.stringify({ member_ids: memberIds }),
    });
  }

  async updateGroupMember(
    conversationId: ConversationId,
    memberId: UserId,
    input: {
      nickname?: string | null;
      role?: 'member' | 'administrator';
      muted_until?: string | null;
    },
  ): Promise<Conversation> {
    return this.request(`/api/v1/groups/${conversationId}/members/${memberId}`, {
      method: 'PATCH',
      body: JSON.stringify(input),
    });
  }

  async removeGroupMember(conversationId: ConversationId, memberId: UserId): Promise<void> {
    await this.request(`/api/v1/groups/${conversationId}/members/${memberId}`, {
      method: 'DELETE',
    });
  }

  async leaveGroup(conversationId: ConversationId): Promise<void> {
    await this.request(`/api/v1/groups/${conversationId}/leave`, { method: 'POST' });
  }

  async disbandGroup(conversationId: ConversationId): Promise<void> {
    await this.request(`/api/v1/groups/${conversationId}`, { method: 'DELETE' });
  }

  async transferGroup(conversationId: ConversationId, userId: UserId): Promise<Conversation> {
    return this.request(`/api/v1/groups/${conversationId}/transfer`, {
      method: 'POST',
      body: JSON.stringify({ user_id: userId }),
    });
  }

  async setGroupMute(conversationId: ConversationId, muted: boolean): Promise<void> {
    await this.request(`/api/v1/groups/${conversationId}/mute`, {
      method: 'POST',
      body: JSON.stringify({ muted }),
    });
  }

  async groupAnnouncements(conversationId: ConversationId): Promise<GroupAnnouncement[]> {
    return this.request(`/api/v1/groups/${conversationId}/announcements`);
  }

  async groupFiles(conversationId: ConversationId): Promise<GroupFileItem[]> {
    return this.request(`/api/v1/groups/${conversationId}/files`);
  }

  async createGroupAnnouncement(
    conversationId: ConversationId,
    content: string,
  ): Promise<GroupAnnouncement> {
    return this.request(`/api/v1/groups/${conversationId}/announcements`, {
      method: 'POST',
      body: JSON.stringify({ content }),
    });
  }

  async readGroupAnnouncement(announcementId: string): Promise<void> {
    await this.request(`/api/v1/group-announcements/${announcementId}/read`, { method: 'POST' });
  }

  async groupJoinRequests(conversationId: ConversationId): Promise<GroupJoinRequest[]> {
    return this.request(`/api/v1/groups/${conversationId}/join-requests`);
  }

  async decideGroupJoinRequest(requestId: string, accept: boolean): Promise<GroupJoinRequest> {
    return this.request(`/api/v1/group-join-requests/${requestId}`, {
      method: 'PATCH',
      body: JSON.stringify({ accept }),
    });
  }

  async groupPolls(conversationId: ConversationId): Promise<GroupPoll[]> {
    return this.request(`/api/v1/groups/${conversationId}/polls`);
  }

  async createGroupPoll(
    conversationId: ConversationId,
    input: {
      question: string;
      options: string[];
      multiple_choice: boolean;
      closes_at: string | null;
    },
  ): Promise<GroupPoll> {
    return this.request(`/api/v1/groups/${conversationId}/polls`, {
      method: 'POST',
      body: JSON.stringify(input),
    });
  }

  async voteGroupPoll(pollId: string, optionIds: string[]): Promise<GroupPoll> {
    return this.request(`/api/v1/polls/${pollId}/vote`, {
      method: 'POST',
      body: JSON.stringify({ option_ids: optionIds }),
    });
  }

  async messages(
    conversationId: ConversationId,
    before?: number,
    limit = 50,
  ): Promise<Page<Message>> {
    const query = new URLSearchParams({ limit: String(limit) });
    if (before !== undefined) query.set('before', String(before));
    return this.request(`/api/v1/conversations/${conversationId}/messages?${query.toString()}`);
  }

  async sendMessage(
    conversationId: ConversationId,
    clientMessageId: MessageId,
    content: MessageContent,
    replyTo: MessageId | null = null,
    expiresInSeconds: number | null = null,
    mentions: UserId[] = [],
    mentionAll = false,
  ): Promise<MessageAck> {
    return this.request(`/api/v1/conversations/${conversationId}/messages`, {
      method: 'POST',
      body: JSON.stringify({
        client_message_id: clientMessageId,
        content,
        reply_to: replyTo,
        mentions,
        mention_all: mentionAll,
        expires_in_seconds: expiresInSeconds,
      }),
    });
  }

  async messageDetails(messageId: MessageId): Promise<MessageDetails> {
    return this.request(`/api/v1/messages/${messageId}`);
  }

  async translateMessage(
    messageId: MessageId,
    targetLanguage: string,
  ): Promise<TranslateMessageResponse> {
    return this.request(`/api/v1/messages/${messageId}/translate`, {
      method: 'POST',
      body: JSON.stringify({ target_language: targetLanguage }),
    });
  }

  async transcribeMessage(messageId: MessageId): Promise<TranscribeMessageResponse> {
    return this.request(`/api/v1/messages/${messageId}/transcribe`, { method: 'POST' });
  }

  async recallMessage(messageId: MessageId): Promise<Message> {
    return this.request(`/api/v1/messages/${messageId}/recall`, { method: 'POST' });
  }

  async reactToMessage(
    messageId: MessageId,
    emoji: string,
    active: boolean,
  ): Promise<MessageReaction[]> {
    return this.request(`/api/v1/messages/${messageId}/reaction`, {
      method: 'POST',
      body: JSON.stringify({ emoji, active }),
    });
  }

  async favoriteMessage(messageId: MessageId, favorite: boolean): Promise<void> {
    await this.request(`/api/v1/messages/${messageId}/favorite`, {
      method: 'POST',
      body: JSON.stringify({ favorite }),
    });
  }

  async favoriteMessages(): Promise<Message[]> {
    return this.request('/api/v1/messages/favorites');
  }

  async scheduleMessage(input: {
    conversation_id: ConversationId;
    client_message_id: MessageId;
    content: MessageContent;
    reply_to: MessageId | null;
    mentions: UserId[];
    mention_all: boolean;
    scheduled_for: string;
    expires_in_seconds: number | null;
  }): Promise<ScheduledMessageResponse> {
    return this.request('/api/v1/scheduled-messages', {
      method: 'POST',
      body: JSON.stringify(input),
    });
  }

  async scheduledMessages(): Promise<ScheduledMessageInfo[]> {
    return this.request('/api/v1/scheduled-messages');
  }

  async cancelScheduledMessage(scheduleId: string): Promise<void> {
    await this.request(`/api/v1/scheduled-messages/${scheduleId}`, { method: 'DELETE' });
  }

  async forwardMessages(
    messageIds: MessageId[],
    targetConversationId: ConversationId,
    mode: 'individually' | 'merged' = 'individually',
  ): Promise<Message[]> {
    return this.request('/api/v1/messages/forward', {
      method: 'POST',
      body: JSON.stringify({
        message_ids: messageIds,
        target_conversation_id: targetConversationId,
        mode,
      }),
    });
  }

  async markRead(conversationId: ConversationId, throughSequence: number): Promise<void> {
    await this.request(`/api/v1/conversations/${conversationId}/read`, {
      method: 'POST',
      body: JSON.stringify({ through_sequence: throughSequence }),
    });
  }

  async updateConversationSettings(
    conversationId: ConversationId,
    settings: {
      pinned?: boolean;
      muted?: boolean;
      hidden?: boolean;
      manually_unread?: boolean;
      draft?: string;
      label?: string | null;
    },
  ): Promise<void> {
    await this.request(`/api/v1/conversations/${conversationId}/settings`, {
      method: 'PATCH',
      body: JSON.stringify(settings),
    });
  }

  async markAllRead(): Promise<void> {
    await this.request('/api/v1/conversations/read-all', { method: 'POST' });
  }

  async upload(
    file: File,
    onProgress: (progress: number) => void = () => undefined,
    signal?: AbortSignal,
  ): Promise<{ attachment: Attachment; downloadUrl: string }> {
    if (signal?.aborted) throw new DOMException('Upload cancelled', 'AbortError');
    onProgress(0);
    if (IS_TAURI) {
      onProgress(5);
      const bytes = new Uint8Array(await file.arrayBuffer());
      const response = await invokeRemoteRaw('remote_upload', bytes, {
        'x-iamrust-file-name': encodeURIComponent(file.name),
        'x-iamrust-mime-type': file.type || 'application/octet-stream',
      });
      const completed = unwrapRemote<{
        attachment: Attachment;
        download_url: string;
      }>(response);
      if (signal?.aborted) throw new DOMException('Upload cancelled', 'AbortError');
      onProgress(100);
      return {
        attachment: completed.attachment,
        downloadUrl: completed.download_url,
      };
    }
    const sha256 = await digestSha256(file);
    const authorization = await this.request<{
      attachment_id: string;
      storage_key: string;
      upload_url: string;
      required_headers: Array<[string, string]>;
    }>('/api/v1/uploads/authorize', {
      method: 'POST',
      body: JSON.stringify({
        file_name: file.name,
        mime_type: file.type || 'application/octet-stream',
        byte_size: file.size,
        sha256,
      }),
    });
    await uploadWithProgress(
      authorization.upload_url,
      file,
      Object.fromEntries(authorization.required_headers),
      onProgress,
      signal,
    );

    const completed = await this.request<{
      attachment: Attachment;
      download_url: string;
    }>('/api/v1/uploads/complete', {
      method: 'POST',
      body: JSON.stringify({ attachment_id: authorization.attachment_id }),
    });
    return {
      attachment: completed.attachment,
      downloadUrl: completed.download_url,
    };
  }

  async stickers(): Promise<Sticker[]> {
    return this.request('/api/v1/me/stickers');
  }

  async createSticker(
    attachmentId: string,
    name: string,
    shortcut: string | null = null,
  ): Promise<Sticker> {
    return this.request('/api/v1/me/stickers', {
      method: 'POST',
      body: JSON.stringify({ attachment_id: attachmentId, name, shortcut }),
    });
  }

  async deleteSticker(stickerId: string): Promise<void> {
    await this.request(`/api/v1/me/stickers/${stickerId}`, { method: 'DELETE' });
  }

  async attachmentDownloadUrl(attachmentId: string): Promise<string> {
    const result = await this.request<{ download_url: string }>(
      `/api/v1/attachments/${attachmentId}/download`,
    );
    return result.download_url;
  }

  async downloadAttachment(
    attachment: Attachment,
    directory: string,
    onProgress: (progress: number) => void = () => undefined,
  ): Promise<DownloadResult> {
    if (IS_TAURI) {
      const unlisten = await listen<{
        attachmentId: string;
        received: number;
        total: number;
        percent: number;
      }>('download-progress', (event) => {
        if (event.payload.attachmentId === attachment.id) onProgress(event.payload.percent);
      });
      try {
        return unwrapRemote<DownloadResult>(
          await invokeRemote('remote_download_attachment', {
            attachmentId: attachment.id,
            directory: directory.trim() || null,
          }),
        );
      } finally {
        unlisten();
      }
    }
    const url = /^(blob:|data:|https?:\/\/)/u.test(attachment.storage_key)
      ? attachment.storage_key
      : await this.attachmentDownloadUrl(attachment.id);
    const blob = await downloadBlob(url, onProgress);
    const objectUrl = URL.createObjectURL(blob);
    const link = document.createElement('a');
    link.href = objectUrl;
    link.download = attachment.file_name;
    link.rel = 'noreferrer noopener';
    link.click();
    window.setTimeout(() => URL.revokeObjectURL(objectUrl), 1_000);
    return {
      path: null,
      file_name: attachment.file_name,
      byte_size: blob.size,
    };
  }

  async revealDownload(path: string, directory: string): Promise<void> {
    if (!IS_TAURI) return;
    await invoke('reveal_download', { path, directory: directory.trim() || null });
  }

  async sync(after: number, limit = 200): Promise<SyncResponse> {
    return this.request(`/api/v1/sync?after=${after}&limit=${limit}`);
  }

  async websocketTicket(): Promise<string> {
    const result = await this.request<{ ticket: string }>('/api/v1/ws-ticket', {
      method: 'POST',
    });
    return result.ticket;
  }

  private setSession(session: WireSessionResponse): SessionResponse {
    this.accessToken = session.access_token;
    this.refreshToken = session.refresh_token;
    saveRefreshToken(session.refresh_token);
    return publicSession(session);
  }

  private clearSession(): void {
    this.accessToken = null;
    this.refreshToken = null;
    clearRefreshToken();
  }

  private refresh(): Promise<SessionResponse> {
    if (this.refreshPromise) return this.refreshPromise;
    const token = this.refreshToken;
    if (!token) return Promise.reject(new ApiClientError(401));
    this.refreshPromise = this.request<WireSessionResponse>(
      '/api/v1/auth/refresh',
      {
        method: 'POST',
        body: JSON.stringify({ refresh_token: token }),
      },
      false,
    )
      .then((session) => this.setSession(session))
      .finally(() => {
        this.refreshPromise = null;
      });
    return this.refreshPromise;
  }

  private async request<T = void>(
    path: string,
    init: RequestInit = {},
    allowRefresh = true,
  ): Promise<T> {
    if (IS_TAURI) {
      let body: unknown;
      if (typeof init.body === 'string') body = JSON.parse(init.body) as unknown;
      return unwrapRemote<T>(
        await invokeRemote('remote_request', {
          method: init.method ?? 'GET',
          path,
          body,
        }),
      );
    }
    const headers = new Headers(init.headers);
    headers.set('Accept', 'application/json');
    if (init.body) headers.set('Content-Type', 'application/json');
    if (this.accessToken) headers.set('Authorization', `Bearer ${this.accessToken}`);

    let response: Response;
    try {
      response = await fetch(`${API_BASE}${path}`, { ...init, headers });
    } catch {
      throw new ApiClientError(0, {
        code: 'offline',
        message_key: 'network_unavailable',
        retryable: true,
      });
    }

    if (response.status === 401 && allowRefresh && this.refreshToken) {
      await this.refresh();
      return this.request<T>(path, init, false);
    }
    if (!response.ok) {
      const body = (await response.json().catch(() => ({}))) as ApiErrorBody;
      throw new ApiClientError(response.status, body);
    }
    if (response.status === 204) return undefined as T;
    return response.json() as Promise<T>;
  }
}

export const api = new ApiClient();
export const apiBaseUrl = API_BASE;

async function digestSha256(file: File): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', await file.arrayBuffer());
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, '0')).join('');
}

function uploadWithProgress(
  url: string,
  file: File,
  headers: Record<string, string>,
  onProgress: (progress: number) => void,
  signal?: AbortSignal,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open('PUT', url);
    Object.entries(headers).forEach(([name, value]) => request.setRequestHeader(name, value));
    request.upload.addEventListener('progress', (event) => {
      if (event.lengthComputable)
        onProgress(Math.min(99, Math.round((event.loaded / event.total) * 100)));
    });
    request.addEventListener('load', () => {
      signal?.removeEventListener('abort', abort);
      if (request.status >= 200 && request.status < 300) {
        onProgress(99);
        resolve();
      } else {
        reject(new ApiClientError(request.status, { code: 'upload_failed' }));
      }
    });
    request.addEventListener('error', () => {
      signal?.removeEventListener('abort', abort);
      reject(new ApiClientError(0, { code: 'upload_failed', retryable: true }));
    });
    request.addEventListener('abort', () => {
      signal?.removeEventListener('abort', abort);
      reject(new DOMException('Upload cancelled', 'AbortError'));
    });
    const abort = () => request.abort();
    signal?.addEventListener('abort', abort, { once: true });
    request.send(file);
  });
}

function downloadBlob(url: string, onProgress: (progress: number) => void): Promise<Blob> {
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open('GET', url);
    request.responseType = 'blob';
    request.addEventListener('progress', (event) => {
      if (event.lengthComputable) onProgress(Math.round((event.loaded / event.total) * 100));
    });
    request.addEventListener('load', () => {
      if (request.status >= 200 && request.status < 300 && request.response instanceof Blob) {
        onProgress(100);
        resolve(request.response);
      } else {
        reject(new ApiClientError(request.status, { code: 'download_failed' }));
      }
    });
    request.addEventListener('error', () =>
      reject(new ApiClientError(0, { code: 'download_failed', retryable: true })),
    );
    request.send();
  });
}

function publicSession(session: WireSessionResponse): SessionResponse {
  return {
    access_expires_at: session.access_expires_at,
    refresh_expires_at: session.refresh_expires_at,
    profile: session.profile,
    device_id: session.device_id,
  };
}

async function invokeRemote(command: string, args?: InvokeArgs): Promise<RemoteResponse> {
  try {
    return await invoke<RemoteResponse>(command, args);
  } catch {
    return {
      status: 0,
      body: {
        code: 'desktop_bridge_unavailable',
        message_key: 'desktop_bridge_unavailable',
        retryable: true,
      },
    };
  }
}

async function invokeRemoteRaw(
  command: string,
  bytes: Uint8Array,
  headers: Record<string, string>,
): Promise<RemoteResponse> {
  try {
    return await invoke<RemoteResponse>(command, bytes, { headers });
  } catch {
    return {
      status: 0,
      body: {
        code: 'desktop_bridge_unavailable',
        message_key: 'desktop_bridge_unavailable',
        retryable: true,
      },
    };
  }
}

function unwrapRemote<T = void>(response: RemoteResponse): T {
  if (response.status < 200 || response.status >= 300) {
    throw new ApiClientError(response.status, response.body as ApiErrorBody);
  }
  return response.body as T;
}
