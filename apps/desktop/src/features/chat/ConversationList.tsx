import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import { BellOff, CheckCheck, MoreHorizontal, Pin, Plus, Search, UsersRound } from 'lucide-react';
import { useMemo, useState } from 'react';

import { Avatar } from '../../components/Avatar';
import { IconButton } from '../../components/IconButton';
import { api } from '../../lib/api';
import { formatConversationTime, messageSummary } from '../../lib/format';
import { conversationAvatarUser, conversationName, useChatStore } from '../../state/chat-store';
import { tr } from '../../lib/i18n';

interface ConversationListProps {
  onCreateGroup: () => void;
}

export function ConversationList({ onCreateGroup }: ConversationListProps) {
  const conversations = useChatStore((state) => state.conversations);
  const friends = useChatStore((state) => state.friends);
  const friendSettings = useChatStore((state) => state.friendSettings);
  const messages = useChatStore((state) => state.messages);
  const me = useChatStore((state) => state.me);
  const meta = useChatStore((state) => state.meta);
  const selected = useChatStore((state) => state.selectedConversationId);
  const selectConversation = useChatStore((state) => state.selectConversation);
  const togglePin = useChatStore((state) => state.togglePin);
  const toggleMute = useChatStore((state) => state.toggleMute);
  const hideConversation = useChatStore((state) => state.hideConversation);
  const markUnread = useChatStore((state) => state.markUnread);
  const markAllRead = useChatStore((state) => state.markAllRead);
  const demo = useChatStore((state) => state.demo);
  const setAnnouncement = useChatStore((state) => state.setAnnouncement);
  const [query, setQuery] = useLocalSearch();

  const visible = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    return conversations
      .filter((conversation) => !meta[conversation.id]?.hidden)
      .filter((conversation) =>
        conversationName(conversation, friends, friendSettings)
          .toLocaleLowerCase()
          .includes(normalized),
      )
      .sort(
        (a, b) => Number(b.pinned) - Number(a.pinned) || b.updated_at.localeCompare(a.updated_at),
      );
  }, [conversations, friendSettings, friends, meta, query]);

  return (
    <aside className="list-pane" aria-label={tr('会话列表')}>
      <header className="list-pane__header">
        <div>
          <h1>{tr('会话')}</h1>
        </div>
        <IconButton
          label={tr('全部标为已读')}
          onClick={() => {
            markAllRead();
            if (!demo)
              void api.markAllRead().catch(() => setAnnouncement(tr('全部标为已读失败。')));
          }}
        >
          <CheckCheck size={19} />
        </IconButton>
        <IconButton label={tr('创建群聊')} onClick={onCreateGroup}>
          <Plus size={20} />
        </IconButton>
      </header>
      <label className="search-box">
        <Search size={17} aria-hidden="true" />
        <span className="sr-only">{tr('筛选会话')}</span>
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={tr('搜索会话')}
          aria-keyshortcuts="Control+K Meta+K"
        />
        {query ? <kbd>Esc</kbd> : <kbd>⌘K</kbd>}
      </label>
      <div className="conversation-list">
        {visible.length === 0 ? (
          <div className="inline-empty">
            <MessageCircleIcon />
            <p>{tr('没有匹配的会话')}</p>
          </div>
        ) : (
          visible.map((conversation) => {
            const name = conversationName(conversation, friends, friendSettings);
            const avatarUser = conversationAvatarUser(conversation, friends);
            const itemMeta = meta[conversation.id];
            const unreadMention = (messages[conversation.id] ?? [])
              .filter(
                (message) =>
                  message.sender_id !== me?.id &&
                  (message.sequence ?? 0) > (itemMeta?.lastReadSequence ?? 0) &&
                  (Boolean(message.mention_all) || message.mentions?.includes(me?.id ?? '')),
              )
              .at(-1);
            const summary = itemMeta?.draft
              ? tr(`草稿：${itemMeta.draft}`)
              : itemMeta?.lastMessage
                ? messageSummary(itemMeta.lastMessage.content)
                : tr('还没有消息');
            const time = itemMeta?.lastMessage?.server_created_at ?? conversation.updated_at;
            return (
              <DropdownMenu.Root key={conversation.id}>
                <div className="conversation-row-shell">
                  <button
                    type="button"
                    aria-current={selected === conversation.id ? 'true' : undefined}
                    className={`conversation-row ${selected === conversation.id ? 'is-selected' : ''}`}
                    onClick={() => selectConversation(conversation.id)}
                  >
                    <Avatar
                      name={name}
                      src={conversation.avatar_url ?? avatarUser?.avatar_url}
                      attachmentId={
                        conversation.avatar_attachment_id ?? avatarUser?.avatar_attachment_id
                      }
                      group={conversation.kind.kind === 'group'}
                      presence={avatarUser?.presence}
                    />
                    <span className="conversation-row__body">
                      <span className="conversation-row__top">
                        <strong>{name}</strong>
                        <time dateTime={time}>{formatConversationTime(time)}</time>
                      </span>
                      <span className="conversation-row__bottom">
                        <span className={itemMeta?.draft ? 'draft-summary' : ''}>
                          {!itemMeta?.draft && unreadMention ? (
                            <b className="mention-indicator">
                              {unreadMention.mention_all ? tr('@所有人') : tr('@我')}
                            </b>
                          ) : null}
                          {summary}
                        </span>
                        <span className="row-indicators">
                          {conversation.muted ? (
                            <BellOff size={13} aria-label={tr('已免打扰')} />
                          ) : null}
                          {conversation.pinned ? <Pin size={13} aria-label={tr('已置顶')} /> : null}
                          {itemMeta && itemMeta.unread > 0 ? (
                            <span
                              className="unread-badge"
                              aria-label={tr(`${itemMeta.unread} 条未读`)}
                            >
                              {itemMeta.unread > 99 ? '99+' : itemMeta.unread}
                            </span>
                          ) : null}
                        </span>
                      </span>
                    </span>
                  </button>
                  <DropdownMenu.Trigger asChild>
                    <button
                      className="row-menu-button"
                      type="button"
                      aria-label={`${name} · ${tr('会话菜单')}`}
                    >
                      <MoreHorizontal size={16} aria-hidden="true" />
                    </button>
                  </DropdownMenu.Trigger>
                </div>
                <DropdownMenu.Portal>
                  <DropdownMenu.Content className="menu-content" sideOffset={4}>
                    <DropdownMenu.Item
                      onSelect={() => {
                        togglePin(conversation.id);
                        if (!demo) {
                          void api
                            .updateConversationSettings(conversation.id, {
                              pinned: !conversation.pinned,
                            })
                            .catch(() => {
                              togglePin(conversation.id);
                              setAnnouncement(tr('置顶设置保存失败。'));
                            });
                        }
                      }}
                    >
                      {conversation.pinned ? tr('取消置顶') : tr('置顶会话')}
                    </DropdownMenu.Item>
                    <DropdownMenu.Item
                      onSelect={() => {
                        toggleMute(conversation.id);
                        if (!demo) {
                          void api
                            .updateConversationSettings(conversation.id, {
                              muted: !conversation.muted,
                            })
                            .catch(() => {
                              toggleMute(conversation.id);
                              setAnnouncement(tr('免打扰设置保存失败。'));
                            });
                        }
                      }}
                    >
                      {conversation.muted ? tr('开启通知') : tr('消息免打扰')}
                    </DropdownMenu.Item>
                    <DropdownMenu.Item
                      onSelect={() => {
                        markUnread(conversation.id);
                        if (!demo) {
                          void api
                            .updateConversationSettings(conversation.id, {
                              manually_unread: true,
                            })
                            .catch(() => setAnnouncement(tr('标记未读失败。')));
                        }
                      }}
                    >
                      {tr('标记未读')}
                    </DropdownMenu.Item>
                    <DropdownMenu.Separator />
                    <DropdownMenu.Item
                      className="menu-danger"
                      onSelect={() => {
                        hideConversation(conversation.id);
                        if (!demo) {
                          void api
                            .updateConversationSettings(conversation.id, { hidden: true })
                            .catch(() => setAnnouncement(tr('隐藏会话失败，可重新同步恢复。')));
                        }
                      }}
                    >
                      {tr('隐藏会话')}
                    </DropdownMenu.Item>
                  </DropdownMenu.Content>
                </DropdownMenu.Portal>
              </DropdownMenu.Root>
            );
          })
        )}
      </div>
    </aside>
  );
}

function useLocalSearch(): [string, (value: string) => void] {
  return useState('');
}

function MessageCircleIcon() {
  return <UsersRound size={24} aria-hidden="true" />;
}
