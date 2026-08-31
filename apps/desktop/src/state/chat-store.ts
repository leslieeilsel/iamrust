import { create } from 'zustand';
import { createJSONStorage, persist } from 'zustand/middleware';

import { demoBootstrap, demoMessages } from '../lib/demo';
import type {
  AppSection,
  AppSettings,
  BootstrapResponse,
  ConnectionState,
  Conversation,
  ConversationId,
  ConversationMeta,
  ConversationState,
  FriendRequest,
  FriendSettings,
  Message,
  MessageId,
  ProfilePrivacySettings,
  SyncEvent,
  UserId,
  UserProfile,
} from '../lib/types';

const defaultSettings: AppSettings = {
  theme: 'system',
  compactMode: false,
  fontScale: 1,
  language: 'zh-CN',
  sendShortcut: 'enter',
  notifications: true,
  notificationSound: true,
  notificationPreview: true,
  privacyMode: false,
  crashReporting: true,
  doNotDisturbEnabled: false,
  doNotDisturbStart: '22:00',
  doNotDisturbEnd: '08:00',
  closeBehavior: 'tray',
  keepCacheOnLogout: true,
  autostart: false,
  globalShortcutEnabled: false,
  downloadDirectory: '',
  localDatabaseEncryption: false,
};

const emptyMeta = (): ConversationMeta => ({
  unread: 0,
  lastMessage: null,
  draft: '',
  hidden: false,
  manuallyUnread: false,
  lastReadSequence: 0,
  label: null,
});

interface ChatState {
  auth: 'restoring' | 'authenticated' | 'unauthenticated';
  demo: boolean;
  me: UserProfile | null;
  profilePrivacy: ProfilePrivacySettings;
  friends: UserProfile[];
  friendRequests: FriendRequest[];
  friendRequestProfiles: UserProfile[];
  friendSettings: Record<UserId, FriendSettings>;
  conversations: Conversation[];
  messages: Record<ConversationId, Message[]>;
  typingUsers: Record<ConversationId, Record<UserId, number>>;
  meta: Record<ConversationId, ConversationMeta>;
  selectedConversationId: ConversationId | null;
  jumpTargetMessageId: MessageId | null;
  section: AppSection;
  searchQuery: string;
  connection: ConnectionState;
  cursor: number;
  seenEvents: string[];
  settings: AppSettings;
  announcement: string;
  setAuth: (value: ChatState['auth']) => void;
  setBootstrap: (value: BootstrapResponse, demo?: boolean) => void;
  useDemo: () => void;
  clearAccount: (keepCache?: boolean) => void;
  setSection: (section: AppSection) => void;
  selectConversation: (id: ConversationId | null) => void;
  openMessage: (conversationId: ConversationId, messageId: MessageId) => void;
  clearJumpTarget: () => void;
  setSearchQuery: (query: string) => void;
  setConnection: (connection: ConnectionState) => void;
  setMessages: (id: ConversationId, messages: Message[], prepend?: boolean) => void;
  addPendingMessage: (message: Message) => void;
  resolveMessage: (clientId: MessageId, server: Partial<Message>) => void;
  failMessage: (clientId: MessageId) => void;
  removeMessage: (clientId: MessageId) => void;
  setDraft: (id: ConversationId, value: string) => void;
  markRead: (id: ConversationId, throughSequence?: number) => void;
  markAllRead: () => void;
  markUnread: (id: ConversationId) => void;
  togglePin: (id: ConversationId) => void;
  toggleMute: (id: ConversationId) => void;
  hideConversation: (id: ConversationId) => void;
  upsertConversation: (conversation: Conversation) => void;
  removeConversation: (id: ConversationId) => void;
  setFriends: (friends: UserProfile[]) => void;
  setFriendRequests: (requests: FriendRequest[]) => void;
  updateFriendSettings: (settings: FriendSettings) => void;
  removeFriend: (id: UserId) => void;
  updateProfile: (profile: UserProfile) => void;
  updateProfilePrivacy: (privacy: ProfilePrivacySettings) => void;
  setTyping: (
    conversationId: ConversationId,
    userId: UserId,
    active: boolean,
    expiresAt?: string,
  ) => void;
  applyEvent: (event: SyncEvent) => void;
  updateSettings: (settings: Partial<AppSettings>) => void;
  setAnnouncement: (text: string) => void;
}

