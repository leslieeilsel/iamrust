import * as Dialog from '@radix-ui/react-dialog';
import {
  AudioLines,
  Bookmark,
  CornerUpRight,
  Info,
  Languages,
  RotateCcw,
  SmilePlus,
  X,
} from 'lucide-react';
import { useEffect, useState } from 'react';

import { IconButton } from '../../components/IconButton';
import { ConfirmDialog } from '../../components/ConfirmDialog';
import { api } from '../../lib/api';
import { messageSummary } from '../../lib/format';
import type {
  Message,
  MessageDetails,
  TranscribeMessageResponse,
  TranslateMessageResponse,
} from '../../lib/types';
import { conversationName, useChatStore } from '../../state/chat-store';
import { currentLanguage, tr } from '../../lib/i18n';

const REACTIONS = ['👍', '❤️', '😂', '🎉', '👀', '🦀'];

export function MessageActionsDialog({
  message,
  open,
  onOpenChange,
  onReply,
}: {
  message: Message | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onReply: (message: Message) => void;
}) {
  const me = useChatStore((state) => state.me);
  const demo = useChatStore((state) => state.demo);
  const conversations = useChatStore((state) => state.conversations);
  const friends = useChatStore((state) => state.friends);
  const friendSettings = useChatStore((state) => state.friendSettings);
  const resolveMessage = useChatStore((state) => state.resolveMessage);
  const setMessages = useChatStore((state) => state.setMessages);
  const setAnnouncement = useChatStore((state) => state.setAnnouncement);
  const [details, setDetails] = useState<MessageDetails | null>(null);
  const [loading, setLoading] = useState(false);
  const [target, setTarget] = useState('');
  const [translation, setTranslation] = useState<TranslateMessageResponse | null>(null);
  const [translating, setTranslating] = useState(false);
  const [transcription, setTranscription] = useState<TranscribeMessageResponse | null>(null);
  const [transcribing, setTranscribing] = useState(false);
  const [recallConfirmOpen, setRecallConfirmOpen] = useState(false);

  useEffect(() => {
    if (!open || !message) return;
    setTranslation(null);
    setTranscription(null);
    setTarget(
      conversations.find((conversation) => conversation.id !== message.conversation_id)?.id ?? '',
    );
    if (demo) {
      setDetails({
        message,
        reactions: [],
        delivered_to: [],
        read_by: [],
        favorited: false,
        expires_at: null,
      });
      return;
    }
    setLoading(true);
    void api
      .messageDetails(message.id)
      .then(setDetails)
      .catch(() => setAnnouncement(tr('消息详情加载失败。')))
      .finally(() => setLoading(false));
  }, [conversations, demo, message, open, setAnnouncement]);

  if (!message) return null;

  async function react(emoji: string) {
    if (!message || !me || !details) return;
    const active = !details.reactions
      .find((reaction) => reaction.emoji === emoji)
      ?.user_ids.includes(me.id);
    if (demo) {
      setDetails({
        ...details,
        reactions: active ? [{ emoji, user_ids: [me.id] }] : [],
      });
      return;
    }
    try {
      setDetails({ ...details, reactions: await api.reactToMessage(message.id, emoji, active) });
    } catch {
      setAnnouncement(tr('表情回应失败。'));
    }
  }

  async function toggleFavorite() {
    if (!message || !details) return;
    const favorite = !details.favorited;
    try {
      if (!demo) await api.favoriteMessage(message.id, favorite);
      setDetails({ ...details, favorited: favorite });
    } catch {
      setAnnouncement(tr('收藏设置失败。'));
    }
  }

  async function recall(): Promise<boolean> {
    if (!message) return false;
    try {
      const recalled = demo
        ? {
            ...message,
            status: 'recalled' as const,
            content: { type: 'system' as const, data: { text: tr('消息已撤回') } },
            edited_at: new Date().toISOString(),
          }
        : await api.recallMessage(message.id);
      resolveMessage(message.client_message_id, recalled);
      setRecallConfirmOpen(false);
      onOpenChange(false);
      return true;
    } catch {
      setAnnouncement(tr('消息已超过可撤回时间，或你没有权限。'));
      return false;
    }
  }

  async function forward() {
    if (!message || !target) return;
    try {
      if (demo) {
        setAnnouncement(tr('演示模式不会转发到其他会话。'));
        return;
      }
      const forwarded = await api.forwardMessages([message.id], target);
      setMessages(target, forwarded);
      setAnnouncement(tr('消息已转发。'));
      onOpenChange(false);
    } catch {
      setAnnouncement(tr('消息转发失败。'));
    }
  }

  async function translate() {
    if (!message || message.content.type !== 'text' || translating) return;
    setTranslating(true);
    try {
      setTranslation(
        await api.translateMessage(
          message.id,
          useChatStore.getState().settings.language === 'zh-CN' ? 'zh' : 'en',
        ),
      );
    } catch {
      setAnnouncement(tr('翻译服务暂不可用，或该消息无法翻译。'));
    } finally {
      setTranslating(false);
    }
  }

  async function transcribe() {
    if (!message || message.content.type !== 'audio' || transcribing) return;
    setTranscribing(true);
    try {
      setTranscription(
        demo
          ? { text: tr('这是演示模式中的语音转写示例。'), language: 'zh' }
          : await api.transcribeMessage(message.id),
      );
    } catch {
      setAnnouncement(tr('语音转文字服务暂不可用。'));
    } finally {
      setTranscribing(false);
    }
  }

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog-content message-details-dialog">
          <header className="dialog-header">
            <div>
              <Dialog.Title>{tr('消息详情')}</Dialog.Title>
              <Dialog.Description>{tr('查看回执、回应，以及执行消息操作。')}</Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <IconButton label={tr('关闭')}>
                <X size={18} />
              </IconButton>
            </Dialog.Close>
          </header>
          <div className="message-details-preview">{messageSummary(message.content)}</div>
          {loading || !details ? (
            <p className="field-hint">{tr('正在加载详情…')}</p>
          ) : (
            <>
              <dl className="message-details-stats">
                <div>
                  <dt>{tr('序列号')}</dt>
                  <dd>{message.sequence ?? tr('待发送')}</dd>
                </div>
                <div>
                  <dt>{tr('已送达')}</dt>
                  <dd>{details.delivered_to.length}</dd>
                </div>
                <div>
                  <dt>{tr('已读')}</dt>
                  <dd>{details.read_by.length}</dd>
                </div>
                <div>
                  <dt>{tr('有效期')}</dt>
                  <dd>
                    {details.expires_at
                      ? new Date(details.expires_at).toLocaleString(currentLanguage())
                      : tr('永久')}
                  </dd>
                </div>
              </dl>
              <div className="reaction-picker" aria-label={tr('消息回应')}>
                {REACTIONS.map((emoji) => {
                  const reaction = details.reactions.find((item) => item.emoji === emoji);
                  const selected = Boolean(me && reaction?.user_ids.includes(me.id));
                  return (
                    <button
                      type="button"
                      className={selected ? 'is-selected' : ''}
                      key={emoji}
                      aria-pressed={selected}
                      onClick={() => void react(emoji)}
                    >
                      {emoji} {reaction?.user_ids.length || ''}
                    </button>
                  );
                })}
              </div>
              <div className="message-action-grid">
                <button
                  className="secondary-button"
                  type="button"
                  onClick={() => {
                    onReply(message);
                    onOpenChange(false);
                  }}
                >
                  <CornerUpRight size={16} /> {tr('回复')}
                </button>
                <button
                  className="secondary-button"
                  type="button"
                  onClick={() => void toggleFavorite()}
                >
                  <Bookmark size={16} /> {details.favorited ? tr('取消收藏') : tr('收藏')}
                </button>
                <button className="secondary-button" type="button" onClick={() => void react('👍')}>
                  <SmilePlus size={16} /> {tr('快速回应')}
                </button>
                {message.content.type === 'text' ? (
                  <button
                    className="secondary-button"
                    type="button"
                    disabled={translating}
                    onClick={() => void translate()}
                  >
                    <Languages size={16} /> {translating ? tr('翻译中…') : tr('翻译')}
                  </button>
                ) : null}
                {message.content.type === 'audio' ? (
                  <button
                    className="secondary-button"
                    type="button"
                    disabled={transcribing}
                    onClick={() => void transcribe()}
                  >
                    <AudioLines size={16} /> {transcribing ? tr('转写中…') : tr('转为文字')}
                  </button>
                ) : null}
                {message.sender_id === me?.id ? (
                  <button
                    className="secondary-button"
                    type="button"
                    onClick={() => setRecallConfirmOpen(true)}
                  >
                    <RotateCcw size={16} /> {tr('撤回')}
                  </button>
                ) : null}
              </div>
              {translation ? (
                <div className="message-translation" aria-live="polite">
                  <small>
                    {translation.source_language ?? tr('自动检测')} → {translation.target_language}
                  </small>
                  <p>{translation.translated_text}</p>
                </div>
              ) : null}
              {transcription ? (
                <div className="message-translation" aria-live="polite">
                  <small>
                    {tr('语音转写')}
                    {transcription.language ? ` · ${transcription.language}` : ''}
                  </small>
                  <p>{transcription.text}</p>
                </div>
              ) : null}
              <div className="forward-row">
                <label>
                  {tr('转发到')}
                  <select value={target} onChange={(event) => setTarget(event.target.value)}>
                    <option value="">{tr('选择会话')}</option>
                    {conversations
                      .filter((conversation) => conversation.id !== message.conversation_id)
                      .map((conversation) => (
                        <option key={conversation.id} value={conversation.id}>
                          {conversationName(conversation, friends, friendSettings)}
                        </option>
                      ))}
                  </select>
                </label>
                <button
                  className="primary-button"
                  type="button"
                  disabled={!target}
                  onClick={() => void forward()}
                >
                  {tr('转发')}
                </button>
              </div>
            </>
          )}
          <p className="message-detail-note">
            <Info size={14} /> {tr('消息 ID：')}
            {message.id}
          </p>
          <ConfirmDialog
            open={recallConfirmOpen}
            onOpenChange={setRecallConfirmOpen}
            title={tr('撤回消息')}
            description={tr('确定撤回这条消息？撤回后会向会话成员显示提示。')}
            confirmLabel={tr('确认撤回')}
            danger
            onConfirm={recall}
          />
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
