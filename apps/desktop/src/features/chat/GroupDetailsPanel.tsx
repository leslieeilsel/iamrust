import {
  Ban,
  Crown,
  Download,
  Files,
  ImageUp,
  Megaphone,
  Plus,
  Search,
  Shield,
  Trash2,
  UserMinus,
  Vote,
} from 'lucide-react';
import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react';

import { Avatar } from '../../components/Avatar';
import { ConfirmDialog } from '../../components/ConfirmDialog';
import { api } from '../../lib/api';
import type {
  Conversation,
  GroupAnnouncement,
  GroupFileItem,
  GroupJoinRequest,
  GroupPoll,
  UserId,
} from '../../lib/types';
import { formatFileSize } from '../../lib/format';
import { useChatStore, userById } from '../../state/chat-store';
import { AvatarCropDialog } from '../settings/AvatarCropDialog';
import { tr } from '../../lib/i18n';

type GroupConfirmAction =
  | { kind: 'remove-member'; memberId: UserId; name: string }
  | { kind: 'transfer'; memberId: UserId; name: string }
  | { kind: 'leave' }
  | { kind: 'disband' };

export function GroupDetailsPanel({ conversation }: { conversation: Conversation }) {
  const me = useChatStore((state) => state.me);
  const friends = useChatStore((state) => state.friends);
  const demo = useChatStore((state) => state.demo);
  const upsertConversation = useChatStore((state) => state.upsertConversation);
  const removeConversation = useChatStore((state) => state.removeConversation);
  const setAnnouncement = useChatStore((state) => state.setAnnouncement);
  const openMessage = useChatStore((state) => state.openMessage);
  const downloadDirectory = useChatStore((state) => state.settings.downloadDirectory);
  const [muteAll, setMuteAll] = useState(false);
  const [announcements, setAnnouncements] = useState<GroupAnnouncement[]>([]);
  const [joinRequests, setJoinRequests] = useState<GroupJoinRequest[]>([]);
  const [polls, setPolls] = useState<GroupPoll[]>([]);
  const [groupFiles, setGroupFiles] = useState<GroupFileItem[]>([]);
  const [downloadingFile, setDownloadingFile] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [avatarFile, setAvatarFile] = useState<File | null>(null);
  const [avatarSaving, setAvatarSaving] = useState(false);
  const [avatarProgress, setAvatarProgress] = useState(0);
  const [groupQuery, setGroupQuery] = useState('');
  const [confirmAction, setConfirmAction] = useState<GroupConfirmAction | null>(null);
  const avatarInput = useRef<HTMLInputElement>(null);

  const membership = me ? conversation.members[me.id] : undefined;
  const canModerate = membership?.role === 'owner' || membership?.role === 'administrator';
  const isOwner = membership?.role === 'owner';
  const availableFriends = useMemo(
    () => friends.filter((friend) => !conversation.members[friend.id]),
    [conversation.members, friends],
  );
  const normalizedGroupQuery = groupQuery.trim().toLocaleLowerCase();
  const visibleMembers = useMemo(
    () =>
      Object.values(conversation.members).filter((member) => {
        if (!normalizedGroupQuery) return true;
        const profile = userById({ me, friends }, member.user_id);
        return `${member.nickname ?? ''} ${profile?.nickname ?? ''} ${profile?.username ?? ''}`
          .toLocaleLowerCase()
          .includes(normalizedGroupQuery);
      }),
    [conversation.members, friends, me, normalizedGroupQuery],
  );
  const visibleGroupFiles = useMemo(
    () =>
      groupFiles.filter((item) =>
        normalizedGroupQuery
          ? item.attachment.file_name.toLocaleLowerCase().includes(normalizedGroupQuery)
          : true,
      ),
    [groupFiles, normalizedGroupQuery],
  );

  useEffect(() => {
    setAnnouncements([]);
    setJoinRequests([]);
    setPolls([]);
    setGroupFiles([]);
    if (demo) return;
    void api
      .groupSettings(conversation.id)
      .then((settings) => setMuteAll(settings.mute_all))
      .catch(() => undefined);
    void api
      .groupAnnouncements(conversation.id)
      .then(setAnnouncements)
      .catch(() => undefined);
    void api
      .groupPolls(conversation.id)
      .then(setPolls)
      .catch(() => undefined);
    void api
      .groupFiles(conversation.id)
      .then(setGroupFiles)
      .catch(() => undefined);
    if (canModerate) {
      void api
        .groupJoinRequests(conversation.id)
        .then(setJoinRequests)
        .catch(() => undefined);
    }
  }, [canModerate, conversation.id, demo]);

  async function mutate(action: () => Promise<Conversation>, failure: string) {
    if (busy) return;
    setBusy(true);
    try {
      upsertConversation(await action());
    } catch {
      setAnnouncement(failure);
    } finally {
      setBusy(false);
    }
  }

  async function saveGroup(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const name = formText(form, 'name').trim();
    if (demo) {
      upsertConversation({ ...conversation, name });
      return;
    }
    await mutate(() => api.updateGroup(conversation.id, { name }), tr('群资料保存失败。'));
  }

  function chooseAvatar(file: File | undefined) {
    if (!file) return;
    if (
      !['image/png', 'image/jpeg', 'image/webp'].includes(file.type) ||
      file.size > 10 * 1024 * 1024
    ) {
      setAnnouncement(tr('群头像仅支持 PNG、JPEG 或 WebP，且不能超过 10 MB。'));
      return;
    }
    setAvatarFile(file);
  }

  async function saveAvatar(blob: Blob) {
    setAvatarSaving(true);
    setAvatarProgress(0);
    try {
      if (demo) {
        upsertConversation({
          ...conversation,
          avatar_url: URL.createObjectURL(blob),
          avatar_attachment_id: null,
        });
      } else {
        const uploaded = await api.upload(
          new File([blob], 'group-avatar.webp', { type: 'image/webp' }),
          setAvatarProgress,
        );
        upsertConversation(
          await api.updateGroup(conversation.id, {
            avatar_url: null,
            avatar_attachment_id: uploaded.attachment.id,
          }),
        );
      }
      setAvatarFile(null);
      setAnnouncement(tr('群头像已更新。'));
    } catch {
      setAnnouncement(tr('群头像上传失败，原头像已保留。'));
    } finally {
      setAvatarSaving(false);
    }
  }

  async function removeAvatar() {
    if (avatarSaving) return;
    if (demo) {
      upsertConversation({
        ...conversation,
        avatar_url: null,
        avatar_attachment_id: null,
      });
      return;
    }
    await mutate(
      () =>
        api.updateGroup(conversation.id, {
          avatar_url: null,
          avatar_attachment_id: null,
        }),
      tr('移除群头像失败。'),
    );
  }

  async function invite(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const memberIds = new FormData(event.currentTarget).getAll('member').map(String);
    if (!memberIds.length) return;
    if (demo) {
      setAnnouncement(tr('演示模式不会邀请新成员。'));
      return;
    }
    await mutate(() => api.addGroupMembers(conversation.id, memberIds), tr('邀请成员失败。'));
    event.currentTarget.reset();
  }

  async function updateMember(
    memberId: UserId,
    input: { nickname?: string; role?: 'member' | 'administrator'; muted_until?: string },
  ) {
    if (demo) {
      setAnnouncement(tr('演示模式不会修改成员权限。'));
      return;
    }
    await mutate(
      () => api.updateGroupMember(conversation.id, memberId, input),
      tr('成员设置保存失败。'),
    );
  }

  async function removeMember(memberId: UserId): Promise<boolean> {
    try {
      if (!demo) await api.removeGroupMember(conversation.id, memberId);
      const members = { ...conversation.members };
      delete members[memberId];
      upsertConversation({ ...conversation, members });
      return true;
    } catch {
      setAnnouncement(tr('移除成员失败。'));
      return false;
    }
  }

  async function leaveOrDisband(disband: boolean): Promise<boolean> {
    try {
      if (!demo) {
        if (disband) await api.disbandGroup(conversation.id);
        else await api.leaveGroup(conversation.id);
      }
      removeConversation(conversation.id);
      return true;
    } catch {
      setAnnouncement(disband ? tr('解散群聊失败。') : tr('退出群聊失败。'));
      return false;
    }
  }

  async function transferOwnership(memberId: UserId): Promise<boolean> {
    if (busy) return false;
    setBusy(true);
    try {
      if (!demo) upsertConversation(await api.transferGroup(conversation.id, memberId));
      setAnnouncement(tr('群主已转让。'));
      return true;
    } catch {
      setAnnouncement(tr('转让群主失败。'));
      return false;
    } finally {
      setBusy(false);
    }
  }

  function runConfirmedAction(): Promise<boolean> {
    if (!confirmAction) return Promise.resolve(false);
    if (confirmAction.kind === 'remove-member') return removeMember(confirmAction.memberId);
    if (confirmAction.kind === 'transfer') return transferOwnership(confirmAction.memberId);
    return leaveOrDisband(confirmAction.kind === 'disband');
  }

  async function createAnnouncement(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const content = formText(new FormData(event.currentTarget), 'announcement').trim();
    if (!content) return;
    if (demo) {
      setAnnouncement(tr('演示模式不会发布群公告。'));
      return;
    }
    try {
      const created = await api.createGroupAnnouncement(conversation.id, content);
      setAnnouncements((current) => [created, ...current]);
      event.currentTarget.reset();
    } catch {
      setAnnouncement(tr('群公告发布失败。'));
    }
  }

  async function createPoll(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const question = formText(form, 'question').trim();
    const options = formText(form, 'options')
      .split('\n')
      .map((option) => option.trim())
      .filter(Boolean);
    if (!question || options.length < 2) {
      setAnnouncement(tr('投票至少需要两个选项。'));
      return;
    }
    if (demo) {
      setAnnouncement(tr('演示模式不会创建投票。'));
      return;
    }
    try {
      const poll = await api.createGroupPoll(conversation.id, {
        question,
        options,
        multiple_choice: form.get('multiple') === 'on',
        closes_at: null,
      });
      setPolls((current) => [poll, ...current]);
      event.currentTarget.reset();
    } catch {
      setAnnouncement(tr('创建投票失败。'));
    }
  }

  async function downloadGroupFile(item: GroupFileItem) {
    if (downloadingFile) return;
    setDownloadingFile(item.attachment.id);
    try {
      await api.downloadAttachment(item.attachment, downloadDirectory);
      setAnnouncement(tr('群文件已保存。'));
    } catch {
      setAnnouncement(tr('群文件下载失败，请检查下载目录或网络。'));
    } finally {
      setDownloadingFile(null);
    }
  }

  return (
    <div className="group-details">
      {canModerate ? (
        <form className="details-form" onSubmit={(event) => void saveGroup(event)}>
          <strong>{tr('群资料')}</strong>
          <div className="group-avatar-editor">
            <Avatar
              name={conversation.name}
              src={conversation.avatar_url}
              attachmentId={conversation.avatar_attachment_id}
              group
              size="large"
            />
            <div>
              <button
                className="secondary-button"
                type="button"
                onClick={() => avatarInput.current?.click()}
              >
                <ImageUp size={15} /> {tr('更换群头像')}
              </button>
              {conversation.avatar_url || conversation.avatar_attachment_id ? (
                <button
                  className="text-button"
                  type="button"
                  disabled={busy || avatarSaving}
                  onClick={() => void removeAvatar()}
                >
                  {tr('移除')}
                </button>
              ) : null}
            </div>
            <input
              ref={avatarInput}
              hidden
              type="file"
              accept="image/png,image/jpeg,image/webp"
              onChange={(event) => {
                chooseAvatar(event.target.files?.[0]);
                event.currentTarget.value = '';
              }}
            />
          </div>
          <input
            name="name"
            defaultValue={conversation.name}
            maxLength={80}
            aria-label={tr('群名称')}
          />
          <button className="secondary-button" type="submit" disabled={busy}>
            {tr('保存群资料')}
          </button>
        </form>
      ) : null}

      <AvatarCropDialog
        file={avatarFile}
        saving={avatarSaving}
        progress={avatarProgress}
        onCancel={() => !avatarSaving && setAvatarFile(null)}
        onSave={saveAvatar}
      />

      <section className="details-section">
        <strong>
          {tr('群成员 ·')} {Object.keys(conversation.members).length}
        </strong>
        <label className="group-content-search">
          <Search size={14} aria-hidden="true" />
          <span className="sr-only">{tr('搜索群成员和群文件')}</span>
          <input
            value={groupQuery}
            onChange={(event) => setGroupQuery(event.target.value)}
            placeholder={tr('搜索成员或群文件')}
          />
        </label>
        <ul className="group-member-list">
          {visibleMembers.map((member) => {
            const profile = userById({ me, friends }, member.user_id);
            return (
              <li key={member.user_id}>
                <span>
                  <strong>{member.nickname || profile?.nickname || shortId(member.user_id)}</strong>
                  <small>
                    {roleName(member.role)}
                    {member.muted_until ? tr(' · 已禁言') : ''}
                  </small>
                </span>
                {isOwner && member.user_id !== me?.id ? (
                  <select
                    aria-label={tr(`设置 ${profile?.nickname ?? '成员'} 的角色`)}
                    value={member.role === 'administrator' ? 'administrator' : 'member'}
                    onChange={(event) =>
                      void updateMember(member.user_id, {
                        role: event.target.value as 'member' | 'administrator',
                      })
                    }
                  >
                    <option value="member">{tr('成员')}</option>
                    <option value="administrator">{tr('管理员')}</option>
                  </select>
                ) : null}
                {canModerate && member.user_id !== me?.id && member.role !== 'owner' ? (
                  <span className="member-actions">
                    <button
                      type="button"
                      title={tr('禁言一小时')}
                      aria-label={tr('禁言一小时')}
                      onClick={() =>
                        void updateMember(member.user_id, {
                          muted_until: new Date(Date.now() + 3_600_000).toISOString(),
                        })
                      }
                    >
                      <Ban size={14} />
                    </button>
                    {isOwner ? (
                      <button
                        type="button"
                        title={tr('转让群主')}
                        aria-label={tr('转让群主')}
                        onClick={() =>
                          setConfirmAction({
                            kind: 'transfer',
                            memberId: member.user_id,
                            name: profile?.nickname ?? tr('该成员'),
                          })
                        }
                      >
                        <Crown size={14} />
                      </button>
                    ) : null}
                    <button
                      type="button"
                      title={tr('移除成员')}
                      aria-label={tr('移除成员')}
                      onClick={() =>
                        setConfirmAction({
                          kind: 'remove-member',
                          memberId: member.user_id,
                          name: profile?.nickname ?? tr('该成员'),
                        })
                      }
                    >
                      <UserMinus size={14} />
                    </button>
                  </span>
                ) : null}
              </li>
            );
          })}
        </ul>
        {membership ? (
          <form
            className="inline-details-form"
            onSubmit={(event) => {
              event.preventDefault();
              const nickname = formText(new FormData(event.currentTarget), 'nickname').trim();
              if (me && nickname) void updateMember(me.id, { nickname });
            }}
          >
            <input
              name="nickname"
              defaultValue={membership.nickname ?? ''}
              maxLength={48}
              placeholder={tr('我的群昵称')}
              aria-label={tr('我的群昵称')}
            />
            <button type="submit">{tr('保存')}</button>
          </form>
        ) : null}
      </section>

      <section className="details-section">
        <strong>
          <Files size={15} /> {tr('群文件 ·')} {groupFiles.length}
        </strong>
        {visibleGroupFiles.length ? (
          <ul className="group-file-list">
            {visibleGroupFiles.map((item) => {
              const sender = userById({ me, friends }, item.sender_id);
              return (
                <li key={item.message_id}>
                  <button
                    className="group-file-name"
                    type="button"
                    title={tr('定位到文件消息')}
                    onClick={() => openMessage(conversation.id, item.message_id)}
                  >
                    <strong>{item.attachment.file_name}</strong>
                    <small>
                      {formatFileSize(item.attachment.byte_size)} ·{' '}
                      {sender?.nickname ?? tr('群成员')}
                    </small>
                  </button>
                  <button
                    type="button"
                    aria-label={tr(`下载 ${item.attachment.file_name}`)}
                    disabled={downloadingFile === item.attachment.id}
                    onClick={() => void downloadGroupFile(item)}
                  >
                    <Download size={15} />
                  </button>
                </li>
              );
            })}
          </ul>
        ) : (
          <small className="details-empty">{tr('暂无群文件')}</small>
        )}
      </section>

      {canModerate && availableFriends.length ? (
        <form className="details-form" onSubmit={(event) => void invite(event)}>
          <strong>
            <Plus size={15} /> {tr('邀请好友')}
          </strong>
          <div className="invite-options">
            {availableFriends.map((friend) => (
              <label key={friend.id}>
                <input type="checkbox" name="member" value={friend.id} /> {friend.nickname}
              </label>
            ))}
          </div>
          <button className="secondary-button" type="submit">
            {tr('邀请选中好友')}
          </button>
        </form>
      ) : null}

      {canModerate ? (
        <label className="inline-checkbox details-toggle">
          <input
            type="checkbox"
            checked={muteAll}
            onChange={(event) => {
              const muted = event.target.checked;
              setMuteAll(muted);
              if (!demo) {
                void api.setGroupMute(conversation.id, muted).catch(() => {
                  setMuteAll(!muted);
                  setAnnouncement(tr('全员禁言设置失败。'));
                });
              }
            }}
          />
          <Shield size={15} /> {tr('全员禁言（群主和管理员除外）')}
        </label>
      ) : null}

      <section className="details-section">
        <strong>
          <Megaphone size={15} /> {tr('群公告')}
        </strong>
        {announcements.map((item) => (
          <article className="group-announcement" key={item.id}>
            <p>{item.content}</p>
            <small>
              {item.read_by.length} {tr('人已读')}
            </small>
            {me && !item.read_by.includes(me.id) ? (
              <button
                type="button"
                onClick={() => {
                  if (demo) return;
                  void api
                    .readGroupAnnouncement(item.id)
                    .then(() =>
                      setAnnouncements((current) =>
                        current.map((candidate) =>
                          candidate.id === item.id
                            ? { ...candidate, read_by: [...candidate.read_by, me.id] }
                            : candidate,
                        ),
                      ),
                    );
                }}
              >
                {tr('标为已读')}
              </button>
            ) : null}
          </article>
        ))}
        {canModerate ? (
          <form className="details-form" onSubmit={(event) => void createAnnouncement(event)}>
            <textarea
              name="announcement"
              maxLength={4000}
              rows={3}
              placeholder={tr('发布群公告')}
            />
            <button className="secondary-button" type="submit">
              {tr('发布公告')}
            </button>
          </form>
        ) : null}
      </section>

      {canModerate && joinRequests.some((item) => item.status === 'pending') ? (
        <section className="details-section">
          <strong>{tr('入群申请')}</strong>
          {joinRequests
            .filter((item) => item.status === 'pending')
            .map((item) => (
              <div className="join-request" key={item.id}>
                <span>
                  {shortId(item.applicant_id)} · {item.message || tr('无验证消息')}
                </span>
                <button type="button" onClick={() => void decideJoin(item.id, true)}>
                  {tr('同意')}
                </button>
                <button type="button" onClick={() => void decideJoin(item.id, false)}>
                  {tr('拒绝')}
                </button>
              </div>
            ))}
        </section>
      ) : null}

      <section className="details-section">
        <strong>
          <Vote size={15} /> {tr('群投票')}
        </strong>
        {polls.map((poll) => (
          <article className="group-poll" key={poll.id}>
            <p>{poll.question}</p>
            {poll.options.map((option) => (
              <button
                type="button"
                key={option.id}
                className={me && option.voter_ids.includes(me.id) ? 'is-selected' : ''}
                onClick={() => void vote(poll, option.id)}
              >
                <span>{option.label}</span>
                <small>
                  {option.voter_ids.length} {tr('票')}
                </small>
              </button>
            ))}
          </article>
        ))}
        <form className="details-form" onSubmit={(event) => void createPoll(event)}>
          <input name="question" maxLength={240} placeholder={tr('投票问题')} />
          <textarea name="options" rows={3} placeholder={tr('每行一个选项\n至少两个选项')} />
          <label className="inline-checkbox">
            <input type="checkbox" name="multiple" /> {tr('允许多选')}
          </label>
          <button className="secondary-button" type="submit">
            {tr('创建投票')}
          </button>
        </form>
      </section>

      <button
        className="danger-button group-exit"
        type="button"
        onClick={() => setConfirmAction({ kind: isOwner ? 'disband' : 'leave' })}
      >
        <Trash2 size={16} /> {isOwner ? tr('解散群聊') : tr('退出群聊')}
      </button>
      <ConfirmDialog
        open={confirmAction !== null}
        onOpenChange={(open) => !open && setConfirmAction(null)}
        title={groupConfirmCopy(confirmAction).title}
        description={groupConfirmCopy(confirmAction).description}
        confirmLabel={groupConfirmCopy(confirmAction).label}
        danger={confirmAction?.kind !== 'transfer'}
        onConfirm={runConfirmedAction}
      />
    </div>
  );

  async function decideJoin(requestId: string, accept: boolean) {
    if (demo) return;
    try {
      const updated = await api.decideGroupJoinRequest(requestId, accept);
      setJoinRequests((current) =>
        current.map((request) => (request.id === requestId ? updated : request)),
      );
      if (accept) {
        const refreshed = await api.bootstrap();
        const group = refreshed.conversations.find((item) => item.id === conversation.id);
        if (group) upsertConversation(group);
      }
    } catch {
      setAnnouncement(tr('入群申请处理失败。'));
    }
  }

  async function vote(poll: GroupPoll, optionId: string) {
    if (demo) return;
    const selected = poll.options
      .filter((option) => me && option.voter_ids.includes(me.id))
      .map((option) => option.id);
    const optionIds = poll.multiple_choice
      ? selected.includes(optionId)
        ? selected.filter((id) => id !== optionId)
        : [...selected, optionId]
      : [optionId];
    if (!optionIds.length) return;
    try {
      const updated = await api.voteGroupPoll(poll.id, optionIds);
      setPolls((current) => current.map((item) => (item.id === poll.id ? updated : item)));
    } catch {
      setAnnouncement(tr('投票失败。'));
    }
  }
}