const defaultProfilePrivacy: ProfilePrivacySettings = {
  gender_visibility: 'friends',
  birthday_visibility: 'friends',
  region_visibility: 'friends',
  presence_visibility: 'friends',
  read_receipts_enabled: true,
};

function mergeMeta(
  conversations: Conversation[],
  messages: Record<ConversationId, Message[]>,
  previous: Record<ConversationId, ConversationMeta>,
  conversationStates: ConversationState[] = [],
): Record<ConversationId, ConversationMeta> {
  const result = { ...previous };
  const serverState = new Map(
    conversationStates.map((state) => [state.conversation_id, state] as const),
  );
  conversations.forEach((conversation) => {
    const list = messages[conversation.id] ?? [];
    const remote = serverState.get(conversation.id);
    const localDraft = result[conversation.id]?.draft ?? '';
    result[conversation.id] = {
      ...emptyMeta(),
      ...result[conversation.id],
      ...(remote
        ? {
            unread: Math.max(remote.unread_count, remote.manually_unread ? 1 : 0),
            draft: remote.draft || localDraft,
            hidden: remote.hidden,
            manuallyUnread: remote.manually_unread,
            lastReadSequence: remote.last_read_sequence,
            label: remote.label,
          }
        : {}),
      lastMessage: list.at(-1) ?? result[conversation.id]?.lastMessage ?? null,
    };
  });
  return result;
}

function uniqueMessages(current: Message[], incoming: Message[]): Message[] {
  const byClientId = new Map(current.map((message) => [message.client_message_id, message]));
  incoming.forEach((message) => byClientId.set(message.client_message_id, message));
  return Array.from(byClientId.values()).sort((a, b) => {
    const aSequence = a.sequence ?? Number.MAX_SAFE_INTEGER;
    const bSequence = b.sequence ?? Number.MAX_SAFE_INTEGER;
    return aSequence - bSequence || a.created_at.localeCompare(b.created_at);
  });
}

