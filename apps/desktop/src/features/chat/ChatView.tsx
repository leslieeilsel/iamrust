import { useQuery } from '@tanstack/react-query';
import {
  Bell,
  BellOff,
  Info,
  MessageCircle,
  PanelTopOpen,
  Phone,
  Search,
  Video,
} from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';

import { Avatar } from '../../components/Avatar';
import { EmptyState } from '../../components/EmptyState';
import { IconButton } from '../../components/IconButton';
import { api } from '../../lib/api';
import { createId } from '../../lib/id';
import { hasSafeImageHeader, prepareImageForUpload } from '../../lib/image-processing';
import { startCall } from '../../lib/realtime';
import { openConversationWindow } from '../../lib/desktop-plugins';
import {
  acknowledgeOutbox,
  cacheMessages,
  enqueueOutbox,
  loadCachedMessages,
} from '../../lib/local-cache';
import type { Attachment, Message, MessageContent, PendingUpload, Sticker } from '../../lib/types';
import {
  conversationAvatarUser,
  conversationName,
  useChatStore,
  userById,
} from '../../state/chat-store';
import { MessageComposer } from './MessageComposer';
import { MessageTimeline } from './MessageTimeline';
import { GroupDetailsPanel } from './GroupDetailsPanel';
import { ConversationSearchPanel } from './ConversationSearchPanel';
import { currentLanguage, tr } from '../../lib/i18n';

const EMPTY_MESSAGES: Message[] = [];
const EMPTY_TYPING_USERS: Record<string, number> = {};

