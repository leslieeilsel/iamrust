import {
  CalendarClock,
  FilePlus2,
  Image,
  MonitorUp,
  Paperclip,
  Plus,
  SendHorizontal,
  Smile,
  Sticker as StickerIcon,
  Timer,
  Trash2,
  X,
} from 'lucide-react';
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
} from 'react';

import { IconButton } from '../../components/IconButton';
import { api } from '../../lib/api';
import { formatFileSize, messageSummary } from '../../lib/format';
import { createId } from '../../lib/id';
import { hasSafeImageHeader, prepareImageForUpload } from '../../lib/image-processing';
import { persistDraft } from '../../lib/local-cache';
import { sendTyping } from '../../lib/realtime';
import type {
  Attachment,
  ConversationId,
  Message,
  PendingUpload,
  Sticker,
  UserId,
} from '../../lib/types';
import { useChatStore } from '../../state/chat-store';
import { VoiceRecorder } from './VoiceRecorder';
import { tr } from '../../lib/i18n';

const MAX_TEXT = 8_000;
const MAX_FILES = 10;
const MAX_FILE_SIZE = 100 * 1024 * 1024;
const EMOJIS = [
  '😀',
  '😂',
  '🥹',
  '😍',
  '🤔',
  '👍',
  '👏',
  '🎉',
  '❤️',
  '🔥',
  '🦀',
  '✨',
  '✅',
  '👀',
  '🙏',
];
const RECENT_EMOJI_KEY = 'iamrust-recent-emojis';

interface MessageComposerProps {
  conversationId: ConversationId;
  disabled?: boolean;
  reply?: Message | null;
  onCancelReply?: () => void;
  onSend: (
    text: string,
    mentions: UserId[],
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
  ) => Promise<boolean>;
  onSchedule?: (
    text: string,
    mentions: UserId[],
    mentionAll: boolean,
    scheduledFor: string,
    expiresInSeconds: number | null,
  ) => Promise<boolean>;
  onVoice?: (blob: Blob, durationMs: number, mimeType: string) => Promise<boolean>;
  onSticker?: (sticker: Sticker) => Promise<boolean>;
}

interface MentionTarget {
  id: UserId | null;
  label: string;
  token: string;
  all: boolean;
}

interface MentionContext {
  start: number;
  cursor: number;
  query: string;
}

