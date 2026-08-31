import {
  Bookmark,
  CalendarClock,
  FileText,
  MessageCircle,
  Search,
  Trash2,
  UserRound,
  UsersRound,
} from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';

import { Avatar } from '../../components/Avatar';
import { EmptyState } from '../../components/EmptyState';
import { messageSummary } from '../../lib/format';
import { api } from '../../lib/api';
import type { Message, ScheduledMessageInfo } from '../../lib/types';
import { conversationName, useChatStore, userById } from '../../state/chat-store';
import { currentLanguage, tr } from '../../lib/i18n';

export function GlobalSearch() {
  const query = useChatStore((state) => state.searchQuery);
  const setQuery = useChatStore((state) => state.setSearchQuery);
  const friends = useChatStore((state) => state.friends);
  const friendSettings = useChatStore((state) => state.friendSettings);
  const me = useChatStore((state) => state.me);
  const conversations = useChatStore((state) => state.conversations);
  const messages = useChatStore((state) => state.messages);
  const selectConversation = useChatStore((state) => state.selectConversation);
  const openMessage = useChatStore((state) => state.openMessage);
  const demo = useChatStore((state) => state.demo);
  const setAnnouncement = useChatStore((state) => state.setAnnouncement);
  const input = useRef<HTMLInputElement>(null);
  const [tab, setTab] = useState<'search' | 'favorites' | 'scheduled'>('search');
  const [favorites, setFavorites] = useState<Message[]>([]);
  const [scheduled, setScheduled] = useState<ScheduledMessageInfo[]>([]);
  const [loadingSaved, setLoadingSaved] = useState(false);
  const [savedError, setSavedError] = useState('');

  const results = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    if (!normalized) return { contacts: [], conversations: [], messages: [] };
    return {
      contacts: friends.filter((friend) =>
        `${friendSettings[friend.id]?.remark ?? ''} ${friend.nickname} ${friend.username} ${friend.signature}`
          .toLocaleLowerCase()
          .includes(normalized),
      ),
      conversations: conversations.filter((conversation) =>
        conversationName(conversation, friends, friendSettings)
          .toLocaleLowerCase()
          .includes(normalized),
      ),
      messages: Object.values(messages)
        .flat()
        .filter(
          (message) =>
            message.content.type === 'text' &&
            message.content.data.text.toLocaleLowerCase().includes(normalized),
        )
        .slice(0, 40),
    };
  }, [conversations, friendSettings, friends, messages, query]);
  const total = results.contacts.length + results.conversations.length + results.messages.length;

  useEffect(() => {
    if (tab === 'search') return;
    if (demo) {
      const local = Object.values(messages).flat();
      setFavorites(tab === 'favorites' ? local.slice(0, 2) : []);
      setScheduled([]);
      return;
    }
    let active = true;
    setLoadingSaved(true);
    setSavedError('');
    const request = tab === 'favorites' ? api.favoriteMessages() : api.scheduledMessages();
    void request
      .then((items) => {
        if (!active) return;
        if (tab === 'favorites') setFavorites(items as Message[]);
        else setScheduled(items as ScheduledMessageInfo[]);
      })
      .catch(() => {
        if (active) setSavedError(tr('内容加载失败，请稍后重试。'));
      })
      .finally(() => {
        if (active) setLoadingSaved(false);
      });
    return () => {
      active = false;
    };
  }, [demo, messages, tab]);

  async function cancelScheduled(scheduleId: string) {
    try {
      if (!demo) await api.cancelScheduledMessage(scheduleId);
      setScheduled((items) => items.filter((item) => item.schedule_id !== scheduleId));
      setAnnouncement(tr('定时消息已取消。'));
    } catch {
      setAnnouncement(tr('取消定时消息失败。'));
    }
  }

  return (
    <section className="global-search-pane">
      <header>
        <h1>{tr('查找与收藏')}</h1>
        <div className="search-tabs" role="tablist" aria-label={tr('查找内容')}>
          <button
            type="button"
            role="tab"
            aria-selected={tab === 'search'}
            onClick={() => setTab('search')}
          >
            <Search size={16} /> {tr('搜索')}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === 'favorites'}
            onClick={() => setTab('favorites')}
          >
            <Bookmark size={16} /> {tr('收藏')}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === 'scheduled'}
            onClick={() => setTab('scheduled')}
          >
            <CalendarClock size={16} /> {tr('定时消息')}
          </button>
        </div>
        {tab === 'search' ? (
          <label className="global-search-input">
            <Search size={20} />
            <span className="sr-only">{tr('搜索联系人、会话和消息')}</span>
            <input
              ref={input}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={tr('联系人、会话或消息内容')}
              autoFocus
            />
            {query ? (
              <button
                type="button"
                onClick={() => {
                  setQuery('');
                  input.current?.focus();
                }}
              >
                {tr('清除')}
              </button>
            ) : null}
          </label>
        ) : null}
      </header>
      {tab === 'favorites' ? (
        <SavedMessages
          kind="favorites"
          loading={loadingSaved}
          error={savedError}
          favorites={favorites}
          scheduled={[]}
          conversations={conversations}
          onOpenMessage={openMessage}
          onCancel={() => undefined}
        />
      ) : tab === 'scheduled' ? (
        <SavedMessages
          kind="scheduled"
          loading={loadingSaved}
          error={savedError}
          favorites={[]}
          scheduled={scheduled}
          conversations={conversations}
          onOpenMessage={openMessage}
          onCancel={(id) => void cancelScheduled(id)}
        />
      ) : !query.trim() ? (
        <EmptyState
          icon={<Search />}
          title={tr('输入关键词')}
          description={tr('本地搜索不会把查询内容发送到服务端。')}
        />
      ) : total === 0 ? (
        <EmptyState
          icon={<FileText />}
          title={tr('没有结果')}
          description={tr(`没有找到与“${query}”匹配的内容。`)}
        />
      ) : (
        <div className="search-results" aria-live="polite" onKeyDown={moveSearchFocus}>
          {results.contacts.length ? (
            <SearchGroup
              icon={<UserRound size={17} />}
              title={tr('联系人')}
              count={results.contacts.length}
            >
              {results.contacts.map((friend) => (
                <button
                  type="button"
                  data-search-result
                  key={friend.id}
                  onClick={() => {
                    const direct = conversations.find(
                      (item) => item.kind.kind === 'direct' && item.kind.peer_user_id === friend.id,
                    );
                    if (direct) selectConversation(direct.id);
                  }}
                >
                  <Avatar
                    name={friendSettings[friend.id]?.remark || friend.nickname}
                    src={friend.avatar_url}
                    attachmentId={friend.avatar_attachment_id}
                    size="small"
                    presence={friend.presence}
                  />
                  <span>
                    <strong>{friendSettings[friend.id]?.remark || friend.nickname}</strong>
                    <small>@{friend.username}</small>
                  </span>
                </button>
              ))}
            </SearchGroup>
          ) : null}
          {results.conversations.length ? (
            <SearchGroup
              icon={<UsersRound size={17} />}
              title={tr('会话')}
              count={results.conversations.length}
            >
              {results.conversations.map((conversation) => (
                <button
                  type="button"
                  data-search-result
                  key={conversation.id}
                  onClick={() => selectConversation(conversation.id)}
                >
                  <Avatar
                    name={conversationName(conversation, friends, friendSettings)}
                    src={conversation.avatar_url}
                    attachmentId={conversation.avatar_attachment_id}
                    size="small"
                    group={conversation.kind.kind === 'group'}
                  />
                  <span>
                    <strong>{conversationName(conversation, friends, friendSettings)}</strong>
                    <small>{tr('打开会话')}</small>
                  </span>
                </button>
              ))}
            </SearchGroup>
          ) : null}
          {results.messages.length ? (
            <SearchGroup
              icon={<MessageCircle size={17} />}
              title={tr('消息')}
              count={results.messages.length}
            >
              {results.messages.map((message) => {
                const sender = userById({ me, friends }, message.sender_id);
                const conversation = conversations.find(
                  (item) => item.id === message.conversation_id,
                );
                return (
                  <button
                    type="button"
                    data-search-result
                    key={message.id}
                    onClick={() => openMessage(message.conversation_id, message.id)}
                  >
                    <Avatar
                      name={sender?.nickname ?? '?'}
                      src={sender?.avatar_url}
                      attachmentId={sender?.avatar_attachment_id}
                      size="small"
                    />
                    <span>
                      <strong>
                        {sender?.nickname ?? tr('未知用户')} ·{' '}
                        {conversation
                          ? conversationName(conversation, friends, friendSettings)
                          : tr('会话')}
                      </strong>
                      <small>{messageSummary(message.content)}</small>
                    </span>
                  </button>
                );
              })}
            </SearchGroup>
          ) : null}
        </div>
      )}
    </section>
  );
}

