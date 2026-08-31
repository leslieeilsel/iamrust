import { invoke } from '@tauri-apps/api/core';

import type { BootstrapResponse, Message, MessageContent, MessageId } from './types';

const isTauri = (): boolean => '__TAURI_INTERNALS__' in window;

export interface OutboxItem {
  client_message_id: MessageId;
  conversation_id: string;
  payload_json: string;
  attempt_count: number;
  next_attempt_at: string;
  last_error_code: string | null;
}

export interface OutboxPayload {
  content: MessageContent;
  reply_to: MessageId | null;
  mentions?: string[];
  mention_all?: boolean;
  expires_in_seconds?: number | null;
}

export interface CacheStats {
  database_bytes: number;
  media_bytes: number;
  message_count: number;
  pending_outbox_count: number;
}

export async function cacheBootstrap(value: BootstrapResponse): Promise<void> {
  if (isTauri()) await invoke('cache_bootstrap', { value });
}

export async function loadCachedBootstrap(): Promise<BootstrapResponse | null> {
  return isTauri() ? invoke<BootstrapResponse | null>('load_cached_bootstrap') : null;
}

export async function cacheMessages(messages: Message[]): Promise<void> {
  if (isTauri() && messages.length) await invoke('cache_messages', { messages });
}

export async function loadCachedMessages(conversationId: string): Promise<Message[]> {
  return isTauri() ? invoke<Message[]>('load_cached_messages', { conversationId }) : [];
}

export async function persistDraft(conversationId: string, body: string): Promise<void> {
  if (isTauri()) await invoke('save_draft', { conversationId, body });
}

export async function enqueueOutbox(
  clientMessageId: string,
  conversationId: string,
  payload: OutboxPayload,
): Promise<void> {
  if (isTauri()) {
    await invoke('enqueue_outbox', {
      clientMessageId,
      conversationId,
      payloadJson: JSON.stringify(payload),
    });
  }
}

export async function readyOutbox(): Promise<OutboxItem[]> {
  return isTauri() ? invoke<OutboxItem[]>('ready_outbox') : [];
}

export async function acknowledgeOutbox(clientMessageId: string): Promise<void> {
  if (isTauri()) await invoke('acknowledge_outbox', { clientMessageId });
}

export async function clearLocalAccountCache(): Promise<void> {
  if (isTauri()) await invoke('clear_account_cache');
}

export async function readCacheStats(): Promise<CacheStats> {
  if (isTauri()) return invoke<CacheStats>('cache_stats');
  return {
    database_bytes: new Blob(Object.values(localStorage)).size,
    media_bytes: 0,
    message_count: Object.keys(localStorage).length,
    pending_outbox_count: 0,
  };
}

export async function clearMediaCache(): Promise<void> {
  if (isTauri()) await invoke('clear_media_cache');
}

export async function localCacheEncryptionStatus(): Promise<boolean | null> {
  return isTauri() ? invoke<boolean>('local_cache_encryption_status') : null;
}

export async function setLocalCacheEncryption(enabled: boolean): Promise<boolean> {
  if (!isTauri()) throw new Error('local cache encryption requires the desktop app');
  return invoke<boolean>('set_local_cache_encryption', { enabled });
}

export async function updateTrayUnread(count: number): Promise<void> {
  if (isTauri()) await invoke('set_tray_unread', { count });
}

export async function updateCloseBehavior(closeToTray: boolean): Promise<void> {
  if (isTauri()) await invoke('set_close_to_tray', { enabled: closeToTray });
}
