import * as Dialog from '@radix-ui/react-dialog';
import { Check, LoaderCircle, Search, UserPlus, X } from 'lucide-react';
import { useState, type FormEvent } from 'react';

import { Avatar } from '../../components/Avatar';
import { IconButton } from '../../components/IconButton';
import { api } from '../../lib/api';
import { cacheBootstrap } from '../../lib/local-cache';
import type { UserProfile } from '../../lib/types';
import { useChatStore } from '../../state/chat-store';
import { tr } from '../../lib/i18n';

interface AddFriendDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function AddFriendDialog({ open, onOpenChange }: AddFriendDialogProps) {
  const [username, setUsername] = useState('');
  const [message, setMessage] = useState(tr('你好，希望和你成为好友。'));
  const [result, setResult] = useState<UserProfile | null>(null);
  const [status, setStatus] = useState<
    'idle' | 'searching' | 'sending' | 'sent' | 'none' | 'error'
  >('idle');
  const friends = useChatStore((state) => state.friends);
  const friendRequests = useChatStore((state) => state.friendRequests);
  const demo = useChatStore((state) => state.demo);
  const me = useChatStore((state) => state.me);
  const setBootstrap = useChatStore((state) => state.setBootstrap);
  const relationship = result
    ? friends.some((friend) => friend.id === result.id)
      ? 'friend'
      : friendRequests.some(
            (request) =>
              request.status === 'pending' &&
              ((request.sender_id === me?.id && request.recipient_id === result.id) ||
                (request.recipient_id === me?.id && request.sender_id === result.id)),
          )
        ? 'pending'
        : 'none'
    : 'none';

  async function search(event: FormEvent) {
    event.preventDefault();
    if (!username.trim()) return;
    setStatus('searching');
    setResult(null);
    if (demo) {
      const found = friends.find((friend) => friend.username === username.trim()) ?? null;
      setResult(found);
      setStatus(found ? 'idle' : 'none');
      return;
    }
    try {
      const users = await api.searchUser(username.trim());
      setResult(users[0] ?? null);
      setStatus(users.length ? 'idle' : 'none');
    } catch {
      setStatus('error');
    }
  }

  async function sendRequest() {
    if (!result) return;
    setStatus('sending');
    if (demo) {
      window.setTimeout(() => setStatus('sent'), 350);
      return;
    }
    try {
      await api.sendFriendRequest(result.username, message.trim());
      const bootstrap = await api.bootstrap();
      setBootstrap(bootstrap);
      void cacheBootstrap(bootstrap);
      setStatus('sent');
    } catch {
      setStatus('error');
    }
  }

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog-content dialog-content--wide">
          <div className="dialog-title-row">
            <div>
              <Dialog.Title>{tr('添加好友')}</Dialog.Title>
              <Dialog.Description>{tr('使用完整用户名进行精确搜索。')}</Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <IconButton label={tr('关闭')}>
                <X size={18} />
              </IconButton>
            </Dialog.Close>
          </div>
          <form className="dialog-search" onSubmit={(event) => void search(event)}>
            <label>
              <span className="sr-only">{tr('用户名')}</span>
              <Search size={17} />
              <input
                value={username}
                onChange={(event) => setUsername(event.target.value)}
                placeholder={tr('输入完整用户名')}
                autoFocus
                maxLength={32}
              />
            </label>
            <button className="secondary-button" type="submit" disabled={status === 'searching'}>
              {status === 'searching' ? <LoaderCircle className="spin" size={17} /> : tr('搜索')}
            </button>
          </form>
          {status === 'none' ? <p className="dialog-notice">{tr('没有找到这个用户。')}</p> : null}
          {status === 'error' ? (
            <p className="dialog-notice is-error">{tr('操作失败，请稍后重试。')}</p>
          ) : null}
          {result ? (
            <div className="friend-result">
              <Avatar
                name={result.nickname}
                src={result.avatar_url}
                attachmentId={result.avatar_attachment_id}
                presence={result.presence}
              />
              <div>
                <strong>{result.nickname}</strong>
                <span>@{result.username}</span>
              </div>
              {status === 'sent' || relationship !== 'none' ? (
                <span className="sent-label">
                  <Check size={16} />{' '}
                  {relationship === 'friend'
                    ? tr('已是好友')
                    : relationship === 'pending'
                      ? tr('申请处理中')
                      : tr('已发送')}
                </span>
              ) : (
                <button
                  className="primary-button"
                  type="button"
                  onClick={() => void sendRequest()}
                  disabled={status === 'sending'}
                >
                  <UserPlus size={16} /> {status === 'sending' ? tr('发送中…') : tr('申请好友')}
                </button>
              )}
            </div>
          ) : null}
          {result && status !== 'sent' && relationship === 'none' ? (
            <label className="verification-message">
              {tr('验证消息')}
              <textarea
                value={message}
                onChange={(event) => setMessage(event.target.value)}
                maxLength={120}
                rows={3}
              />
              <small>{Array.from(message).length}/120</small>
            </label>
          ) : null}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
