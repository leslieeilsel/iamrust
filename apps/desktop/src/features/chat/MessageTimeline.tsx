import * as DropdownMenu from '@radix-ui/react-dropdown-menu';
import {
  Check,
  CheckCheck,
  CircleAlert,
  Clock3,
  Copy,
  CornerUpRight,
  Forward,
  Info,
  ListChecks,
  RotateCcw,
  Trash2,
} from 'lucide-react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';

import { Avatar } from '../../components/Avatar';
import { formatFullTime, messageSummary } from '../../lib/format';
import type { Attachment, Message, MessageId, UserProfile } from '../../lib/types';
import { useChatStore, userById } from '../../state/chat-store';
import { MessageContentView } from './MessageContentView';
import { MessageActionsDialog } from './MessageActionsDialog';
import { BatchForwardDialog } from './BatchForwardDialog';
import { currentLanguage, tr } from '../../lib/i18n';

interface MessageTimelineProps {
  messages: Message[];
  unreadFromSequence: number | null;
  jumpTargetMessageId: MessageId | null;
  onJumpTargetHandled: () => void;
  hasOlder: boolean;
  loadingOlder: boolean;
  onLoadOlder: () => void;
  onRetry: (message: Message) => void;
  onDelete: (clientId: MessageId) => void;
  onReply: (message: Message) => void;
}

