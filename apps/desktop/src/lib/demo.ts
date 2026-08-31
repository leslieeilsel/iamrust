import type { BootstrapResponse, Message, UserProfile } from './types';

const now = Date.now();

export const demoMe: UserProfile = {
  id: '0199a000-0000-7000-8000-000000000001',
  username: 'ferris',
  nickname: 'Ferris',
  avatar_url: null,
  avatar_attachment_id: null,
  signature: 'Fearless concurrency, friendly conversations.',
  gender: null,
  birthday: null,
  region: 'Shanghai',
  presence: 'online',
  last_seen_at: new Date(now).toISOString(),
};

export const demoFriends: UserProfile[] = [
  {
    id: '0199a000-0000-7000-8000-000000000002',
    username: 'luna',
    nickname: 'Luna',
    avatar_url: null,
    avatar_attachment_id: null,
    signature: '正在学习 Rust 的所有权。',
    gender: null,
    birthday: null,
    region: 'Hangzhou',
    presence: 'online',
    last_seen_at: new Date(now - 60_000).toISOString(),
  },
  {
    id: '0199a000-0000-7000-8000-000000000003',
    username: 'atlas',
    nickname: 'Atlas',
    avatar_url: null,
    avatar_attachment_id: null,
    signature: '把复杂的事情做简单。',
    gender: null,
    birthday: null,
    region: null,
    presence: 'away',
    last_seen_at: new Date(now - 25 * 60_000).toISOString(),
  },
  {
    id: '0199a000-0000-7000-8000-000000000004',
    username: 'mika',
    nickname: 'Mika',
    avatar_url: null,
    avatar_attachment_id: null,
    signature: '愿代码与咖啡常在。',
    gender: null,
    birthday: null,
    region: 'Chengdu',
    presence: 'offline',
    last_seen_at: new Date(now - 3 * 60 * 60_000).toISOString(),
  },
];

export const demoBootstrap: BootstrapResponse = {
  profile: demoMe,
  profile_privacy: {
    gender_visibility: 'friends',
    birthday_visibility: 'friends',
    region_visibility: 'friends',
    presence_visibility: 'friends',
    read_receipts_enabled: true,
  },
  friends: demoFriends,
  friend_settings: [],
  friend_requests: [],
  friend_request_profiles: [],
  conversations: [
    {
      id: '0199b000-0000-7000-8000-000000000001',
      kind: { kind: 'direct', peer_user_id: demoFriends[0]!.id },
      name: '',
      avatar_url: null,
      avatar_attachment_id: null,
      members: {},
      muted: false,
      pinned: true,
      created_at: new Date(now - 12 * 86_400_000).toISOString(),
      updated_at: new Date(now - 4 * 60_000).toISOString(),
    },
    {
      id: '0199b000-0000-7000-8000-000000000002',
      kind: { kind: 'group', group_id: '0199c000-0000-7000-8000-000000000001' },
      name: 'Rustaceans',
      avatar_url: null,
      avatar_attachment_id: null,
      members: Object.fromEntries(
        [demoMe, ...demoFriends].map((profile, index) => [
          profile.id,
          {
            user_id: profile.id,
            role: index === 0 ? 'owner' : 'member',
            nickname: null,
            muted_until: null,
            joined_at: new Date(now - 8 * 86_400_000).toISOString(),
          },
        ]),
      ),
      muted: false,
      pinned: false,
      created_at: new Date(now - 8 * 86_400_000).toISOString(),
      updated_at: new Date(now - 47 * 60_000).toISOString(),
    },
    {
      id: '0199b000-0000-7000-8000-000000000003',
      kind: { kind: 'direct', peer_user_id: demoFriends[1]!.id },
      name: '',
      avatar_url: null,
      avatar_attachment_id: null,
      members: {},
      muted: true,
      pinned: false,
      created_at: new Date(now - 3 * 86_400_000).toISOString(),
      updated_at: new Date(now - 2 * 60 * 60_000).toISOString(),
    },
  ],
  conversation_states: [],
  cursor: 6,
  server_features: { demo: true },
};

function textMessage(
  id: string,
  conversationId: string,
  senderId: string,
  text: string,
  minutesAgo: number,
  sequence: number,
): Message {
  const createdAt = new Date(now - minutesAgo * 60_000).toISOString();
  return {
    id,
    client_message_id: id,
    conversation_id: conversationId,
    sender_id: senderId,
    sequence,
    content: { type: 'text', data: { text } },
    status: 'read',
    reply_to: null,
    mentions: [],
    mention_all: false,
    created_at: createdAt,
    server_created_at: createdAt,
    edited_at: null,
  };
}

export const demoMessages: Record<string, Message[]> = {
  '0199b000-0000-7000-8000-000000000001': [
    textMessage(
      '0199d000-0000-7000-8000-000000000001',
      '0199b000-0000-7000-8000-000000000001',
      demoFriends[0]!.id,
      '新的桌面版终于能跑起来了。',
      18,
      1,
    ),
    textMessage(
      '0199d000-0000-7000-8000-000000000002',
      '0199b000-0000-7000-8000-000000000001',
      demoMe.id,
      '嗯，先把最核心的聊天体验打磨好。',
      12,
      2,
    ),
    textMessage(
      '0199d000-0000-7000-8000-000000000003',
      '0199b000-0000-7000-8000-000000000001',
      demoFriends[0]!.id,
      '没问题，我来试试消息、草稿和离线恢复。',
      4,
      3,
    ),
  ],
  '0199b000-0000-7000-8000-000000000002': [
    textMessage(
      '0199d000-0000-7000-8000-000000000004',
      '0199b000-0000-7000-8000-000000000002',
      demoFriends[1]!.id,
      '今晚讨论一下同步游标的边界情况？',
      52,
      1,
    ),
    textMessage(
      '0199d000-0000-7000-8000-000000000005',
      '0199b000-0000-7000-8000-000000000002',
      demoMe.id,
      '可以，重复事件和乱序也一起覆盖。',
      47,
      2,
    ),
  ],
  '0199b000-0000-7000-8000-000000000003': [
    textMessage(
      '0199d000-0000-7000-8000-000000000006',
      '0199b000-0000-7000-8000-000000000003',
      demoFriends[1]!.id,
      '我把界面规范整理好了。',
      120,
      1,
    ),
  ],
};
