import {
  Check,
  ChevronDown,
  ChevronRight,
  LoaderCircle,
  Radio,
  Search,
  UserPlus,
  X,
} from 'lucide-react';
import { useMemo, useState } from 'react';

import { Avatar } from '../../components/Avatar';
import { IconButton } from '../../components/IconButton';
import { api } from '../../lib/api';
import { cacheBootstrap } from '../../lib/local-cache';
import type { FriendRequest, UserId } from '../../lib/types';
import { useChatStore } from '../../state/chat-store';
import { tr } from '../../lib/i18n';

interface ContactsListProps {
  selectedId: UserId | null;
  onSelect: (id: UserId) => void;
  onAddFriend: () => void;
}

export function ContactsList({ selectedId, onSelect, onAddFriend }: ContactsListProps) {
  const friends = useChatStore((state) => state.friends);
  const friendRequests = useChatStore((state) => state.friendRequests);
  const friendRequestProfiles = useChatStore((state) => state.friendRequestProfiles);
  const friendSettings = useChatStore((state) => state.friendSettings);
  const me = useChatStore((state) => state.me);
  const demo = useChatStore((state) => state.demo);
  const setBootstrap = useChatStore((state) => state.setBootstrap);
  const [query, setQuery] = useState('');
  const [onlineOnly, setOnlineOnly] = useState(false);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [processingRequest, setProcessingRequest] = useState<string | null>(null);
  const [requestError, setRequestError] = useState('');
  const filtered = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    return friends.filter((friend) => {
      const settings = friendSettings[friend.id];
      return (
        (!onlineOnly || friend.presence === 'online') &&
        [settings?.remark, settings?.group, friend.nickname, friend.username, friend.signature]
          .filter(Boolean)
          .join(' ')
          .toLocaleLowerCase()
          .includes(normalized)
      );
    });
  }, [friendSettings, friends, onlineOnly, query]);
  const groups = useMemo(() => {
    const grouped = new Map<string, typeof filtered>();
    filtered.forEach((friend) => {
      const group = friendSettings[friend.id]?.group || tr('我的好友');
      grouped.set(group, [...(grouped.get(group) ?? []), friend]);
    });
    return [...grouped.entries()].sort(([left], [right]) => left.localeCompare(right));
  }, [filtered, friendSettings]);
  const pending = friendRequests.filter(
    (request) => request.recipient_id === me?.id && request.status === 'pending',
  ).length;
  const profileById = useMemo(
    () => new Map([...friends, ...friendRequestProfiles].map((profile) => [profile.id, profile])),
    [friendRequestProfiles, friends],
  );

  async function decideRequest(request: FriendRequest, decision: 'accept' | 'reject') {
    if (demo || processingRequest) return;
    setProcessingRequest(request.id);
    setRequestError('');
    try {
      await api.decideFriendRequest(request.id, decision);
      const bootstrap = await api.bootstrap();
      setBootstrap(bootstrap);
      void cacheBootstrap(bootstrap);
    } catch {
      setRequestError(tr('好友申请处理失败，请稍后重试。'));
    } finally {
      setProcessingRequest(null);
    }
  }

  return (
    <aside className="list-pane" aria-label={tr('联系人列表')}>
      <header className="list-pane__header">
        <div>
          <h1>{tr('联系人')}</h1>
        </div>
        <span className="add-friend-wrap">
          <IconButton
            label={onlineOnly ? tr('显示全部好友') : tr('只看在线好友')}
            active={onlineOnly}
            onClick={() => setOnlineOnly((value) => !value)}
          >
            <Radio size={18} />
          </IconButton>
          <IconButton label={tr('添加好友')} onClick={onAddFriend}>
            <UserPlus size={19} />
          </IconButton>
          {pending > 0 ? <span className="rail-badge">{pending}</span> : null}
        </span>
      </header>
      <label className="search-box">
        <Search size={17} aria-hidden="true" />
        <span className="sr-only">{tr('筛选联系人')}</span>
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={tr('昵称、用户名或签名')}
        />
      </label>
      {friendRequests.length > 0 ? (
        <section className="friend-requests" aria-labelledby="friend-requests-title">
          <div className="friend-requests__title">
            <strong id="friend-requests-title">{tr('好友申请')}</strong>
            {pending > 0 ? (
              <span>
                {pending} {tr('个待处理')}
              </span>
            ) : null}
          </div>
          <div className="friend-requests__list">
            {friendRequests.map((request) => {
              const incoming = request.recipient_id === me?.id;
              const counterpartId = incoming ? request.sender_id : request.recipient_id;
              const profile = profileById.get(counterpartId);
              const busy = processingRequest === request.id;
              return (
                <article className="friend-request" key={request.id}>
                  <Avatar
                    name={profile?.nickname ?? tr('用户')}
                    src={profile?.avatar_url}
                    attachmentId={profile?.avatar_attachment_id}
                  />
                  <div className="friend-request__body">
                    <strong>{profile?.nickname ?? tr('未知用户')}</strong>
                    <small>
                      {incoming ? tr('收到') : tr('发出')} · {requestStatusLabel(request.status)}
                    </small>
                    {request.message ? <p>{request.message}</p> : null}
                  </div>
                  {incoming && request.status === 'pending' ? (
                    <div className="friend-request__actions">
                      <IconButton
                        label={tr(`接受 ${profile?.nickname ?? '好友'} 的申请`)}
                        disabled={processingRequest !== null}
                        onClick={() => void decideRequest(request, 'accept')}
                      >
                        {busy ? <LoaderCircle className="spin" size={16} /> : <Check size={16} />}
                      </IconButton>
                      <IconButton
                        label={tr(`拒绝 ${profile?.nickname ?? '好友'} 的申请`)}
                        disabled={processingRequest !== null}
                        onClick={() => void decideRequest(request, 'reject')}
                      >
                        <X size={16} />
                      </IconButton>
                    </div>
                  ) : null}
                </article>
              );
            })}
          </div>
          {requestError ? (
            <p className="friend-requests__error" role="alert">
              {requestError}
            </p>
          ) : null}
        </section>
      ) : null}
      <div className="contact-count">
        {filtered.length} {tr('位好友')}
      </div>
      <div className="contact-list" aria-label={tr('好友')}>
        {groups.map(([group, members]) => {
          const hidden = collapsed.has(group);
          return (
            <section className="contact-group" key={group}>
              <button
                className="contact-group__header"
                type="button"
                aria-expanded={!hidden}
                onClick={() =>
                  setCollapsed((current) => {
                    const next = new Set(current);
                    if (next.has(group)) next.delete(group);
                    else next.add(group);
                    return next;
                  })
                }
              >
                {hidden ? <ChevronRight size={15} /> : <ChevronDown size={15} />}
                <span>{group}</span>
                <small>{members.length}</small>
              </button>
              {!hidden
                ? members.map((friend) => {
                    const displayName = friendSettings[friend.id]?.remark || friend.nickname;
                    return (
                      <button
                        type="button"
                        aria-current={selectedId === friend.id ? 'true' : undefined}
                        className={`contact-row ${selectedId === friend.id ? 'is-selected' : ''}`}
                        key={friend.id}
                        onClick={() => onSelect(friend.id)}
                      >
                        <Avatar
                          name={displayName}
                          src={friend.avatar_url}
                          attachmentId={friend.avatar_attachment_id}
                          presence={friend.presence}
                        />
                        <span>
                          <strong>{displayName}</strong>
                          <small>@{friend.username}</small>
                        </span>
                      </button>
                    );
                  })
                : null}
            </section>
          );
        })}
        {filtered.length === 0 ? (
          <div className="inline-empty">
            <p>{tr('没有匹配的联系人')}</p>
          </div>
        ) : null}
      </div>
    </aside>
  );
}

function requestStatusLabel(status: FriendRequest['status']): string {
  switch (status) {
    case 'pending':
      return tr('等待处理');
    case 'accepted':
      return tr('已接受');
    case 'rejected':
      return tr('已拒绝');
    case 'cancelled':
      return tr('已取消');
  }
}