export function ChatView() {
  const selectedId = useChatStore((state) => state.selectedConversationId);
  const conversation = useChatStore((state) =>
    state.conversations.find((item) => item.id === selectedId),
  );
  const friends = useChatStore((state) => state.friends);
  const friendSettings = useChatStore((state) => state.friendSettings);
  const me = useChatStore((state) => state.me);
  const demo = useChatStore((state) => state.demo);
  const messages = useChatStore((state) =>
    selectedId ? (state.messages[selectedId] ?? EMPTY_MESSAGES) : EMPTY_MESSAGES,
  );
  const typingUsers = useChatStore((state) =>
    selectedId ? (state.typingUsers[selectedId] ?? EMPTY_TYPING_USERS) : EMPTY_TYPING_USERS,
  );
  const setMessages = useChatStore((state) => state.setMessages);
  const addPendingMessage = useChatStore((state) => state.addPendingMessage);
  const resolveMessage = useChatStore((state) => state.resolveMessage);
  const failMessage = useChatStore((state) => state.failMessage);
  const removeMessage = useChatStore((state) => state.removeMessage);
  const markRead = useChatStore((state) => state.markRead);
  const toggleMute = useChatStore((state) => state.toggleMute);
  const setAnnouncement = useChatStore((state) => state.setAnnouncement);
  const jumpTargetMessageId = useChatStore((state) => state.jumpTargetMessageId);
  const clearJumpTarget = useChatStore((state) => state.clearJumpTarget);
  const openMessage = useChatStore((state) => state.openMessage);
  const [nextCursor, setNextCursor] = useState<number | null>(null);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [replyTarget, setReplyTarget] = useState<Message | null>(null);
  const [unreadFromSequence, setUnreadFromSequence] = useState<number | null>(null);
  const readConversation = useRef<string | null>(null);
  const lastReadSequence = useRef(0);

  const query = useQuery({
    queryKey: ['messages', selectedId],
    queryFn: async () => {
      if (!selectedId) throw new Error('conversation is not selected');
      return api.messages(selectedId);
    },
    enabled: Boolean(selectedId) && !demo,
    staleTime: 15_000,
    retry: 2,
  });

  useEffect(() => {
    if (!selectedId || !query.data) return;
    setMessages(selectedId, query.data.items);
    setNextCursor(query.data.next_cursor ? Number(query.data.next_cursor) : null);
  }, [query.data, selectedId, setMessages]);

  useEffect(() => {
    if (!selectedId || demo || messages.length > 0) return;
    void loadCachedMessages(selectedId)
      .then((cached) => setMessages(selectedId, cached))
      .catch(() => undefined);
  }, [demo, messages.length, selectedId, setMessages]);

  useEffect(() => {
    if (!demo && messages.length) void cacheMessages(messages).catch(() => undefined);
  }, [demo, messages]);

  useEffect(() => {
    setNextCursor(null);
    setDetailsOpen(false);
    setSearchOpen(false);
    setReplyTarget(null);
    setUnreadFromSequence(null);
    readConversation.current = selectedId;
    lastReadSequence.current = 0;
  }, [selectedId]); // intentionally reacts to conversation changes only

  useEffect(() => {
    if (!selectedId || readConversation.current !== selectedId) return;
    const latest = messages.at(-1)?.sequence;
    if (latest === null || latest === undefined) return;
    if (lastReadSequence.current === 0) {
      const itemMeta = useChatStore.getState().meta[selectedId];
      if ((itemMeta?.unread ?? 0) > 0) {
        setUnreadFromSequence(Math.max(1, (itemMeta?.lastReadSequence ?? 0) + 1));
      }
    }
    if (latest <= lastReadSequence.current) return;
    lastReadSequence.current = latest;
    markRead(selectedId, latest);
    if (!demo) void api.markRead(selectedId, latest).catch(() => undefined);
  }, [demo, markRead, messages, selectedId]);

  const header = useMemo(() => {
    if (!conversation) return null;
    const name = conversationName(conversation, friends, friendSettings);
    const avatarUser = conversationAvatarUser(conversation, friends);
    return { name, avatarUser };
  }, [conversation, friendSettings, friends]);

  const typingLabel = useMemo(() => {
    const state = useChatStore.getState();
    const names = Object.entries(typingUsers)
      .filter(([userId, expiresAt]) => userId !== me?.id && expiresAt > Date.now())
      .map(([userId]) => {
        const groupNickname = conversation?.members[userId]?.nickname;
        return groupNickname ?? userById(state, userId)?.nickname ?? tr('成员');
      });
    if (names.length === 0) return '';
    if (conversation?.kind.kind === 'direct') return tr('正在输入…');
    if (names.length === 1) return tr(`${names[0]} 正在输入…`);
    return tr(`${names.slice(0, 2).join('、')} 等 ${names.length} 人正在输入…`);
  }, [conversation, me?.id, typingUsers]);

  if (!conversation || !selectedId || !header) {
    return (
      <section className="content-pane chat-pane">
        <EmptyState
          icon={<MessageCircle />}
          title={tr('开始一段对话')}
          description={tr('从左侧选择会话，或前往联系人列表发起新聊天。')}
        />
      </section>
    );
  }

  async function loadOlder() {
    if (!selectedId || !nextCursor || demo || loadingOlder) return;
    setLoadingOlder(true);
    try {
      const page = await api.messages(selectedId, nextCursor);
      setMessages(selectedId, page.items, true);
      setNextCursor(page.next_cursor ? Number(page.next_cursor) : null);
    } catch {
      setAnnouncement(tr('更早的消息加载失败。'));
    } finally {
      setLoadingOlder(false);
    }
  }

  async function sendOne(
    content: MessageContent,
    existing?: Message,
    replyTo: string | null = null,
    expiresInSeconds: number | null = null,
    mentions: string[] = [],
    mentionAll = false,
  ): Promise<boolean> {
    if (!selectedId || !me) return false;
    const clientId = existing?.client_message_id ?? createId();
    const pending: Message = existing ?? {
      id: clientId,
      client_message_id: clientId,
      conversation_id: selectedId,
      sender_id: me.id,
      sequence: null,
      content,
      status: 'pending',
      reply_to: replyTo,
      mentions,
      mention_all: mentionAll,
      created_at: new Date().toISOString(),
      server_created_at: null,
      edited_at: null,
    };
    if (existing) resolveMessage(clientId, { status: 'pending' });
    else addPendingMessage(pending);
    if (demo) {
      await new Promise((resolve) => window.setTimeout(resolve, 260));
      resolveMessage(clientId, {
        status: 'sent',
        sequence: Math.max(0, ...messages.map((message) => message.sequence ?? 0)) + 1,
        server_created_at: new Date().toISOString(),
      });
      return true;
    }
    try {
      await enqueueOutbox(clientId, selectedId, {
        content,
        reply_to: pending.reply_to,
        mentions: pending.mentions ?? [],
        mention_all: pending.mention_all ?? false,
        expires_in_seconds: expiresInSeconds,
      });
      const ack = await api.sendMessage(
        selectedId,
        clientId,
        content,
        pending.reply_to,
        expiresInSeconds,
        pending.mentions ?? [],
        pending.mention_all ?? false,
      );
      resolveMessage(clientId, {
        id: ack.message_id,
        status: 'sent',
        sequence: ack.sequence,
        server_created_at: ack.server_time,
      });
      await acknowledgeOutbox(clientId);
      return true;
    } catch {
      failMessage(clientId);
      return false;
    }
  }

  async function send(
    text: string,
    mentions: string[],
    mentionAll: boolean,
    sendOriginalImages: boolean,
    files: PendingUpload[],
    expiresInSeconds: number | null,
    onUploadProgress: (
      localId: string,
      progress: number,
      status: PendingUpload['status'],
      error: string | null,
    ) => void,
    signal: AbortSignal,
  ): Promise<boolean> {
    const replyTo = replyTarget?.id ?? null;
    for (const pending of files) {
      let attachment: Attachment = {
        id: createId(),
        kind: pending.file.type.startsWith('image/') ? 'image' : 'file',
        file_name: pending.file.name,
        mime_type: pending.file.type || 'application/octet-stream',
        byte_size: pending.file.size,
        sha256: null,
        storage_key: pending.previewUrl ?? pending.file.name,
        thumbnail_key: pending.previewUrl,
      };
      try {
        if (signal.aborted) throw new DOMException('Upload cancelled', 'AbortError');
        onUploadProgress(pending.localId, 0, 'uploading', null);
        if (!demo) {
          let uploadFile = pending.file;
          if (attachment.kind === 'image') {
            if (!(await hasSafeImageHeader(pending.file))) {
              throw new Error('invalid image signature');
            }
            uploadFile = await prepareImageForUpload(pending.file, sendOriginalImages);
          }
          const uploaded = await api.upload(
            uploadFile,
            (progress) => onUploadProgress(pending.localId, progress, 'uploading', null),
            signal,
          );
          attachment = uploaded.attachment;
        } else {
          await new Promise((resolve) => window.setTimeout(resolve, 180));
        }
        const content: MessageContent =
          attachment.kind === 'image'
            ? { type: 'image', data: { attachment } }
            : { type: 'file', data: { attachment } };
        await sendOne(content, undefined, replyTo, expiresInSeconds);
        onUploadProgress(pending.localId, 100, 'completed', null);
      } catch (error) {
        const cancelled = error instanceof DOMException && error.name === 'AbortError';
        onUploadProgress(
          pending.localId,
          0,
          'failed',
          cancelled ? tr('传输已取消') : tr('上传失败，可重试'),
        );
        setAnnouncement(
          cancelled
            ? tr('附件传输已取消。')
            : tr(`${pending.file.name} 上传失败，请检查文件格式或网络。`),
        );
        return false;
      }
    }
    if (text) {
      await sendOne(
        { type: 'text', data: { text } },
        undefined,
        replyTo,
        expiresInSeconds,
        mentions,
        mentionAll,
      );
    }
    setReplyTarget(null);
    return true;
  }

  async function schedule(
    text: string,
    mentions: string[],
    mentionAll: boolean,
    scheduledFor: string,
    expiresInSeconds: number | null,
  ): Promise<boolean> {
    if (!selectedId || !text.trim()) return false;
    if (demo) {
      setAnnouncement(
        tr(`已模拟定时发送：${new Date(scheduledFor).toLocaleString(currentLanguage())}`),
      );
      setReplyTarget(null);
      return true;
    }
    try {
      await api.scheduleMessage({
        conversation_id: selectedId,
        client_message_id: createId(),
        content: { type: 'text', data: { text: text.trim() } },
        reply_to: replyTarget?.id ?? null,
        mentions,
        mention_all: mentionAll,
        scheduled_for: scheduledFor,
        expires_in_seconds: expiresInSeconds,
      });
      setReplyTarget(null);
      setAnnouncement(tr('定时消息已创建，可在全局搜索页管理。'));
      return true;
    } catch {
      setAnnouncement(tr('定时消息创建失败，请检查发送时间。'));
      return false;
    }
  }

  async function sendVoice(blob: Blob, durationMs: number, mimeType: string): Promise<boolean> {
    if (!selectedId) return false;
    const extension = mimeType === 'audio/ogg' ? 'ogg' : 'webm';
    const file = new File([blob], `voice-${Date.now()}.${extension}`, { type: mimeType });
    try {
      const attachment: Attachment = demo
        ? {
            id: createId(),
            kind: 'audio',
            file_name: file.name,
            mime_type: mimeType,
            byte_size: file.size,
            sha256: null,
            storage_key: URL.createObjectURL(blob),
            thumbnail_key: null,
          }
        : (await api.upload(file)).attachment;
      const ok = await sendOne({
        type: 'audio',
        data: { attachment, duration_ms: Math.max(1, Math.min(120_000, durationMs)) },
      });
      if (!ok) setAnnouncement(tr('语音发送失败，可以重试。'));
      return ok;
    } catch {
      setAnnouncement(tr('语音上传失败，请检查网络后重试。'));
      return false;
    }
  }

  async function sendSticker(sticker: Sticker): Promise<boolean> {
    const ok = await sendOne({
      type: 'sticker',
      data: { attachment: sticker.attachment, name: sticker.name },
    });
    if (!ok) setAnnouncement(tr('表情发送失败，可以重试。'));
    return ok;
  }

  return (
    <section className="content-pane chat-pane">
      <header className="chat-header">
        <Avatar
          name={header.name}
          src={conversation.avatar_url ?? header.avatarUser?.avatar_url}
          attachmentId={
            conversation.avatar_attachment_id ?? header.avatarUser?.avatar_attachment_id
          }
          group={conversation.kind.kind === 'group'}
          presence={header.avatarUser?.presence}
        />
        <div className="chat-header__identity">
          <h2>{header.name}</h2>
          <p className={typingLabel ? 'is-typing' : ''} aria-live="polite">
            {typingLabel ||
              (conversation.kind.kind === 'group'
                ? tr(`${Object.keys(conversation.members).length || friends.length + 1} 位成员`)
                : header.avatarUser?.signature || `@${header.avatarUser?.username ?? 'unknown'}`)}
          </p>
        </div>
        <div className="chat-header__actions">
          <IconButton
            label={tr('在当前会话中搜索')}
            active={searchOpen}
            onClick={() => {
              setSearchOpen((value) => !value);
              setDetailsOpen(false);
            }}
          >
            <Search size={19} />
          </IconButton>
          <IconButton
            label={tr('语音通话')}
            onClick={() => {
              if (demo) setAnnouncement(tr('演示模式不会发起真实通话。'));
              else if (conversation.kind.kind === 'direct') startCall(conversation.id, false);
              else setAnnouncement(tr('当前版本先支持一对一通话。'));
            }}
          >
            <Phone size={19} />
          </IconButton>
          <IconButton
            label={tr('视频通话')}
            onClick={() => {
              if (demo) setAnnouncement(tr('演示模式不会发起真实通话。'));
              else if (conversation.kind.kind === 'direct') startCall(conversation.id, true);
              else setAnnouncement(tr('当前版本先支持一对一通话。'));
            }}
          >
            <Video size={19} />
          </IconButton>
          <IconButton
            label={tr('会话详情')}
            active={detailsOpen}
            onClick={() => {
              setDetailsOpen((value) => !value);
              setSearchOpen(false);
            }}
          >
            <Info size={19} />
          </IconButton>
          <IconButton
            label={tr('在独立窗口打开')}
            onClick={() =>
              void openConversationWindow(selectedId, header.name)
                .then((opened) => {
                  if (!opened) setAnnouncement(tr('浏览器演示模式不支持独立窗口。'));
                })
                .catch(() => setAnnouncement(tr('无法打开独立聊天窗口。')))
            }
          >
            <PanelTopOpen size={19} />
          </IconButton>
        </div>
      </header>
      <MessageTimeline
        messages={messages}
        unreadFromSequence={unreadFromSequence}
        jumpTargetMessageId={jumpTargetMessageId}
        onJumpTargetHandled={clearJumpTarget}
        hasOlder={nextCursor !== null}
        loadingOlder={loadingOlder || query.isLoading}
        onLoadOlder={() => void loadOlder()}
        onRetry={(message) => void sendOne(message.content, message)}
        onDelete={removeMessage}
        onReply={setReplyTarget}
      />
      <MessageComposer
        conversationId={selectedId}
        reply={replyTarget}
        onCancelReply={() => setReplyTarget(null)}
        onSend={send}
        onSchedule={schedule}
        onVoice={sendVoice}
        onSticker={sendSticker}
      />
      {detailsOpen ? (
        <aside className="conversation-details" aria-label={tr('会话详情')}>
          <div className="details-hero">
            <Avatar
              name={header.name}
              src={conversation.avatar_url ?? header.avatarUser?.avatar_url}
              attachmentId={
                conversation.avatar_attachment_id ?? header.avatarUser?.avatar_attachment_id
              }
              group={conversation.kind.kind === 'group'}
              size="large"
            />
            <strong>{header.name}</strong>
          </div>
          {conversation.kind.kind === 'group' ? (
            <GroupDetailsPanel conversation={conversation} />
          ) : (
            <button
              type="button"
              onClick={() => {
                toggleMute(conversation.id);
                if (!demo) {
                  void api
                    .updateConversationSettings(conversation.id, { muted: !conversation.muted })
                    .catch(() => {
                      toggleMute(conversation.id);
                      setAnnouncement(tr('免打扰设置保存失败。'));
                    });
                }
              }}
            >
              {conversation.muted ? <Bell size={18} /> : <BellOff size={18} />}
              {conversation.muted ? tr('开启消息通知') : tr('消息免打扰')}
            </button>
          )}
        </aside>
      ) : null}
      {searchOpen ? (
        <ConversationSearchPanel
          messages={messages}
          onClose={() => setSearchOpen(false)}
          onSelect={(messageId) => {
            openMessage(selectedId, messageId);
            setSearchOpen(false);
          }}
        />
      ) : null}
    </section>
  );
}