function SavedMessages({
  kind,
  loading,
  error,
  favorites,
  scheduled,
  conversations,
  onOpenMessage,
  onCancel,
}: {
  kind: 'favorites' | 'scheduled';
  loading: boolean;
  error: string;
  favorites: Message[];
  scheduled: ScheduledMessageInfo[];
  conversations: ReturnType<typeof useChatStore.getState>['conversations'];
  onOpenMessage: (conversationId: string, messageId: string) => void;
  onCancel: (scheduleId: string) => void;
}) {
  if (loading) {
    return <div className="saved-state">{tr('正在加载…')}</div>;
  }
  if (error) {
    return <div className="saved-state is-error">{error}</div>;
  }
  if (kind === 'favorites' && favorites.length === 0) {
    return (
      <EmptyState
        icon={<Bookmark />}
        title={tr('还没有收藏')}
        description={tr('在消息详情中点击收藏，稍后可从这里快速找到。')}
      />
    );
  }
  if (kind === 'scheduled' && scheduled.length === 0) {
    return (
      <EmptyState
        icon={<CalendarClock />}
        title={tr('没有待发送消息')}
        description={tr('在消息编辑器的发送选项中创建定时消息。')}
      />
    );
  }
  return (
    <div className="saved-message-list">
      {kind === 'favorites'
        ? favorites.map((message) => (
            <button
              type="button"
              key={message.id}
              onClick={() => onOpenMessage(message.conversation_id, message.id)}
            >
              <Bookmark size={17} />
              <span>
                <strong>{messageSummary(message.content)}</strong>
                <small>
                  {new Date(message.server_created_at ?? message.created_at).toLocaleString(
                    currentLanguage(),
                  )}
                </small>
              </span>
            </button>
          ))
        : scheduled.map((item) => {
            const conversation = conversations.find(
              (candidate) => candidate.id === item.conversation_id,
            );
            return (
              <article key={item.schedule_id}>
                <CalendarClock size={18} />
                <span>
                  <strong>{messageSummary(item.content)}</strong>
                  <small>
                    {conversation?.name || tr('会话')} ·{' '}
                    {new Date(item.scheduled_for).toLocaleString(currentLanguage())}
                  </small>
                </span>
                <button
                  type="button"
                  aria-label={tr('取消定时消息')}
                  onClick={() => onCancel(item.schedule_id)}
                >
                  <Trash2 size={16} />
                </button>
              </article>
            );
          })}
    </div>
  );
}

function moveSearchFocus(event: React.KeyboardEvent<HTMLDivElement>) {
  if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
  const items = Array.from(
    event.currentTarget.querySelectorAll<HTMLButtonElement>('[data-search-result]'),
  );
  if (!items.length) return;
  event.preventDefault();
  const current = items.indexOf(document.activeElement as HTMLButtonElement);
  const direction = event.key === 'ArrowDown' ? 1 : -1;
  const next = current < 0 ? (direction > 0 ? 0 : items.length - 1) : current + direction;
  items[(next + items.length) % items.length]?.focus();
}

function SearchGroup({
  icon,
  title,
  count,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  count: number;
  children: React.ReactNode;
}) {
  return (
    <section className="search-group">
      <h2>
        {icon}
        {title}
        <span>{count}</span>
      </h2>
      <div>{children}</div>
    </section>
  );
}
