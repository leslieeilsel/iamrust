import { Ban, MessageCircle, Trash2, UserRoundX } from 'lucide-react';
import { useState, type FormEvent } from 'react';

import { Avatar } from '../../components/Avatar';
import { ConfirmDialog } from '../../components/ConfirmDialog';
import { EmptyState } from '../../components/EmptyState';
import { api } from '../../lib/api';
import { presenceLabel } from '../../lib/format';
import type { UserId } from '../../lib/types';
import { useChatStore } from '../../state/chat-store';
import { tr } from '../../lib/i18n';

interface ContactProfileProps {
  userId: UserId | null;
}

export function ContactProfile({ userId }: ContactProfileProps) {
  const friend = useChatStore((state) => state.friends.find((item) => item.id === userId));
  const conversations = useChatStore((state) => state.conversations);
  const upsertConversation = useChatStore((state) => state.upsertConversation);
  const selectConversation = useChatStore((state) => state.selectConversation);
  const setSection = useChatStore((state) => state.setSection);
  const demo = useChatStore((state) => state.demo);
  const setAnnouncement = useChatStore((state) => state.setAnnouncement);
  const settings = useChatStore((state) => (userId ? state.friendSettings[userId] : undefined));
  const updateSettings = useChatStore((state) => state.updateFriendSettings);
  const removeFriend = useChatStore((state) => state.removeFriend);
  const [saving, setSaving] = useState(false);
  const [pendingRemoval, setPendingRemoval] = useState<'delete' | 'block' | null>(null);

  if (!friend) {
    return (
      <section className="content-pane">
        <EmptyState
          icon={<UserRoundX />}
          title={tr('选择一位联系人')}
          description={tr('这里会显示资料、在线状态和聊天入口。')}
        />
      </section>
    );
  }

  async function startChat() {
    const existing = conversations.find(
      (item) => item.kind.kind === 'direct' && item.kind.peer_user_id === friend?.id,
    );
    if (existing) {
      selectConversation(existing.id);
      setSection('conversations');
      return;
    }
    if (demo || !friend) {
      setAnnouncement(tr('演示数据暂未创建新的会话。'));
      return;
    }
    try {
      const conversation = await api.createDirect(friend.id);
      upsertConversation(conversation);
      selectConversation(conversation.id);
      setSection('conversations');
    } catch {
      setAnnouncement(tr('无法创建会话，请稍后重试。'));
    }
  }

  async function saveSettings(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!friend || saving) return;
    const form = new FormData(event.currentTarget);
    const next = {
      remark: formText(form, 'remark').trim() || null,
      group: formText(form, 'group').trim() || null,
      share_presence: form.get('share-presence') === 'on',
      allow_files: form.get('allow-files') === 'on',
    };
    if (demo) {
      updateSettings({ user_id: friend.id, ...next });
      setAnnouncement(tr('好友设置已在演示模式中更新。'));
      return;
    }
    setSaving(true);
    try {
      updateSettings(await api.updateFriendSettings(friend.id, next));
      setAnnouncement(tr('好友设置已保存。'));
    } catch {
      setAnnouncement(tr('好友设置保存失败。'));
    } finally {
      setSaving(false);
    }
  }

  async function deleteContact(block: boolean): Promise<boolean> {
    if (!friend) return false;
    try {
      if (!demo) {
        if (block) await api.blockUser(friend.id);
        else await api.deleteFriend(friend.id);
      }
      removeFriend(friend.id);
      setAnnouncement(block ? tr('用户已加入黑名单。') : tr('好友已删除。'));
      return true;
    } catch {
      setAnnouncement(block ? tr('拉黑失败。') : tr('删除好友失败。'));
      return false;
    }
  }

  async function report(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!friend) return;
    const form = new FormData(event.currentTarget);
    try {
      if (!demo) {
        await api.reportUser(friend.id, formText(form, 'reason'), formText(form, 'details'));
      }
      event.currentTarget.reset();
      setAnnouncement(tr('举报已提交，我们会进行审核。'));
    } catch {
      setAnnouncement(tr('举报提交失败。'));
    }
  }

  return (
    <section className="content-pane profile-pane">
      <div className="profile-card">
        <Avatar
          name={friend.nickname}
          src={friend.avatar_url}
          attachmentId={friend.avatar_attachment_id}
          presence={friend.presence}
          size="large"
        />
        <h2>{friend.nickname}</h2>
        <p className="profile-username">@{friend.username}</p>
        <span className="presence-label">
          <span className={`presence-dot presence-dot--${friend.presence}`} />
          {presenceLabel(friend.presence)}
        </span>
        <p className="profile-signature">{friend.signature || tr('这个人还没有写个性签名。')}</p>
        <button
          className="primary-button profile-action"
          type="button"
          onClick={() => void startChat()}
        >
          <MessageCircle size={18} /> {tr('发消息')}
        </button>
        <form
          className="friend-settings-form"
          key={friend.id}
          onSubmit={(event) => void saveSettings(event)}
        >
          <label>
            {tr('备注')}
            <input name="remark" defaultValue={settings?.remark ?? ''} maxLength={48} />
          </label>
          <label>
            {tr('分组')}
            <input
              name="group"
              defaultValue={settings?.group ?? ''}
              maxLength={48}
              placeholder={tr('我的好友')}
            />
          </label>
          <label className="inline-checkbox">
            <input
              name="share-presence"
              type="checkbox"
              defaultChecked={settings?.share_presence ?? true}
            />
            {tr('允许对方查看我的在线状态')}
          </label>
          <label className="inline-checkbox">
            <input
              name="allow-files"
              type="checkbox"
              defaultChecked={settings?.allow_files ?? true}
            />
            {tr('允许对方直接发送文件')}
          </label>
          <button className="secondary-button" type="submit" disabled={saving}>
            {saving ? tr('保存中…') : tr('保存好友设置')}
          </button>
        </form>
        <details className="report-panel">
          <summary>{tr('举报该用户')}</summary>
          <form onSubmit={(event) => void report(event)}>
            <label>
              {tr('原因')}
              <select name="reason" required defaultValue="spam">
                <option value="spam">{tr('垃圾信息')}</option>
                <option value="harassment">{tr('骚扰')}</option>
                <option value="fraud">{tr('疑似欺诈')}</option>
                <option value="other">{tr('其他')}</option>
              </select>
            </label>
            <label>
              {tr('补充说明')}
              <textarea name="details" maxLength={500} rows={3} />
            </label>
            <button className="secondary-button" type="submit">
              {tr('提交举报')}
            </button>
          </form>
        </details>
        <div className="profile-danger-actions">
          <button
            className="secondary-button"
            type="button"
            onClick={() => setPendingRemoval('delete')}
          >
            <Trash2 size={16} /> {tr('删除好友')}
          </button>
          <button
            className="danger-button"
            type="button"
            onClick={() => setPendingRemoval('block')}
          >
            <Ban size={16} /> {tr('拉黑')}
          </button>
        </div>
      </div>
      <ConfirmDialog
        open={pendingRemoval !== null}
        onOpenChange={(open) => !open && setPendingRemoval(null)}
        title={pendingRemoval === 'block' ? tr('拉黑用户') : tr('删除好友')}
        description={
          pendingRemoval === 'block'
            ? tr(`拉黑 ${friend.nickname}？你们将解除好友关系，并无法互相发起联系。`)
            : tr(`删除好友 ${friend.nickname}？历史消息不会被删除。`)
        }
        confirmLabel={pendingRemoval === 'block' ? tr('确认拉黑') : tr('确认删除')}
        danger
        onConfirm={() => deleteContact(pendingRemoval === 'block')}
      />
    </section>
  );
}

function formText(form: FormData, name: string): string {
  const value = form.get(name);
  return typeof value === 'string' ? value : '';
}
