import type { MessageContent, Presence } from './types';
import { currentLanguage, tr } from './i18n';

export function formatConversationTime(value: string): string {
  const date = new Date(value);
  const now = new Date();
  if (Number.isNaN(date.getTime())) return '';
  if (date.toDateString() === now.toDateString()) {
    return new Intl.DateTimeFormat(currentLanguage(), {
      hour: '2-digit',
      minute: '2-digit',
    }).format(date);
  }
  const sameYear = date.getFullYear() === now.getFullYear();
  return new Intl.DateTimeFormat(
    currentLanguage(),
    sameYear
      ? { month: '2-digit', day: '2-digit' }
      : {
          year: 'numeric',
          month: '2-digit',
          day: '2-digit',
        },
  ).format(date);
}

export function formatFullTime(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? ''
    : new Intl.DateTimeFormat(currentLanguage(), {
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      }).format(date);
}

export function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}

export function messageSummary(content: MessageContent): string {
  switch (content.type) {
    case 'text':
      return content.data.text.replace(/\s+/g, ' ').trim();
    case 'image':
      return tr('[图片]');
    case 'file':
      return tr(`[文件] ${content.data.attachment.file_name}`);
    case 'audio':
      return tr('[语音]');
    case 'sticker':
      return tr(`[表情] ${content.data.name}`);
    case 'forward_bundle':
      return tr(`[聊天记录] ${content.data.title}`);
    case 'system':
      return content.data.text;
  }
}

export function presenceLabel(presence: Presence): string {
  return {
    online: tr('在线'),
    away: tr('离开'),
    busy: tr('忙碌'),
    invisible: tr('隐身'),
    offline: tr('离线'),
  }[presence];
}

export function initials(name: string): string {
  return Array.from(name.trim()).slice(0, 2).join('').toUpperCase() || '?';
}

export function safeHttpUrl(value: string): URL | null {
  try {
    const url = new URL(value);
    return url.protocol === 'http:' || url.protocol === 'https:' ? url : null;
  } catch {
    return null;
  }
}

export function splitLinks(text: string): Array<{ value: string; href: string | null }> {
  const matcher = /https?:\/\/[^\s<>{}[\]"]+/giu;
  const parts: Array<{ value: string; href: string | null }> = [];
  let position = 0;
  for (const match of text.matchAll(matcher)) {
    const index = match.index;
    if (index > position) parts.push({ value: text.slice(position, index), href: null });
    const value = match[0];
    parts.push({ value, href: safeHttpUrl(value)?.href ?? null });
    position = index + value.length;
  }
  if (position < text.length) parts.push({ value: text.slice(position), href: null });
  return parts;
}
