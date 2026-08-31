import { beforeEach, describe, expect, it } from 'vitest';

import type { BootstrapResponse, Message, SyncEvent, UserProfile } from '../lib/types';
import { useChatStore } from './chat-store';

const conversationId = '0199b000-0000-7000-8000-000000000111';
const me = '0199a000-0000-7000-8000-000000000111';
const peer = '0199a000-0000-7000-8000-000000000112';

function message(id: string): Message {
  return {
    id,
    client_message_id: id,
    conversation_id: conversationId,
    sender_id: peer,
    sequence: 1,
    content: { type: 'text', data: { text: 'hello' } },
    status: 'sent',
    reply_to: null,
    mentions: [],
    mention_all: false,
    created_at: '2026-01-01T00:00:00Z',
    server_created_at: '2026-01-01T00:00:00Z',
    edited_at: null,
  };
}

function profile(id: string, username: string): UserProfile {
  return {
    id,
    username,
    nickname: username,
    avatar_url: null,
    avatar_attachment_id: null,
    signature: '',
    gender: null,
    birthday: null,
    region: null,
    presence: 'online',
    last_seen_at: null,
  };
}

describe('chat store synchronization', () => {
  beforeEach(() => {
    localStorage.clear();
    useChatStore.setState({
      me: {
        id: me,
        username: 'me',
        nickname: 'Me',
        avatar_url: null,
        avatar_attachment_id: null,
        signature: '',
        gender: null,
        birthday: null,
        region: null,
        presence: 'online',
        last_seen_at: null,
      },
      messages: {},
      friends: [],
      conversations: [],
      meta: {},
      cursor: 0,
      seenEvents: [],
      selectedConversationId: null,
    });
  });

  it('deduplicates events and increments incoming unread once', () => {
    const event: SyncEvent = {
      id: '0199e000-0000-7000-8000-000000000111',
      cursor: 1,
      kind: 'message_created',
      payload: { message: message('0199d000-0000-7000-8000-000000000111') },
      created_at: '2026-01-01T00:00:00Z',
    };
    useChatStore.getState().applyEvent(event);
    useChatStore.getState().applyEvent(event);
    expect(useChatStore.getState().messages[conversationId]).toHaveLength(1);
    expect(useChatStore.getState().meta[conversationId]?.unread).toBe(1);
  });

  it('keeps each conversation draft independent', () => {
    useChatStore.getState().setDraft(conversationId, 'one');
    useChatStore.getState().setDraft('0199b000-0000-7000-8000-000000000222', 'two');
    expect(useChatStore.getState().meta[conversationId]?.draft).toBe('one');
    expect(useChatStore.getState().meta['0199b000-0000-7000-8000-000000000222']?.draft).toBe('two');
  });

  it('collapses an optimistic message and its websocket acknowledgement', () => {
    const clientId = '0199d000-0000-7000-8000-000000000111';
    const pending = {
      ...message(clientId),
      sender_id: me,
      sequence: null,
      status: 'pending' as const,
      server_created_at: null,
    };
    const delivered = {
      ...pending,
      id: '0199d000-0000-7000-8000-000000000222',
      sequence: 1,
      status: 'sent' as const,
      server_created_at: '2026-01-01T00:00:01Z',
    };
    useChatStore.getState().addPendingMessage(pending);
    useChatStore.getState().applyEvent({
      id: '0199e000-0000-7000-8000-000000000222',
      cursor: 1,
      kind: 'message_created',
      payload: { message: delivered },
      created_at: '2026-01-01T00:00:01Z',
    });
    useChatStore.getState().resolveMessage(clientId, delivered);

    expect(useChatStore.getState().messages[conversationId]).toEqual([delivered]);
  });

  it('normalizes a direct conversation peer for the signed-in user', () => {
    const bootstrap: BootstrapResponse = {
      profile: profile(me, 'me'),
      profile_privacy: {
        gender_visibility: 'friends',
        birthday_visibility: 'friends',
        region_visibility: 'friends',
        presence_visibility: 'friends',
        read_receipts_enabled: true,
      },
      friends: [profile(peer, 'peer')],
      friend_settings: [],
      friend_requests: [],
      friend_request_profiles: [],
      conversations: [
        {
          id: conversationId,
          kind: { kind: 'direct', peer_user_id: me },
          name: '',
          avatar_url: null,
          avatar_attachment_id: null,
          members: {
            [me]: { user_id: me, role: 'member', nickname: null, muted_until: null, joined_at: '' },
            [peer]: {
              user_id: peer,
              role: 'member',
              nickname: null,
              muted_until: null,
              joined_at: '',
            },
          },
          muted: false,
          pinned: false,
          created_at: '',
          updated_at: '',
        },
      ],
      conversation_states: [],
      cursor: 0,
      server_features: {},
    };
    useChatStore.getState().setBootstrap(bootstrap);

    expect(useChatStore.getState().conversations[0]?.kind).toEqual({
      kind: 'direct',
      peer_user_id: peer,
    });
  });
});