export function MessageComposer({
  conversationId,
  disabled = false,
  reply = null,
  onCancelReply = () => undefined,
  onSend,
  onSchedule = () => Promise.resolve(false),
  onVoice = () => Promise.resolve(false),
  onSticker = () => Promise.resolve(false),
}: MessageComposerProps) {
  const draft = useChatStore((state) => state.meta[conversationId]?.draft ?? '');
  const setDraft = useChatStore((state) => state.setDraft);
  const shortcut = useChatStore((state) => state.settings.sendShortcut);
  const demo = useChatStore((state) => state.demo);
  const conversation = useChatStore((state) =>
    state.conversations.find((item) => item.id === conversationId),
  );
  const friends = useChatStore((state) => state.friends);
  const me = useChatStore((state) => state.me);
  const [files, setFiles] = useState<PendingUpload[]>([]);
  const [emojiOpen, setEmojiOpen] = useState(false);
  const [recentEmojis, setRecentEmojis] = useState(loadRecentEmojis);
  const [stickerOpen, setStickerOpen] = useState(false);
  const [stickers, setStickers] = useState<Sticker[]>([]);
  const [stickersLoading, setStickersLoading] = useState(false);
  const [sendingStickerId, setSendingStickerId] = useState<string | null>(null);
  const [uploadingSticker, setUploadingSticker] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState('');
  const [sending, setSending] = useState(false);
  const [sendOptionsOpen, setSendOptionsOpen] = useState(false);
  const [expiresInSeconds, setExpiresInSeconds] = useState<number | null>(null);
  const [sendOriginalImages, setSendOriginalImages] = useState(false);
  const [scheduledFor, setScheduledFor] = useState(() => datetimeLocalValue(Date.now() + 60_000));
  const [mentionContext, setMentionContext] = useState<MentionContext | null>(null);
  const [activeMention, setActiveMention] = useState(0);
  const composing = useRef(false);
  const textarea = useRef<HTMLTextAreaElement>(null);
  const imageInput = useRef<HTMLInputElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const stickerInput = useRef<HTMLInputElement>(null);
  const transfer = useRef<AbortController | null>(null);
  const typingTimeout = useRef<number | null>(null);
  const lastTypingSignal = useRef(0);

  const mentionTargets = useMemo<MentionTarget[]>(() => {
    if (conversation?.kind.kind !== 'group' || !me) return [];
    const used = new Set<string>([tr('所有人')]);
    const targets: MentionTarget[] = Object.values(conversation.members)
      .filter((member) => member.user_id !== me.id)
      .map((member): MentionTarget => {
        const profile = friends.find((friend) => friend.id === member.user_id);
        const label = member.nickname ?? profile?.nickname ?? profile?.username ?? tr('群成员');
        const base = mentionToken(
          profile?.username ?? member.nickname ?? member.user_id.slice(0, 8),
        );
        let token = base;
        if (used.has(token.toLocaleLowerCase())) token = `${base}_${member.user_id.slice(0, 4)}`;
        used.add(token.toLocaleLowerCase());
        return { id: member.user_id, label, token, all: false };
      });
    const role = conversation.members[me.id]?.role;
    if (role === 'owner' || role === 'administrator') {
      targets.unshift({ id: null, label: tr('所有人'), token: tr('所有人'), all: true });
    }
    return targets;
  }, [conversation, friends, me]);

  const mentionSuggestions = useMemo(() => {
    if (!mentionContext) return [];
    const query = mentionContext.query.toLocaleLowerCase();
    return mentionTargets
      .filter(
        (target) =>
          target.token.toLocaleLowerCase().includes(query) ||
          target.label.toLocaleLowerCase().includes(query),
      )
      .slice(0, 8);
  }, [mentionContext, mentionTargets]);
  const emojiOptions = useMemo(
    () => [...recentEmojis, ...EMOJIS.filter((emoji) => !recentEmojis.includes(emoji))],
    [recentEmojis],
  );

  useEffect(() => setActiveMention(0), [mentionContext?.query]);

  useEffect(() => {
    setFiles((current) => {
      current.forEach((item) => {
        if (item.previewUrl) URL.revokeObjectURL(item.previewUrl);
      });
      return [];
    });
    setEmojiOpen(false);
    setStickerOpen(false);
    setSendOptionsOpen(false);
    setMentionContext(null);
    setError('');
    requestAnimationFrame(() => textarea.current?.focus());
    return () => {
      if (typingTimeout.current !== null) window.clearTimeout(typingTimeout.current);
      if (!demo) sendTyping(conversationId, false);
    };
  }, [conversationId, demo]);

  useEffect(() => {
    if (!stickerOpen || demo) return;
    let cancelled = false;
    setStickersLoading(true);
    void api
      .stickers()
      .then((items) => {
        if (!cancelled) setStickers(items);
      })
      .catch(() => {
        if (!cancelled) setError(tr('表情库加载失败。'));
      })
      .finally(() => {
        if (!cancelled) setStickersLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [demo, stickerOpen]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void persistDraft(conversationId, draft).catch(() => undefined);
      if (!demo) {
        void api.updateConversationSettings(conversationId, { draft }).catch(() => undefined);
      }
    }, 350);
    return () => window.clearTimeout(timer);
  }, [conversationId, demo, draft]);

  function appendFiles(incoming: FileList | File[]) {
    const nextFiles = Array.from(incoming);
    const available = Math.max(0, MAX_FILES - files.length);
    const accepted: PendingUpload[] = [];
    let rejection = '';
    for (const file of nextFiles.slice(0, available)) {
      if (file.size === 0) {
        rejection = tr(`${file.name} 是空文件。`);
        continue;
      }
      if (file.size > MAX_FILE_SIZE) {
        rejection = tr(`${file.name} 超过 100 MB。`);
        continue;
      }
      accepted.push({
        localId: createId(),
        file,
        previewUrl: file.type.startsWith('image/') ? URL.createObjectURL(file) : null,
        progress: 0,
        status: 'ready',
        error: null,
      });
    }
    if (nextFiles.length > available) rejection = tr(`一次最多发送 ${MAX_FILES} 个文件。`);
    setError(rejection);
    setFiles((current) => [...current, ...accepted]);
  }

  function removeFile(localId: string) {
    setFiles((current) => {
      const target = current.find((item) => item.localId === localId);
      if (target?.previewUrl) URL.revokeObjectURL(target.previewUrl);
      return current.filter((item) => item.localId !== localId);
    });
  }

  async function captureScreenshot() {
    if (!navigator.mediaDevices?.getDisplayMedia) {
      setError(tr('当前系统不支持截图。'));
      return;
    }
    let stream: MediaStream | null = null;
    try {
      stream = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: false });
      const video = document.createElement('video');
      video.srcObject = stream;
      video.muted = true;
      await video.play();
      if (!video.videoWidth || !video.videoHeight) throw new Error('empty display frame');
      const canvas = document.createElement('canvas');
      canvas.width = video.videoWidth;
      canvas.height = video.videoHeight;
      const context = canvas.getContext('2d');
      if (!context) throw new Error('canvas unavailable');
      context.drawImage(video, 0, 0);
      const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/png'));
      if (!blob) throw new Error('screenshot encoding failed');
      appendFiles([
        new File([blob], `screenshot-${new Date().toISOString().replace(/[:.]/gu, '-')}.png`, {
          type: 'image/png',
        }),
      ]);
      setError('');
    } catch {
      setError(tr('截图已取消或没有屏幕录制权限。'));
    } finally {
      stream?.getTracks().forEach((track) => track.stop());
    }
  }

  async function addSticker(file: File) {
    if (uploadingSticker) return;
    if (file.size === 0 || file.size > 10 * 1024 * 1024) {
      setError(tr('自定义表情需为 10 MB 以内的图片。'));
      return;
    }
    setUploadingSticker(true);
    try {
      if (!(await hasSafeImageHeader(file))) throw new Error('invalid image signature');
      const prepared = await prepareImageForUpload(file, false);
      const name =
        file.name
          .replace(/\.[^.]+$/u, '')
          .trim()
          .slice(0, 48) || tr('自定义表情');
      let sticker: Sticker;
      if (demo) {
        const source = URL.createObjectURL(prepared);
        const attachment: Attachment = {
          id: createId(),
          kind: 'image',
          file_name: prepared.name,
          mime_type: prepared.type,
          byte_size: prepared.size,
          sha256: null,
          storage_key: source,
          thumbnail_key: source,
        };
        sticker = {
          id: createId(),
          owner_id: me?.id ?? createId(),
          attachment,
          name,
          shortcut: null,
          created_at: new Date().toISOString(),
        };
      } else {
        const uploaded = await api.upload(prepared);
        sticker = await api.createSticker(uploaded.attachment.id, name);
      }
      setStickers((items) => [...items, sticker]);
      setError('');
    } catch {
      setError(tr('表情添加失败，请使用 PNG、JPEG、GIF 或 WebP。'));
    } finally {
      setUploadingSticker(false);
    }
  }

  async function chooseSticker(sticker: Sticker) {
    if (sendingStickerId || disabled) return;
    setSendingStickerId(sticker.id);
    const ok = await onSticker(sticker);
    setSendingStickerId(null);
    if (ok) {
      setStickerOpen(false);
      setError('');
    } else {
      setError(tr('表情发送失败，可以重试。'));
    }
  }

  async function removeSticker(sticker: Sticker) {
    try {
      if (!demo) await api.deleteSticker(sticker.id);
      if (demo && sticker.attachment.storage_key.startsWith('blob:')) {
        URL.revokeObjectURL(sticker.attachment.storage_key);
      }
      setStickers((items) => items.filter((item) => item.id !== sticker.id));
      setError('');
    } catch {
      setError(tr('表情删除失败。'));
    }
  }

  async function submit() {
    if (sending || disabled || (!draft.trim() && files.length === 0)) return;
    setSending(true);
    stopTyping();
    const controller = new AbortController();
    transfer.current = controller;
    setFiles((items) =>
      items.map((item) => ({ ...item, progress: 0, status: 'uploading', error: null })),
    );
    const updateUpload = (
      localId: string,
      progress: number,
      status: PendingUpload['status'],
      uploadError: string | null,
    ) => {
      setFiles((items) =>
        items.map((item) =>
          item.localId === localId ? { ...item, progress, status, error: uploadError } : item,
        ),
      );
    };
    const mentionMetadata = extractMentions(draft, mentionTargets);
    const ok = await onSend(
      draft.trim(),
      mentionMetadata.mentions,
      mentionMetadata.mentionAll,
      sendOriginalImages,
      files,
      expiresInSeconds,
      updateUpload,
      controller.signal,
    );
    transfer.current = null;
    if (ok) {
      setDraft(conversationId, '');
      files.forEach((item) => {
        if (item.previewUrl) URL.revokeObjectURL(item.previewUrl);
      });
      setFiles([]);
      setError('');
    } else {
      setFiles((items) =>
        items
          .filter((item) => item.status !== 'completed')
          .map((item) =>
            item.status === 'uploading'
              ? { ...item, status: 'failed', error: tr('传输已取消') }
              : item,
          ),
      );
    }
    setSending(false);
    requestAnimationFrame(() => textarea.current?.focus());
  }

  async function scheduleSubmit() {
    if (sending || disabled || !draft.trim() || files.length > 0) return;
    const timestamp = new Date(scheduledFor).getTime();
    if (!Number.isFinite(timestamp) || timestamp < Date.now() + 10_000) {
      setError(tr('定时发送时间至少要晚于现在 10 秒。'));
      return;
    }
    setSending(true);
    stopTyping();
    const mentionMetadata = extractMentions(draft, mentionTargets);
    const ok = await onSchedule(
      draft.trim(),
      mentionMetadata.mentions,
      mentionMetadata.mentionAll,
      new Date(timestamp).toISOString(),
      expiresInSeconds,
    );
    if (ok) {
      setDraft(conversationId, '');
      setSendOptionsOpen(false);
      setScheduledFor(datetimeLocalValue(Date.now() + 60_000));
      setError('');
    }
    setSending(false);
    requestAnimationFrame(() => textarea.current?.focus());
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (composing.current || event.nativeEvent.isComposing) return;
    if (mentionContext && mentionSuggestions.length > 0) {
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault();
        setActiveMention((current) => {
          const offset = event.key === 'ArrowDown' ? 1 : -1;
          return (current + offset + mentionSuggestions.length) % mentionSuggestions.length;
        });
        return;
      }
      if (event.key === 'Enter' || event.key === 'Tab') {
        event.preventDefault();
        selectMention(mentionSuggestions[activeMention] ?? mentionSuggestions[0]!);
        return;
      }
    }
    if (event.key === 'Escape' && mentionContext) {
      event.preventDefault();
      setMentionContext(null);
      return;
    }
    if (event.key !== 'Enter') return;
    const shouldSend =
      shortcut === 'enter'
        ? !event.shiftKey && !event.ctrlKey && !event.metaKey
        : event.ctrlKey || event.metaKey;
    if (shouldSend) {
      event.preventDefault();
      void submit();
    }
  }

  function onPaste(event: ClipboardEvent<HTMLTextAreaElement>) {
    const pastedFiles = Array.from(event.clipboardData.files);
    if (pastedFiles.length) {
      event.preventDefault();
      appendFiles(pastedFiles);
    }
  }

  function onDrop(event: DragEvent) {
    event.preventDefault();
    setDragging(false);
    appendFiles(event.dataTransfer.files);
  }

  function updateDraft(value: string, cursor = value.length) {
    setDraft(conversationId, value);
    setMentionContext(findMentionContext(value, cursor));
    if (demo || disabled) return;
    if (!value.trim()) {
      stopTyping();
      return;
    }
    const now = Date.now();
    if (now - lastTypingSignal.current > 1_500) {
      sendTyping(conversationId, true);
      lastTypingSignal.current = now;
    }
    if (typingTimeout.current !== null) window.clearTimeout(typingTimeout.current);
    typingTimeout.current = window.setTimeout(stopTyping, 3_000);
  }

  function selectMention(target: MentionTarget) {
    if (!mentionContext) return;
    const insertion = `@${target.token} `;
    const next = `${draft.slice(0, mentionContext.start)}${insertion}${draft.slice(mentionContext.cursor)}`;
    const nextCursor = mentionContext.start + insertion.length;
    updateDraft(next, nextCursor);
    setMentionContext(null);
    requestAnimationFrame(() => {
      textarea.current?.focus();
      textarea.current?.setSelectionRange(nextCursor, nextCursor);
    });
  }

  function stopTyping() {
    if (typingTimeout.current !== null) window.clearTimeout(typingTimeout.current);
    typingTimeout.current = null;
    if (lastTypingSignal.current !== 0 && !demo) sendTyping(conversationId, false);
    lastTypingSignal.current = 0;
  }

  return (
    <div
      className={`message-composer ${dragging ? 'is-dragging' : ''}`}
      onDragEnter={(event) => {
        event.preventDefault();
        setDragging(true);
      }}
      onDragOver={(event) => event.preventDefault()}
      onDragLeave={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDragging(false);
      }}
      onDrop={onDrop}
    >
      {dragging ? (
        <div className="drop-overlay">
          <FilePlus2 />
          <strong>{tr('松开发送文件')}</strong>
        </div>
      ) : null}
      {reply ? (
        <div className="composer-reply">
          <span>
            <strong>{tr('回复消息')}</strong>
            <small>{messageSummary(reply.content)}</small>
          </span>
          <IconButton label={tr('取消回复')} onClick={onCancelReply}>
            <X size={15} />
          </IconButton>
        </div>
      ) : null}
      {files.length ? (
        <div className="attachment-preview" aria-label={tr('待发送附件')}>
          {files.map((item) => (
            <div key={item.localId}>
              {item.previewUrl ? <img src={item.previewUrl} alt="" /> : <Paperclip size={20} />}
              <span>
                <strong title={item.file.name}>{item.file.name}</strong>
                <small>
                  {formatFileSize(item.file.size)}
                  {item.status === 'uploading' ? ` · ${item.progress}%` : ''}
                  {item.status === 'failed' ? tr(` · ${item.error ?? '上传失败，可重试'}`) : ''}
                </small>
                {item.status === 'uploading' ? (
                  <span className="upload-progress" aria-label={tr(`上传进度 ${item.progress}%`)}>
                    <i style={{ width: `${item.progress}%` }} />
                  </span>
                ) : null}
              </span>
              <IconButton
                label={
                  item.status === 'uploading'
                    ? tr(`取消 ${item.file.name}`)
                    : tr(`移除 ${item.file.name}`)
                }
                onClick={() => {
                  if (item.status === 'uploading') transfer.current?.abort();
                  else removeFile(item.localId);
                }}
              >
                <X size={15} />
              </IconButton>
            </div>
          ))}
        </div>
      ) : null}
      <div className="composer-toolbar">
        <IconButton label={tr('选择图片')} onClick={() => imageInput.current?.click()}>
          <Image size={19} />
        </IconButton>
        <IconButton label={tr('选择文件')} onClick={() => fileInput.current?.click()}>
          <Paperclip size={19} />
        </IconButton>
        <IconButton label={tr('截取屏幕并发送')} onClick={() => void captureScreenshot()}>
          <MonitorUp size={18} />
        </IconButton>
        <VoiceRecorder disabled={disabled || sending} onSend={onVoice} />
        <span className="emoji-wrap">
          <IconButton
            label={tr('选择 Emoji')}
            active={emojiOpen}
            onClick={() => {
              setEmojiOpen((value) => !value);
              setStickerOpen(false);
            }}
          >
            <Smile size={19} />
          </IconButton>
          {emojiOpen ? (
            <div className="emoji-picker" role="listbox" aria-label="Emoji">
              {emojiOptions.map((emoji) => (
                <button
                  key={emoji}
                  type="button"
                  role="option"
                  onClick={() => {
                    updateDraft(`${draft}${emoji}`);
                    rememberEmoji(emoji, setRecentEmojis);
                    setEmojiOpen(false);
                    textarea.current?.focus();
                  }}
                >
                  {emoji}
                </button>
              ))}
            </div>
          ) : null}
        </span>
        <span className="sticker-wrap">
          <IconButton
            label={tr('自定义表情')}
            active={stickerOpen}
            onClick={() => {
              setStickerOpen((value) => !value);
              setEmojiOpen(false);
              setSendOptionsOpen(false);
            }}
          >
            <StickerIcon size={19} />
          </IconButton>
          {stickerOpen ? (
            <div className="sticker-picker" role="dialog" aria-label={tr('自定义表情')}>
              <header>
                <strong>{tr('自定义表情')}</strong>
                <button
                  type="button"
                  disabled={uploadingSticker || stickers.length >= 100}
                  onClick={() => stickerInput.current?.click()}
                >
                  <Plus size={15} /> {uploadingSticker ? tr('正在添加…') : tr('添加')}
                </button>
              </header>
              {stickersLoading ? <p>{tr('正在加载表情…')}</p> : null}
              {!stickersLoading && stickers.length === 0 ? (
                <p>{tr('还没有自定义表情，可以添加 PNG、JPEG、GIF 或 WebP。')}</p>
              ) : null}
              <div className="sticker-grid">
                {stickers.map((sticker) => (
                  <span key={sticker.id} className="sticker-tile">
                    <button
                      type="button"
                      disabled={Boolean(sendingStickerId)}
                      aria-label={tr(`发送表情 ${sticker.name}`)}
                      title={sticker.name}
                      onClick={() => void chooseSticker(sticker)}
                    >
                      <StickerThumbnail sticker={sticker} />
                    </button>
                    <button
                      className="sticker-delete"
                      type="button"
                      aria-label={tr(`删除表情 ${sticker.name}`)}
                      title={tr('删除表情')}
                      onClick={() => void removeSticker(sticker)}
                    >
                      <Trash2 size={12} />
                    </button>
                  </span>
                ))}
              </div>
            </div>
          ) : null}
        </span>
        <span className="send-options-wrap">
          <IconButton
            label={tr('定时发送与消息有效期')}
            active={sendOptionsOpen}
            onClick={() => setSendOptionsOpen((value) => !value)}
          >
            <CalendarClock size={18} />
          </IconButton>
          {sendOptionsOpen ? (
            <div className="send-options" role="dialog" aria-label={tr('发送选项')}>
              <label>
                <span>
                  <Timer size={14} /> {tr('消息有效期')}
                </span>
                <select
                  value={expiresInSeconds ?? ''}
                  onChange={(event) =>
                    setExpiresInSeconds(event.target.value ? Number(event.target.value) : null)
                  }
                >
                  <option value="">{tr('永久保留')}</option>
                  <option value="10">{tr('阅后 10 秒')}</option>
                  <option value="3600">{tr('1 小时')}</option>
                  <option value="86400">{tr('1 天')}</option>
                  <option value="604800">{tr('7 天')}</option>
                </select>
              </label>
              <label className="checkbox-row">
                <input
                  type="checkbox"
                  checked={sendOriginalImages}
                  onChange={(event) => setSendOriginalImages(event.target.checked)}
                />
                <span>{tr('发送原图（保留原始尺寸与元数据）')}</span>
              </label>
              <label>
                <span>{tr('定时发送')}</span>
                <input
                  type="datetime-local"
                  value={scheduledFor}
                  min={datetimeLocalValue(Date.now() + 10_000)}
                  onChange={(event) => setScheduledFor(event.target.value)}
                />
              </label>
              <button
                className="secondary-button"
                type="button"
                disabled={sending || !draft.trim() || files.length > 0}
                onClick={() => void scheduleSubmit()}
              >
                <CalendarClock size={15} /> {tr('创建定时消息')}
              </button>
              {files.length > 0 ? <small>{tr('定时消息目前仅支持文字。')}</small> : null}
            </div>
          ) : null}
        </span>
        <span className="composer-shortcut">
          {shortcut === 'enter' ? tr('Enter 发送 · Shift+Enter 换行') : tr('⌘/Ctrl+Enter 发送')}
        </span>
      </div>
      {mentionContext && mentionSuggestions.length > 0 ? (
        <div className="mention-suggestions" role="listbox" aria-label={tr('选择要提及的群成员')}>
          {mentionSuggestions.map((target, index) => (
            <button
              key={target.id ?? 'all'}
              type="button"
              role="option"
              aria-selected={index === activeMention}
              className={index === activeMention ? 'is-active' : ''}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => selectMention(target)}
            >
              <strong>{target.all ? tr('@所有人') : target.label}</strong>
              <small>{target.all ? tr('通知全部群成员') : `@${target.token}`}</small>
            </button>
          ))}
        </div>
      ) : null}
      <div className="composer-input-row">
        <label className="sr-only" htmlFor="message-input">
          {tr('输入消息')}
        </label>
        <textarea
          id="message-input"
          ref={textarea}
          value={draft}
          disabled={disabled}
          rows={2}
          maxLength={MAX_TEXT}
          placeholder={disabled ? tr('当前离线，仍可编辑草稿') : tr('输入消息…')}
          aria-autocomplete={mentionTargets.length ? 'list' : undefined}
          aria-expanded={mentionTargets.length ? Boolean(mentionSuggestions.length) : undefined}
          onChange={(event) => updateDraft(event.target.value, event.target.selectionStart)}
          onClick={(event) =>
            setMentionContext(
              findMentionContext(event.currentTarget.value, event.currentTarget.selectionStart),
            )
          }
          onKeyUp={(event) => {
            if (['ArrowDown', 'ArrowUp', 'Enter', 'Tab', 'Escape'].includes(event.key)) return;
            setMentionContext(
              findMentionContext(event.currentTarget.value, event.currentTarget.selectionStart),
            );
          }}
          onKeyDown={onKeyDown}
          onCompositionStart={() => {
            composing.current = true;
          }}
          onCompositionEnd={() => {
            composing.current = false;
          }}
          onPaste={onPaste}
        />
        <button
          className="send-button"
          type="button"
          aria-label={tr('发送消息')}
          disabled={sending || disabled || (!draft.trim() && files.length === 0)}
          onClick={() => void submit()}
        >
          <SendHorizontal size={19} />
        </button>
      </div>
      <div className="composer-footer">
        <span role="alert">{error}</span>
        <span className={Array.from(draft).length > MAX_TEXT * 0.9 ? 'is-near-limit' : ''}>
          {Array.from(draft).length}/{MAX_TEXT}
        </span>
      </div>
      <input
        ref={imageInput}
        hidden
        type="file"
        accept="image/png,image/jpeg,image/gif,image/webp"
        multiple
        onChange={(event) => event.target.files && appendFiles(event.target.files)}
      />
      <input
        ref={fileInput}
        hidden
        type="file"
        multiple
        onChange={(event) => event.target.files && appendFiles(event.target.files)}
      />
      <input
        ref={stickerInput}
        hidden
        type="file"
        accept="image/png,image/jpeg,image/gif,image/webp"
        onChange={(event) => {
          const file = event.target.files?.[0];
          event.target.value = '';
          if (file) void addSticker(file);
        }}
      />
    </div>
  );
}

