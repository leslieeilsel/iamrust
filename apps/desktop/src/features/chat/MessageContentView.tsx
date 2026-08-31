import * as Dialog from '@radix-ui/react-dialog';
import {
  ChevronLeft,
  ChevronRight,
  Download,
  File,
  FolderOpen,
  ImageOff,
  Pause,
  Play,
  RotateCw,
  X,
  ZoomIn,
  ZoomOut,
} from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { IconButton } from '../../components/IconButton';
import { api } from '../../lib/api';
import { formatFileSize, formatFullTime, splitLinks } from '../../lib/format';
import { claimAudio, releaseAudio } from '../../lib/audio-playback';
import type { Attachment, MessageContent } from '../../lib/types';
import { useChatStore } from '../../state/chat-store';
import { tr } from '../../lib/i18n';

interface MessageContentViewProps {
  content: MessageContent;
  imageGallery?: Attachment[];
  hasMentions?: boolean;
}

export function MessageContentView({
  content,
  imageGallery = [],
  hasMentions = false,
}: MessageContentViewProps) {
  switch (content.type) {
    case 'text':
      return (
        <p className="message-text">
          {splitLinks(content.data.text).map((part, index) =>
            part.href ? (
              <a
                key={`${part.value}-${index}`}
                href={part.href}
                target="_blank"
                rel="noreferrer noopener"
              >
                {part.value}
              </a>
            ) : (
              <span key={`${part.value}-${index}`}>
                {renderInlineText(part.value, hasMentions)}
              </span>
            ),
          )}
        </p>
      );
    case 'system':
      return <p className="system-message">{content.data.text}</p>;
    case 'image':
      return <ImageMessage content={content} gallery={imageGallery} />;
    case 'file':
      return <FileMessage attachment={content.data.attachment} />;
    case 'audio':
      return <AudioMessage content={content} />;
    case 'sticker':
      return <StickerMessage content={content} />;
    case 'forward_bundle':
      return <ForwardBundle content={content} />;
  }
}

function StickerMessage({ content }: { content: Extract<MessageContent, { type: 'sticker' }> }) {
  const source = useAttachmentSource(content.data.attachment, true);
  const [failed, setFailed] = useState(false);

  if (!source || failed) {
    return (
      <div className="sticker-message sticker-message--loading" aria-label={content.data.name}>
        <ImageOff size={22} />
      </div>
    );
  }

  return (
    <img
      className="sticker-message"
      src={source}
      alt={content.data.name}
      loading="lazy"
      decoding="async"
      onError={() => setFailed(true)}
    />
  );
}

function ForwardBundle({
  content,
}: {
  content: Extract<MessageContent, { type: 'forward_bundle' }>;
}) {
  return (
    <details className="forward-bundle">
      <summary>
        <strong>{content.data.title}</strong>
        <small>
          {content.data.messages.length} {tr('条消息')}
        </small>
      </summary>
      <div className="forward-bundle__messages">
        {content.data.messages.map((message, index) => (
          <article key={`${message.sender_id}-${message.created_at}-${index}`}>
            <header>
              <strong>{message.sender_name}</strong>
              <time dateTime={message.created_at}>{formatFullTime(message.created_at)}</time>
            </header>
            <MessageContentView content={message.content} />
          </article>
        ))}
      </div>
    </details>
  );
}

