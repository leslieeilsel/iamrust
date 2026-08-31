import { FileText, Search, X } from 'lucide-react';
import { useMemo, useRef, useState } from 'react';

import { Avatar } from '../../components/Avatar';
import { IconButton } from '../../components/IconButton';
import { formatFullTime, messageSummary } from '../../lib/format';
import type { Message, MessageId } from '../../lib/types';
import { useChatStore, userById } from '../../state/chat-store';
import { tr } from '../../lib/i18n';

type SearchType = 'all' | 'text' | 'image' | 'file' | 'audio';

export function ConversationSearchPanel({
  messages,
  onClose,
  onSelect,
}: {
  messages: Message[];
  onClose: () => void;
  onSelect: (messageId: MessageId) => void;
}) {
  const me = useChatStore((state) => state.me);
  const friends = useChatStore((state) => state.friends);
  const [query, setQuery] = useState('');
  const [senderId, setSenderId] = useState('all');
  const [type, setType] = useState<SearchType>('all');
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const input = useRef<HTMLInputElement>(null);

  const senders = useMemo(
    () =>
      Array.from(new Set(messages.map((message) => message.sender_id)))
        .map((id) => userById({ me, friends }, id))
        .filter((profile) => profile !== null),
    [friends, me, messages],
  );

  const results = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    const fromTime = from ? new Date(`${from}T00:00:00`).getTime() : Number.NEGATIVE_INFINITY;
    const toTime = to ? new Date(`${to}T23:59:59.999`).getTime() : Number.POSITIVE_INFINITY;
    return messages
      .filter((message) => senderId === 'all' || message.sender_id === senderId)
      .filter((message) => type === 'all' || message.content.type === type)
      .filter((message) => {
        const time = new Date(message.server_created_at ?? message.created_at).getTime();
        return time >= fromTime && time <= toTime;
      })
      .filter((message) =>
        needle ? messageSummary(message.content).toLocaleLowerCase().includes(needle) : true,
      )
      .reverse();
  }, [from, messages, query, senderId, to, type]);

  return (
    <aside className="conversation-search-panel" aria-label={tr('会话内搜索')}>
      <header>
        <div>
          <strong>{tr('搜索聊天记录')}</strong>
          <small>{tr('仅搜索已同步到本机的消息')}</small>
        </div>
        <IconButton label={tr('关闭会话搜索')} onClick={onClose}>
          <X size={17} />
        </IconButton>
      </header>
      <label className="conversation-search-input">
        <Search size={16} aria-hidden="true" />
        <span className="sr-only">{tr('输入消息关键词')}</span>
        <input
          ref={input}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={tr('消息关键词')}
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
      <div className="conversation-search-filters">
        <label>
          <span>{tr('发送者')}</span>
          <select value={senderId} onChange={(event) => setSenderId(event.target.value)}>
            <option value="all">{tr('全部')}</option>
            {senders.map((sender) => (
              <option key={sender.id} value={sender.id}>
                {sender.nickname}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>{tr('类型')}</span>
          <select value={type} onChange={(event) => setType(event.target.value as SearchType)}>
            <option value="all">{tr('全部')}</option>
            <option value="text">{tr('文字')}</option>
            <option value="image">{tr('图片')}</option>
            <option value="file">{tr('文件')}</option>
            <option value="audio">{tr('语音')}</option>
          </select>
        </label>
        <label>
          <span>{tr('开始日期')}</span>
          <input type="date" value={from} onChange={(event) => setFrom(event.target.value)} />
        </label>
        <label>
          <span>{tr('结束日期')}</span>
          <input type="date" value={to} onChange={(event) => setTo(event.target.value)} />
        </label>
      </div>
      <div className="conversation-search-results" aria-live="polite">
        {results.length ? (
          results.map((message) => {
            const sender = userById({ me, friends }, message.sender_id);
            return (
              <button type="button" key={message.id} onClick={() => onSelect(message.id)}>
                <Avatar
                  name={sender?.nickname ?? '?'}
                  src={sender?.avatar_url}
                  attachmentId={sender?.avatar_attachment_id}
                  size="small"
                />
                <span>
                  <span>
                    <strong>{sender?.nickname ?? tr('未知用户')}</strong>
                    <time dateTime={message.server_created_at ?? message.created_at}>
                      {formatFullTime(message.server_created_at ?? message.created_at)}
                    </time>
                  </span>
                  <small>{highlight(messageSummary(message.content), query.trim())}</small>
                </span>
              </button>
            );
          })
        ) : (
          <div className="conversation-search-empty">
            <FileText size={22} />
            <span>{tr('没有符合条件的消息')}</span>
          </div>
        )}
      </div>
    </aside>
  );
}

function highlight(text: string, query: string) {
  if (!query) return text;
  const index = text.toLocaleLowerCase().indexOf(query.toLocaleLowerCase());
  if (index < 0) return text;
  return (
    <>
      {text.slice(0, index)}
      <mark>{text.slice(index, index + query.length)}</mark>
      {text.slice(index + query.length)}
    </>
  );
}