export function MessageTimeline({
  messages,
  unreadFromSequence,
  jumpTargetMessageId,
  onJumpTargetHandled,
  hasOlder,
  loadingOlder,
  onLoadOlder,
  onRetry,
  onDelete,
  onReply,
}: MessageTimelineProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const me = useChatStore((state) => state.me);
  const friends = useChatStore((state) => state.friends);
  const [atBottom, setAtBottom] = useState(true);
  const [unseen, setUnseen] = useState(0);
  const previousCount = useRef(messages.length);
  const previousFirstId = useRef<MessageId | null>(messages.at(0)?.id ?? null);
  const previousTotalSize = useRef(0);
  const positionedConversation = useRef<string | null>(null);
  const [detailsMessage, setDetailsMessage] = useState<Message | null>(null);
  const [highlighted, setHighlighted] = useState<MessageId | null>(null);
  const [selectedIds, setSelectedIds] = useState<Set<MessageId>>(() => new Set());
  const [batchForwardOpen, setBatchForwardOpen] = useState(false);
  const virtualizer = useVirtualizer({
    count: messages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 82,
    overscan: 8,
    getItemKey: (index) => messages[index]?.id ?? index,
  });
  const dates = useMemo(
    () =>
      Array.from(
        new Map(
          messages.map((message) => {
            const value = message.server_created_at ?? message.created_at;
            return [new Date(value).toDateString(), value] as const;
          }),
        ).entries(),
      ),
    [messages],
  );
  const imageGallery = useMemo(
    () =>
      messages.flatMap((message) =>
        message.content.type === 'image' ? [message.content.data.attachment] : [],
      ),
    [messages],
  );
  const conversationId = messages.at(0)?.conversation_id ?? null;

  useEffect(() => {
    setSelectedIds(new Set());
    setBatchForwardOpen(false);
  }, [conversationId]);

  useEffect(() => {
    if (messages.length > previousCount.current) {
      if (atBottom) virtualizer.scrollToIndex(messages.length - 1, { align: 'end' });
      else setUnseen((value) => value + messages.length - previousCount.current);
    }
    previousCount.current = messages.length;
  }, [atBottom, messages.length, virtualizer]);

  useEffect(() => {
    const conversationId = messages.at(-1)?.conversation_id ?? null;
    if (!conversationId || positionedConversation.current === conversationId) return;
    positionedConversation.current = conversationId;
    const unreadIndex =
      unreadFromSequence === null
        ? -1
        : messages.findIndex(
            (message) => message.sequence !== null && message.sequence >= unreadFromSequence,
          );
    requestAnimationFrame(() =>
      virtualizer.scrollToIndex(unreadIndex >= 0 ? unreadIndex : messages.length - 1, {
        align: unreadIndex >= 0 ? 'center' : 'end',
      }),
    );
  }, [messages, unreadFromSequence, virtualizer]);

  useLayoutEffect(() => {
    const node = parentRef.current;
    const firstId = messages.at(0)?.id ?? null;
    const total = virtualizer.getTotalSize();
    const prepended =
      previousFirstId.current !== null &&
      firstId !== previousFirstId.current &&
      messages.some((message) => message.id === previousFirstId.current);
    if (node && prepended) {
      node.scrollTop += Math.max(0, total - previousTotalSize.current);
    }
    previousFirstId.current = firstId;
    previousTotalSize.current = total;
  }, [messages, virtualizer]);

  function onScroll() {
    const node = parentRef.current;
    if (!node) return;
    const nextAtBottom = node.scrollHeight - node.scrollTop - node.clientHeight < 40;
    setAtBottom(nextAtBottom);
    if (nextAtBottom) setUnseen(0);
    if (node.scrollTop < 80 && hasOlder && !loadingOlder) onLoadOlder();
  }

  function scrollBottom() {
    virtualizer.scrollToIndex(messages.length - 1, { align: 'end', behavior: 'smooth' });
    setAtBottom(true);
    setUnseen(0);
  }

  function jumpTo(messageId: MessageId) {
    const index = messages.findIndex((message) => message.id === messageId);
    if (index < 0) return;
    virtualizer.scrollToIndex(index, { align: 'center', behavior: 'smooth' });
    setHighlighted(messageId);
    window.setTimeout(
      () => setHighlighted((current) => (current === messageId ? null : current)),
      1800,
    );
  }

  useEffect(() => {
    if (!jumpTargetMessageId) return;
    const index = messages.findIndex((message) => message.id === jumpTargetMessageId);
    if (index < 0) return;
    virtualizer.scrollToIndex(index, { align: 'center', behavior: 'smooth' });
    setHighlighted(jumpTargetMessageId);
    onJumpTargetHandled();
    const timer = window.setTimeout(() => setHighlighted(null), 2200);
    return () => window.clearTimeout(timer);
  }, [jumpTargetMessageId, messages, onJumpTargetHandled, virtualizer]);

  function jumpToDate(dateKey: string) {
    const index = messages.findIndex(
      (message) =>
        new Date(message.server_created_at ?? message.created_at).toDateString() === dateKey,
    );
    if (index >= 0) virtualizer.scrollToIndex(index, { align: 'start', behavior: 'smooth' });
  }

  function toggleSelected(messageId: MessageId) {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(messageId)) next.delete(messageId);
      else if (next.size < 100) next.add(messageId);
      return next;
    });
  }

  return (
    <div className="timeline-wrap">
      {selectedIds.size > 0 ? (
        <div className="batch-selection-toolbar" role="toolbar" aria-label={tr('批量消息操作')}>
          <strong>
            {tr('已选择')} {selectedIds.size} {tr('条')}
          </strong>
          <button
            className="secondary-button"
            type="button"
            onClick={() => setSelectedIds(new Set())}
          >
            {tr('取消')}
          </button>
          <button
            className="primary-button"
            type="button"
            onClick={() => setBatchForwardOpen(true)}
          >
            <Forward size={15} /> {tr('转发')}
          </button>
        </div>
      ) : null}
      {dates.length > 1 ? (
        <label className="timeline-date-jump">
          <span className="sr-only">{tr('按日期定位')}</span>
          <select defaultValue="" onChange={(event) => jumpToDate(event.target.value)}>
            <option value="" disabled>
              {tr('定位日期')}
            </option>
            {dates.map(([key, value]) => (
              <option key={key} value={key}>
                {new Intl.DateTimeFormat(currentLanguage(), {
                  year: 'numeric',
                  month: 'short',
                  day: 'numeric',
                }).format(new Date(value))}
              </option>
            ))}
          </select>
        </label>
      ) : null}
      <div
        className="message-timeline"
        ref={parentRef}
        onScroll={onScroll}
        tabIndex={0}
        aria-label={tr('消息记录')}
      >
        <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
          {virtualizer.getVirtualItems().map((item) => {
            const message = messages[item.index];
            if (!message) return null;
            const previous = messages[item.index - 1];
            const sender = userById({ me, friends }, message.sender_id);
            const mine = message.sender_id === me?.id;
            const grouped = isGrouped(previous, message);
            const newDate =
              !previous ||
              new Date(previous.created_at).toDateString() !==
                new Date(message.created_at).toDateString();
            return (
              <div
                key={message.id}
                ref={virtualizer.measureElement}
                data-index={item.index}
                className="virtual-message-row"
                style={{ transform: `translateY(${item.start}px)` }}
              >
                {newDate ? (
                  <div className="date-separator">
                    <span>{formatDate(message.created_at)}</span>
                  </div>
                ) : null}
                {unreadFromSequence !== null && message.sequence === unreadFromSequence ? (
                  <div className="unread-separator" role="separator">
                    <span>{tr('以下为未读消息')}</span>
                  </div>
                ) : null}
                <MessageRow
                  message={message}
                  replyMessage={messages.find((candidate) => candidate.id === message.reply_to)}
                  sender={sender}
                  mine={mine}
                  grouped={grouped}
                  highlighted={highlighted === message.id}
                  selectionMode={selectedIds.size > 0}
                  selected={selectedIds.has(message.id)}
                  imageGallery={imageGallery}
                  onRetry={onRetry}
                  onDelete={onDelete}
                  onReply={onReply}
                  onDetails={setDetailsMessage}
                  onToggleSelect={() => toggleSelected(message.id)}
                  onJump={jumpTo}
                />
              </div>
            );
          })}
        </div>
      </div>
      {!atBottom ? (
        <button className="jump-bottom" type="button" onClick={scrollBottom}>
          {unseen > 0 ? tr(`${unseen} 条新消息`) : tr('回到底部')}
        </button>
      ) : null}
      {loadingOlder ? (
        <div className="history-loader" role="status">
          {tr('正在加载更早的消息…')}
        </div>
      ) : null}
      <MessageActionsDialog
        message={detailsMessage}
        open={detailsMessage !== null}
        onOpenChange={(open) => {
          if (!open) setDetailsMessage(null);
        }}
        onReply={onReply}
      />
      <BatchForwardDialog
        messageIds={Array.from(selectedIds)}
        sourceConversationId={conversationId}
        open={batchForwardOpen}
        onOpenChange={setBatchForwardOpen}
        onComplete={() => setSelectedIds(new Set())}
      />
    </div>
  );
}