export const useChatStore = create<ChatState>()(
  persist(
    (set, get) => ({
      auth: 'restoring',
      demo: false,
      me: null,
      profilePrivacy: defaultProfilePrivacy,
      friends: [],
      friendRequests: [],
      friendRequestProfiles: [],
      friendSettings: {},
      conversations: [],
      messages: {},
      typingUsers: {},
      meta: {},
      selectedConversationId: null,
      jumpTargetMessageId: null,
      section: 'conversations',
      searchQuery: '',
      connection: 'offline',
      cursor: 0,
      seenEvents: [],
      settings: defaultSettings,
      announcement: '',

      setAuth: (auth) => set({ auth }),
      setBootstrap: (value, demo = false) =>
        set((state) => {
          const conversations = value.conversations.map((conversation) =>
            conversationForUser(conversation, value.profile.id),
          );
          return {
            auth: 'authenticated',
            demo,
            me: value.profile,
            profilePrivacy: value.profile_privacy ?? defaultProfilePrivacy,
            friends: value.friends,
            friendRequests: value.friend_requests,
            friendRequestProfiles: value.friend_request_profiles ?? [],
            friendSettings: Object.fromEntries(
              (value.friend_settings ?? []).map((settings) => [settings.user_id, settings]),
            ),
            conversations,
            typingUsers: {},
            meta: mergeMeta(
              conversations,
              state.messages,
              state.meta,
              value.conversation_states ?? [],
            ),
            selectedConversationId: state.selectedConversationId ?? conversations.at(0)?.id ?? null,
            jumpTargetMessageId: null,
            cursor: value.cursor,
            connection: demo ? 'online' : state.connection,
          };
        }),
      useDemo: () =>
        set((state) => ({
          auth: 'authenticated',
          demo: true,
          me: demoBootstrap.profile,
          profilePrivacy: demoBootstrap.profile_privacy,
          friends: demoBootstrap.friends,
          friendRequests: demoBootstrap.friend_requests,
          friendRequestProfiles: demoBootstrap.friend_request_profiles,
          friendSettings: Object.fromEntries(
            demoBootstrap.friend_settings.map((settings) => [settings.user_id, settings]),
          ),
          conversations: demoBootstrap.conversations,
          messages: demoMessages,
          typingUsers: {},
          meta: mergeMeta(demoBootstrap.conversations, demoMessages, state.meta),
          selectedConversationId: demoBootstrap.conversations[0]?.id ?? null,
          jumpTargetMessageId: null,
          cursor: demoBootstrap.cursor,
          connection: 'online',
        })),
      clearAccount: (keepCache = true) =>
        set((state) => ({
          auth: 'unauthenticated',
          demo: false,
          me: null,
          profilePrivacy: defaultProfilePrivacy,
          friends: [],
          friendRequests: [],
          friendRequestProfiles: [],
          friendSettings: {},
          conversations: [],
          messages: keepCache ? state.messages : {},
          typingUsers: {},
          meta: keepCache ? state.meta : {},
          selectedConversationId: null,
          jumpTargetMessageId: null,
          connection: 'offline',
          cursor: 0,
          seenEvents: [],
        })),
      setSection: (section) => set({ section }),
      selectConversation: (selectedConversationId) => {
        set(() => {
          if (!selectedConversationId) {
            return { selectedConversationId, jumpTargetMessageId: null };
          }
          return {
            selectedConversationId,
            jumpTargetMessageId: null,
            section: 'conversations',
          };
        });
      },
      openMessage: (selectedConversationId, jumpTargetMessageId) =>
        set({ selectedConversationId, jumpTargetMessageId, section: 'conversations' }),
      clearJumpTarget: () => set({ jumpTargetMessageId: null }),
      setSearchQuery: (searchQuery) => set({ searchQuery }),
      setConnection: (connection) => set({ connection }),
      setMessages: (id, incoming, prepend = false) =>
        set((state) => {
          const next = prepend
            ? uniqueMessages(incoming, state.messages[id] ?? [])
            : uniqueMessages(state.messages[id] ?? [], incoming);
          return {
            messages: { ...state.messages, [id]: next },
            meta: {
              ...state.meta,
              [id]: { ...(state.meta[id] ?? emptyMeta()), lastMessage: next.at(-1) ?? null },
            },
          };
        }),
      addPendingMessage: (message) => get().setMessages(message.conversation_id, [message]),
      resolveMessage: (clientId, server) =>
        set((state) => {
          const messages = Object.fromEntries(
            Object.entries(state.messages).map(([id, list]) => [
              id,
              uniqueMessages(
                [],
                list.map((message) =>
                  message.client_message_id === clientId ? { ...message, ...server } : message,
                ),
              ),
            ]),
          );
          return { messages, meta: mergeMeta(state.conversations, messages, state.meta) };
        }),
      failMessage: (clientId) => get().resolveMessage(clientId, { status: 'failed' }),
      removeMessage: (clientId) =>
        set((state) => {
          const messages = Object.fromEntries(
            Object.entries(state.messages).map(([id, list]) => [
              id,
              list.filter((message) => message.client_message_id !== clientId),
            ]),
          );
          return { messages, meta: mergeMeta(state.conversations, messages, state.meta) };
        }),
      setDraft: (id, draft) =>
        set((state) => ({
          meta: {
            ...state.meta,
            [id]: { ...(state.meta[id] ?? emptyMeta()), draft },
          },
        })),
      markRead: (id, throughSequence) =>
        set((state) => ({
          meta: {
            ...state.meta,
            [id]: {
              ...(state.meta[id] ?? emptyMeta()),
              unread: 0,
              manuallyUnread: false,
              lastReadSequence: throughSequence ?? state.meta[id]?.lastReadSequence ?? 0,
            },
          },
        })),
      markAllRead: () =>
        set((state) => ({
          meta: Object.fromEntries(
            Object.entries(state.meta).map(([id, value]) => [
              id,
              {
                ...value,
                unread: 0,
                manuallyUnread: false,
                lastReadSequence: value.lastMessage?.sequence ?? value.lastReadSequence,
              },
            ]),
          ),
        })),
      markUnread: (id) =>
        set((state) => ({
          meta: {
            ...state.meta,
            [id]: {
              ...(state.meta[id] ?? emptyMeta()),
              unread: Math.max(1, state.meta[id]?.unread ?? 0),
              manuallyUnread: true,
            },
          },
        })),
      togglePin: (id) =>
        set((state) => ({
          conversations: state.conversations.map((item) =>
            item.id === id ? { ...item, pinned: !item.pinned } : item,
          ),
        })),
      toggleMute: (id) =>
        set((state) => ({
          conversations: state.conversations.map((item) =>
            item.id === id ? { ...item, muted: !item.muted } : item,
          ),
        })),
      hideConversation: (id) =>
        set((state) => ({
          meta: {
            ...state.meta,
            [id]: { ...(state.meta[id] ?? emptyMeta()), hidden: true },
          },
          selectedConversationId:
            state.selectedConversationId === id ? null : state.selectedConversationId,
        })),
      upsertConversation: (conversation) =>
        set((state) => {
          const existing = state.conversations.some((item) => item.id === conversation.id);
          const conversations = existing
            ? state.conversations.map((item) => (item.id === conversation.id ? conversation : item))
            : [conversation, ...state.conversations];
          return {
            conversations,
            meta: mergeMeta(conversations, state.messages, state.meta),
          };
        }),
      removeConversation: (id) =>
        set((state) => ({
          conversations: state.conversations.filter((conversation) => conversation.id !== id),
          selectedConversationId:
            state.selectedConversationId === id ? null : state.selectedConversationId,
        })),
      setFriends: (friends) => set({ friends }),
      setFriendRequests: (friendRequests) => set({ friendRequests }),
      updateFriendSettings: (settings) =>
        set((state) => ({
          friendSettings: { ...state.friendSettings, [settings.user_id]: settings },
        })),
      removeFriend: (id) =>
        set((state) => {
          const friendSettings = { ...state.friendSettings };
          delete friendSettings[id];
          return {
            friends: state.friends.filter((friend) => friend.id !== id),
            friendSettings,
          };
        }),
      updateProfile: (me) => set({ me }),
      updateProfilePrivacy: (profilePrivacy) => set({ profilePrivacy }),
      setTyping: (conversationId, userId, active, expiresAt) => {
        const expires = expiresAt ? Date.parse(expiresAt) : Date.now() + 6_000;
        set((state) => {
          const conversation = { ...(state.typingUsers[conversationId] ?? {}) };
          if (active) conversation[userId] = expires;
          else delete conversation[userId];
          return {
            typingUsers: {
              ...state.typingUsers,
              [conversationId]: conversation,
            },
          };
        });
        if (active) {
          window.setTimeout(
            () =>
              set((state) => {
                const conversation = state.typingUsers[conversationId];
                if (!conversation || (conversation[userId] ?? 0) > Date.now()) return state;
                const nextConversation = { ...conversation };
                delete nextConversation[userId];
                return {
                  typingUsers: {
                    ...state.typingUsers,
                    [conversationId]: nextConversation,
                  },
                };
              }),
            Math.max(0, expires - Date.now()) + 25,
          );
        }
      },
      applyEvent: (event) =>
        set((state) => {
          if (state.seenEvents.includes(event.id) || event.cursor <= state.cursor) return state;
          const seenEvents = [...state.seenEvents.slice(-999), event.id];
          const base = { cursor: event.cursor, seenEvents };
          if (event.kind === 'message_created') {
            const message = event.payload.message as Message | undefined;
            if (!message) return base;
            const messages = {
              ...state.messages,
              [message.conversation_id]: uniqueMessages(
                state.messages[message.conversation_id] ?? [],
                [message],
              ),
            };
            const isIncoming = message.sender_id !== state.me?.id;
            const isOpen = state.selectedConversationId === message.conversation_id;
            const currentMeta = state.meta[message.conversation_id] ?? emptyMeta();
            return {
              ...base,
              messages,
              meta: {
                ...state.meta,
                [message.conversation_id]: {
                  ...currentMeta,
                  lastMessage: message,
                  unread: isIncoming && !isOpen ? Math.min(999, currentMeta.unread + 1) : 0,
                },
              },
            };
          }
          if (event.kind === 'conversation_updated') {
            const incoming = event.payload.conversation as Conversation | undefined;
            if (!incoming) return base;
            const conversation = state.me ? conversationForUser(incoming, state.me.id) : incoming;
            const exists = state.conversations.some((item) => item.id === conversation.id);
            return {
              ...base,
              conversations: exists
                ? state.conversations.map((item) =>
                    item.id === conversation.id ? conversation : item,
                  )
                : [conversation, ...state.conversations],
            };
          }
          if (event.kind === 'presence_updated') {
            const profile = event.payload.profile as UserProfile | undefined;
            const privacy = event.payload.privacy as ProfilePrivacySettings | undefined;
            if (!profile && !privacy) return base;
            return {
              ...base,
              profilePrivacy: privacy ?? state.profilePrivacy,
              me: profile && profile.id === state.me?.id ? profile : state.me,
              friends: profile
                ? state.friends.map((friend) => (friend.id === profile.id ? profile : friend))
                : state.friends,
            };
          }
          return base;
        }),
      updateSettings: (partial) =>
        set((state) => ({ settings: { ...state.settings, ...partial } })),
      setAnnouncement: (announcement) => set({ announcement }),
    }),
    {
      name: 'iamrust-preferences-v1',
      storage: createJSONStorage(() => localStorage),
      partialize: (state) => ({
        settings: state.settings,
        meta: Object.fromEntries(
          Object.entries(state.meta).map(([id, value]) => [
            id,
            { ...emptyMeta(), draft: value.draft, hidden: value.hidden },
          ]),
        ),
      }),
      merge: (persisted, current) => {
        const saved = persisted as Partial<ChatState>;
        return {
          ...current,
          ...saved,
          settings: { ...current.settings, ...saved.settings },
        };
      },
    },
  ),
);