function StickerThumbnail({ sticker }: { sticker: Sticker }) {
  const local = sticker.attachment.thumbnail_key ?? sticker.attachment.storage_key;
  const [source, setSource] = useState(() =>
    /^(blob:|data:|https?:\/\/)/u.test(local) ? local : null,
  );

  useEffect(() => {
    if (/^(blob:|data:|https?:\/\/)/u.test(local)) {
      setSource(local);
      return;
    }
    let cancelled = false;
    void api
      .attachmentDownloadUrl(sticker.attachment.id)
      .then((url) => {
        if (!cancelled) setSource(url);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [local, sticker.attachment.id]);

  return source ? <img src={source} alt="" loading="lazy" /> : <StickerIcon size={22} />;
}

function datetimeLocalValue(timestamp: number): string {
  const date = new Date(timestamp);
  const local = new Date(timestamp - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

function mentionToken(value: string): string {
  return (
    value
      .trim()
      .replace(/^@/u, '')
      .replace(/[^\p{L}\p{N}_.-]+/gu, '_')
      .slice(0, 48) || 'member'
  );
}

function findMentionContext(value: string, cursor: number): MentionContext | null {
  const before = value.slice(0, cursor);
  const match = before.match(/(?:^|\s)@([\p{L}\p{N}_.-]*)$/u);
  if (!match) return null;
  const query = match[1] ?? '';
  return { start: cursor - query.length - 1, cursor, query };
}

function loadRecentEmojis(): string[] {
  try {
    const value = JSON.parse(localStorage.getItem(RECENT_EMOJI_KEY) ?? '[]') as unknown;
    return Array.isArray(value)
      ? value.filter((item): item is string => typeof item === 'string').slice(0, 8)
      : [];
  } catch {
    return [];
  }
}

function rememberEmoji(emoji: string, update: (value: string[]) => void) {
  const recent = [emoji, ...loadRecentEmojis().filter((item) => item !== emoji)].slice(0, 8);
  localStorage.setItem(RECENT_EMOJI_KEY, JSON.stringify(recent));
  update(recent);
}

function extractMentions(
  text: string,
  targets: MentionTarget[],
): { mentions: UserId[]; mentionAll: boolean } {
  const matches = (token: string) => {
    const escaped = token.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&');
    return new RegExp(`(^|\\s)@${escaped}(?=$|[\\s,.!?，。！？:：;；)\\]}])`, 'iu').test(text);
  };
  return {
    mentions: targets
      .filter((target) => target.id && matches(target.token))
      .map((target) => target.id as UserId),
    mentionAll: targets.some((target) => target.all && matches(target.token)),
  };
}