function MessageRow({
  message,
  replyMessage,
  sender,
  mine,
  grouped,
  highlighted,
  selectionMode,
  selected,
  imageGallery,
  onRetry,
  onDelete,
  onReply,
  onDetails,
  onToggleSelect,
  onJump,
}: {
  message: Message;
  replyMessage: Message | undefined;
  sender: UserProfile | null;
  mine: boolean;
  grouped: boolean;
  highlighted: boolean;
  selectionMode: boolean;
  selected: boolean;
  imageGallery: Attachment[];
  onRetry: (message: Message) => void;
  onDelete: (clientId: MessageId) => void;
  onReply: (message: Message) => void;
  onDetails: (message: Message) => void;
  onToggleSelect: () => void;
  onJump: (messageId: MessageId) => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  async function copy() {
    if (message.content.type === 'text')
      await navigator.clipboard.writeText(message.content.data.text);
  }
  return (
    <DropdownMenu.Root open={menuOpen} onOpenChange={setMenuOpen}>
      <DropdownMenu.Trigger asChild>
        <article
          className={`message-row ${mine ? 'is-mine' : ''} ${grouped ? 'is-grouped' : ''} ${highlighted ? 'is-highlighted' : ''}`}
          tabIndex={0}
          aria-label={tr(
            `${sender?.nickname ?? (mine ? '我' : '未知用户')}的消息，${formatFullTime(message.server_created_at ?? message.created_at)}`,
          )}
          onContextMenu={(event) => {
            event.preventDefault();
            setMenuOpen(true);
          }}
          onKeyDown={(event) => {
            if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
              event.preventDefault();
              setMenuOpen(true);
            }
          }}
        >
          {selectionMode ? (
            <button
              className={`message-select-toggle ${selected ? 'is-selected' : ''}`}
              type="button"
              aria-label={selected ? tr('取消选择此消息') : tr('选择此消息')}
              aria-pressed={selected}
              onClick={(event) => {
                event.stopPropagation();
                onToggleSelect();
              }}
            >
              {selected ? <Check size={14} /> : null}
            </button>
          ) : null}
          {!mine && !grouped ? (
            <Avatar
              name={sender?.nickname ?? tr('未知')}
              src={sender?.avatar_url}
              attachmentId={sender?.avatar_attachment_id}
              size="small"
            />
          ) : (
            <span className="message-avatar-space" />
          )}
          <div className="message-body">
            {!grouped ? (
              <div className="message-meta">
                {!mine ? <span>{sender?.nickname ?? tr('未知用户')}</span> : null}
                <time dateTime={message.server_created_at ?? message.created_at}>
                  {formatFullTime(message.server_created_at ?? message.created_at)}
                </time>
              </div>
            ) : null}
            {replyMessage ? (
              <button
                className="message-quote"
                type="button"
                onClick={() => onJump(replyMessage.id)}
              >
                {messageSummary(replyMessage.content)}
              </button>
            ) : null}
            <div className={`message-bubble message-bubble--${message.content.type}`}>
              <MessageContentView
                content={message.content}
                imageGallery={imageGallery}
                hasMentions={(message.mentions?.length ?? 0) > 0 || Boolean(message.mention_all)}
              />
            </div>
          </div>
          {mine ? <MessageStatusIcon status={message.status} /> : null}
        </article>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content className="menu-content" sideOffset={4}>
          {message.content.type === 'text' ? (
            <DropdownMenu.Item onSelect={() => void copy()}>
              <Copy size={15} />
              {tr('复制文字')}
            </DropdownMenu.Item>
          ) : null}
          <DropdownMenu.Item onSelect={onToggleSelect}>
            <ListChecks size={15} />
            {selected ? tr('取消选择') : tr('批量选择')}
          </DropdownMenu.Item>
          <DropdownMenu.Item onSelect={() => onReply(message)}>
            <CornerUpRight size={15} />
            {tr('回复')}
          </DropdownMenu.Item>
          {message.status !== 'pending' ? (
            <DropdownMenu.Item onSelect={() => onDetails(message)}>
              <Info size={15} />
              {tr('消息详情')}
            </DropdownMenu.Item>
          ) : null}
          {message.status === 'failed' ? (
            <DropdownMenu.Item onSelect={() => onRetry(message)}>
              <RotateCcw size={15} />
              {tr('重新发送')}
            </DropdownMenu.Item>
          ) : null}
          <DropdownMenu.Item
            className="menu-danger"
            onSelect={() => onDelete(message.client_message_id)}
          >
            <Trash2 size={15} />
            {tr('从本地删除')}
          </DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

function MessageStatusIcon({ status }: { status: Message['status'] }) {
  const map = {
    pending: <Clock3 size={13} />,
    sent: <Check size={13} />,
    delivered: <CheckCheck size={13} />,
    read: <CheckCheck size={13} />,
    failed: <CircleAlert size={14} />,
    recalled: null,
  };
  return (
    <span className={`message-status message-status--${status}`} title={status}>
      {map[status]}
    </span>
  );
}

function isGrouped(previous: Message | undefined, current: Message): boolean {
  if (!previous || previous.sender_id !== current.sender_id) return false;
  return (
    new Date(current.created_at).getTime() - new Date(previous.created_at).getTime() < 5 * 60_000
  );
}

function formatDate(value: string): string {
  const date = new Date(value);
  const today = new Date();
  if (date.toDateString() === today.toDateString()) return tr('今天');
  const yesterday = new Date(today.getTime() - 86_400_000);
  if (date.toDateString() === yesterday.toDateString()) return tr('昨天');
  return new Intl.DateTimeFormat(currentLanguage(), {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
  }).format(date);
}