function conversationForUser(conversation: Conversation, userId: UserId): Conversation {
  if (conversation.kind.kind !== 'direct') return conversation;
  const peerUserId = Object.keys(conversation.members).find((memberId) => memberId !== userId);
  if (!peerUserId || conversation.kind.peer_user_id === peerUserId) return conversation;
  return {
    ...conversation,
    kind: { kind: 'direct', peer_user_id: peerUserId },
  };
}

export function conversationName(
  conversation: Conversation,
  friends: UserProfile[],
  friendSettings?: Record<UserId, FriendSettings>,
): string {
  if (conversation.kind.kind === 'group') return conversation.name;
  const peerUserId = conversation.kind.peer_user_id;
  const friend = friends.find((candidate) => candidate.id === peerUserId);
  const unknownContact =
    useChatStore.getState().settings.language === 'en-US' ? 'Unknown contact' : '未知联系人';
  return (
    friendSettings?.[peerUserId]?.remark || friend?.nickname || friend?.username || unknownContact
  );
}

export function conversationAvatarUser(
  conversation: Conversation,
  friends: UserProfile[],
): UserProfile | null {
  if (conversation.kind.kind === 'group') return null;
  const peerUserId = conversation.kind.peer_user_id;
  return friends.find((candidate) => candidate.id === peerUserId) ?? null;
}

export function userById(state: Pick<ChatState, 'me' | 'friends'>, id: UserId): UserProfile | null {
  if (state.me?.id === id) return state.me;
  return state.friends.find((candidate) => candidate.id === id) ?? null;
}