function groupConfirmCopy(action: GroupConfirmAction | null) {
  switch (action?.kind) {
    case 'remove-member':
      return {
        title: tr('移除群成员'),
        description: tr(`确定将 ${action.name} 移出群聊？`),
        label: tr('确认移除'),
      };
    case 'transfer':
      return {
        title: tr('转让群主'),
        description: tr(`将群主转让给 ${action.name}？你将变为普通成员。`),
        label: tr('确认转让'),
      };
    case 'leave':
      return {
        title: tr('退出群聊'),
        description: tr('确定退出群聊？你将不再收到新消息。'),
        label: tr('确认退出'),
      };
    case 'disband':
      return {
        title: tr('解散群聊'),
        description: tr('确定解散群聊？此操作不可撤销。'),
        label: tr('确认解散'),
      };
    default:
      return { title: tr('确认操作'), description: tr('请确认是否继续。'), label: tr('确认') };
  }
}

function roleName(role: string): string {
  if (role === 'owner') return tr('群主');
  if (role === 'administrator') return tr('管理员');
  return tr('成员');
}

function shortId(id: string): string {
  return `${id.slice(0, 8)}…`;
}

function formText(form: FormData, name: string): string {
  const value = form.get(name);
  return typeof value === 'string' ? value : '';
}