function renderInlineText(text: string, hasMentions: boolean) {
  const pattern = /(\*\*[^*\n]+\*\*|~~[^~\n]+~~|`[^`\n]+`|\*[^*\n]+\*|@[\p{L}\p{N}_.-]+)/gu;
  return text.split(pattern).map((part, index) => {
    const key = `${part}-${index}`;
    if (part.startsWith('**') && part.endsWith('**')) {
      return <strong key={key}>{part.slice(2, -2)}</strong>;
    }
    if (part.startsWith('~~') && part.endsWith('~~')) {
      return <del key={key}>{part.slice(2, -2)}</del>;
    }
    if (part.startsWith('`') && part.endsWith('`')) {
      return <code key={key}>{part.slice(1, -1)}</code>;
    }
    if (part.startsWith('*') && part.endsWith('*')) {
      return <em key={key}>{part.slice(1, -1)}</em>;
    }
    if (hasMentions && part.startsWith('@')) {
      return (
        <mark className="message-mention" key={key}>
          {part}
        </mark>
      );
    }
    return part;
  });
}

function AudioMessage({ content }: { content: Extract<MessageContent, { type: 'audio' }> }) {
  const source = useAttachmentSource(content.data.attachment, true);
  const audio = useRef<HTMLAudioElement>(null);
  const [playing, setPlaying] = useState(false);
  const [currentMs, setCurrentMs] = useState(0);
  const [failed, setFailed] = useState(false);
  const durationMs = Math.max(1, content.data.duration_ms);

  useEffect(
    () => () => {
      if (audio.current) releaseAudio(audio.current);
    },
    [],
  );

  function toggle() {
    const player = audio.current;
    if (!player || !source || failed) return;
    if (player.paused) void player.play().catch(() => setFailed(true));
    else player.pause();
  }

  return (
    <div className="audio-message">
      <audio
        ref={audio}
        src={source ?? undefined}
        preload="metadata"
        onPlay={(event) => {
          claimAudio(event.currentTarget);
          setPlaying(true);
        }}
        onPause={() => setPlaying(false)}
        onTimeUpdate={(event) => setCurrentMs(event.currentTarget.currentTime * 1_000)}
        onEnded={(event) => {
          releaseAudio(event.currentTarget);
          setPlaying(false);
          setCurrentMs(0);
        }}
        onError={() => setFailed(true)}
      />
      <button
        type="button"
        disabled={!source || failed}
        aria-label={failed ? tr('语音加载失败') : playing ? tr('暂停语音消息') : tr('播放语音消息')}
        onClick={toggle}
      >
        {playing ? <Pause size={17} /> : <Play size={17} />}
      </button>
      <span className="audio-progress" aria-hidden="true">
        <i style={{ width: `${Math.min(100, (currentMs / durationMs) * 100)}%` }} />
      </span>
      <span>{failed ? tr('加载失败') : `${Math.ceil(durationMs / 1_000)}″`}</span>
    </div>
  );
}

function ImageMessage({
  content,
  gallery,
}: {
  content: Extract<MessageContent, { type: 'image' }>;
  gallery: Attachment[];
}) {
  const [failed, setFailed] = useState(false);
  const [viewerOpen, setViewerOpen] = useState(false);
  const [remoteSource, setRemoteSource] = useState<string | null>(null);
  const [pausedSource, setPausedSource] = useState<string | null>(null);
  const image = useRef<HTMLImageElement>(null);
  const localSource = content.data.attachment.thumbnail_key ?? content.data.attachment.storage_key;
  const source = /^(blob:|data:|https?:\/\/)/u.test(localSource) ? localSource : remoteSource;
  const canAnimate = ['image/gif', 'image/webp'].includes(content.data.attachment.mime_type);

  useEffect(() => {
    if (/^(blob:|data:|https?:\/\/)/u.test(localSource)) return;
    let cancelled = false;
    void api
      .attachmentDownloadUrl(content.data.attachment.id)
      .then((url) => {
        if (!cancelled) setRemoteSource(url);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [content.data.attachment.id, localSource]);

  useEffect(() => setPausedSource(null), [source]);

  function toggleAnimation() {
    if (pausedSource) {
      setPausedSource(null);
      return;
    }
    const current = image.current;
    if (!current?.naturalWidth || !current.naturalHeight) return;
    try {
      const canvas = document.createElement('canvas');
      canvas.width = current.naturalWidth;
      canvas.height = current.naturalHeight;
      const context = canvas.getContext('2d');
      if (!context) return;
      context.drawImage(current, 0, 0);
      setPausedSource(canvas.toDataURL('image/png'));
    } catch {
      setFailed(true);
    }
  }

  if (failed || !source || !/^(blob:|data:|https?:\/\/)/u.test(source)) {
    return (
      <div className="image-placeholder">
        <ImageOff size={22} />
        <span>{content.data.attachment.file_name}</span>
      </div>
    );
  }
  return (
    <>
      <div className="message-image-wrap">
        <button
          className="message-image"
          type="button"
          aria-label={tr(`查看图片 ${content.data.attachment.file_name}`)}
          onClick={() => setViewerOpen(true)}
        >
          <img
            ref={image}
            src={pausedSource ?? source}
            alt={content.data.attachment.file_name}
            loading="lazy"
            decoding="async"
            onError={() => setFailed(true)}
          />
        </button>
        {canAnimate ? (
          <button
            className="animation-toggle"
            type="button"
            aria-label={pausedSource ? tr('播放动态图片') : tr('暂停动态图片')}
            title={pausedSource ? tr('播放动态图片') : tr('暂停动态图片')}
            onClick={toggleAnimation}
          >
            {pausedSource ? <Play size={15} /> : <Pause size={15} />}
          </button>
        ) : null}
      </div>
      <ImageViewer
        open={viewerOpen}
        onOpenChange={setViewerOpen}
        initial={content.data.attachment}
        gallery={gallery.length ? gallery : [content.data.attachment]}
      />
    </>
  );
}

function ImageViewer({
  open,
  onOpenChange,
  initial,
  gallery,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initial: Attachment;
  gallery: Attachment[];
}) {
  const initialIndex = Math.max(
    0,
    gallery.findIndex((attachment) => attachment.id === initial.id),
  );
  const [index, setIndex] = useState(initialIndex);
  const [scale, setScale] = useState(1);
  const [rotation, setRotation] = useState(0);
  const attachment = gallery[index] ?? initial;
  const source = useAttachmentSource(attachment, open);

  useEffect(() => {
    if (!open) return;
    setIndex(initialIndex);
    setScale(1);
    setRotation(0);
  }, [initialIndex, open]);

  function move(direction: number) {
    setIndex((current) => (current + direction + gallery.length) % gallery.length);
    setScale(1);
    setRotation(0);
  }

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="image-viewer-overlay" />
        <Dialog.Content className="image-viewer" aria-describedby={undefined}>
          <Dialog.Title className="sr-only">
            {tr('查看图片')} {attachment.file_name}
          </Dialog.Title>
          <div className="image-viewer__toolbar">
            <span>
              {attachment.file_name} · {index + 1}/{gallery.length}
            </span>
            <IconButton
              label={tr('缩小')}
              onClick={() => setScale((value) => Math.max(0.25, value - 0.25))}
            >
              <ZoomOut size={18} />
            </IconButton>
            <IconButton
              label={tr('放大')}
              onClick={() => setScale((value) => Math.min(4, value + 0.25))}
            >
              <ZoomIn size={18} />
            </IconButton>
            <IconButton label={tr('顺时针旋转')} onClick={() => setRotation((value) => value + 90)}>
              <RotateCw size={18} />
            </IconButton>
            {source ? (
              <a
                className="icon-button"
                aria-label={tr('保存图片')}
                title={tr('保存图片')}
                href={source}
                download={attachment.file_name}
              >
                <Download size={18} />
              </a>
            ) : null}
            <Dialog.Close asChild>
              <IconButton label={tr('关闭图片查看器')}>
                <X size={19} />
              </IconButton>
            </Dialog.Close>
          </div>
          <div className="image-viewer__stage">
            {gallery.length > 1 ? (
              <IconButton label={tr('上一张')} onClick={() => move(-1)}>
                <ChevronLeft size={28} />
              </IconButton>
            ) : null}
            {source ? (
              <img
                src={source}
                alt={attachment.file_name}
                style={{ transform: `scale(${scale}) rotate(${rotation}deg)` }}
              />
            ) : (
              <div className="image-viewer__loading">{tr('正在加载原图…')}</div>
            )}
            {gallery.length > 1 ? (
              <IconButton label={tr('下一张')} onClick={() => move(1)}>
                <ChevronRight size={28} />
              </IconButton>
            ) : null}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function useAttachmentSource(attachment: Attachment, enabled: boolean): string | null {
  const localSource = attachment.storage_key;
  const [remoteSource, setRemoteSource] = useState<string | null>(null);

  useEffect(() => {
    setRemoteSource(null);
    if (!enabled || /^(blob:|data:|https?:\/\/)/u.test(localSource)) return;
    let cancelled = false;
    void api
      .attachmentDownloadUrl(attachment.id)
      .then((url) => {
        if (!cancelled) setRemoteSource(url);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [attachment.id, enabled, localSource]);

  return /^(blob:|data:|https?:\/\/)/u.test(localSource) ? localSource : remoteSource;
}

function FileMessage({ attachment }: { attachment: Attachment }) {
  const downloadDirectory = useChatStore((state) => state.settings.downloadDirectory);
  const [downloading, setDownloading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [progress, setProgress] = useState(0);
  const [downloadedPath, setDownloadedPath] = useState<string | null>(null);

  async function download() {
    if (downloading) return;
    setDownloading(true);
    setFailed(false);
    setProgress(0);
    try {
      const result = await api.downloadAttachment(attachment, downloadDirectory, setProgress);
      setDownloadedPath(result.path);
    } catch {
      setFailed(true);
    } finally {
      setDownloading(false);
    }
  }

  return (
    <div className="file-message">
      <span className="file-message__icon">
        <File size={22} />
      </span>
      <span className="file-message__info">
        <strong title={attachment.file_name}>{attachment.file_name}</strong>
        <small>
          {formatFileSize(attachment.byte_size)}
          {downloading ? ` · ${progress}%` : ''}
          {failed ? tr(' · 下载失败') : ''}
          {downloadedPath ? tr(' · 已保存') : ''}
        </small>
        {downloading ? (
          <span className="upload-progress" aria-label={tr(`下载进度 ${progress}%`)}>
            <i style={{ width: `${progress}%` }} />
          </span>
        ) : null}
      </span>
      {downloadedPath ? (
        <button
          type="button"
          aria-label={tr(`打开 ${attachment.file_name} 所在文件夹`)}
          onClick={() =>
            void api.revealDownload(downloadedPath, downloadDirectory).catch(() => setFailed(true))
          }
        >
          <FolderOpen size={18} />
        </button>
      ) : (
        <button
          type="button"
          disabled={downloading}
          aria-label={tr(`下载 ${attachment.file_name}`)}
          onClick={() => void download()}
        >
          <Download size={18} />
        </button>
      )}
    </div>
  );
}
