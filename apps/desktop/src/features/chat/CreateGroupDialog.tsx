import * as Dialog from '@radix-ui/react-dialog';
import { Check, LoaderCircle, Search, X } from 'lucide-react';
import { useMemo, useState, type FormEvent } from 'react';

import { Avatar } from '../../components/Avatar';
import { IconButton } from '../../components/IconButton';
import { api } from '../../lib/api';
import type { UserId } from '../../lib/types';
import { useChatStore } from '../../state/chat-store';
import { tr } from '../../lib/i18n';

interface CreateGroupDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function CreateGroupDialog({ open, onOpenChange }: CreateGroupDialogProps) {
  const friends = useChatStore((state) => state.friends);
  const demo = useChatStore((state) => state.demo);
  const upsertConversation = useChatStore((state) => state.upsertConversation);
  const selectConversation = useChatStore((state) => state.selectConversation);
  const setAnnouncement = useChatStore((state) => state.setAnnouncement);
  const [name, setName] = useState('');
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<UserId[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const filtered = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    return friends.filter((friend) =>
      `${friend.nickname} ${friend.username}`.toLocaleLowerCase().includes(normalized),
    );
  }, [friends, query]);

  function toggle(id: UserId) {
    setSelected((current) =>
      current.includes(id) ? current.filter((item) => item !== id) : [...current, id],
    );
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!name.trim() || selected.length === 0 || submitting) return;
    setSubmitting(true);
    try {
      if (demo) {
        setAnnouncement(tr('演示模式已验证群聊创建表单；连接服务端后会正式创建。'));
      } else {
        const conversation = await api.createGroup(name.trim(), selected);
        upsertConversation(conversation);
        selectConversation(conversation.id);
      }
      onOpenChange(false);
      setName('');
      setSelected([]);
    } catch {
      setAnnouncement(tr('群聊创建失败，请检查成员后重试。'));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog-content dialog-content--group">
          <form onSubmit={(event) => void submit(event)}>
            <div className="dialog-title-row">
              <div>
                <Dialog.Title>{tr('创建群聊')}</Dialog.Title>
                <Dialog.Description>{tr('命名群聊并选择至少一位好友。')}</Dialog.Description>
              </div>
              <Dialog.Close asChild>
                <IconButton label={tr('关闭')}>
                  <X size={18} />
                </IconButton>
              </Dialog.Close>
            </div>
            <label className="group-name-field">
              {tr('群名称')}
              <input
                value={name}
                onChange={(event) => setName(event.target.value)}
                maxLength={80}
                required
                autoFocus
              />
            </label>
            <label className="search-box group-search">
              <Search size={17} />
              <span className="sr-only">{tr('筛选好友')}</span>
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={tr('搜索好友')}
              />
            </label>
            <div className="member-picker" role="group" aria-label={tr('选择群成员')}>
              {filtered.map((friend) => {
                const checked = selected.includes(friend.id);
                return (
                  <button
                    type="button"
                    className={checked ? 'is-selected' : ''}
                    key={friend.id}
                    onClick={() => toggle(friend.id)}
                    aria-pressed={checked}
                  >
                    <Avatar
                      name={friend.nickname}
                      src={friend.avatar_url}
                      attachmentId={friend.avatar_attachment_id}
                      size="small"
                    />
                    <span>
                      <strong>{friend.nickname}</strong>
                      <small>@{friend.username}</small>
                    </span>
                    <span className="member-check">{checked ? <Check size={15} /> : null}</span>
                  </button>
                );
              })}
            </div>
            <div className="dialog-actions">
              <span>
                {tr('已选择')} {selected.length} {tr('人')}
              </span>
              <Dialog.Close asChild>
                <button className="ghost-button" type="button">
                  {tr('取消')}
                </button>
              </Dialog.Close>
              <button
                className="primary-button"
                type="submit"
                disabled={!name.trim() || selected.length === 0 || submitting}
              >
                {submitting ? <LoaderCircle className="spin" size={17} /> : null} {tr('创建')}
              </button>
            </div>
          </form>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
