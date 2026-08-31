import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  isPermissionGranted,
  onAction,
  requestPermission,
  sendNotification,
} from '@tauri-apps/plugin-notification';
import { tr } from './i18n';

const isTauri = (): boolean => '__TAURI_INTERNALS__' in window;
const groups = new Map<
  string,
  {
    count: number;
    title: string;
    body: string;
    sound: boolean;
    lastAt: number;
    timer: number;
  }
>();
let openConversation: ((conversationId: string) => void) | null = null;

export interface NotificationMessage {
  conversationId: string;
  title: string;
  body: string;
  sound: boolean;
}

export function notify(message: NotificationMessage): void {
  const previous = groups.get(message.conversationId);
  const now = Date.now();
  const count = previous && now - previous.lastAt < 10_000 ? previous.count + 1 : 1;
  if (previous) window.clearTimeout(previous.timer);
  const timer = window.setTimeout(() => {
    const group = groups.get(message.conversationId);
    if (group) void deliver(message.conversationId, group);
  }, 500);
  groups.set(message.conversationId, {
    count,
    title: message.title,
    body: message.body,
    sound: message.sound,
    lastAt: now,
    timer,
  });
}

export async function setupNotificationInteractions(
  handler: (conversationId: string) => void,
): Promise<() => void> {
  openConversation = handler;
  if (!isTauri()) {
    return () => {
      if (openConversation === handler) openConversation = null;
    };
  }
  const listener = await onAction((notification) => {
    const conversationId = notification.extra?.conversationId;
    if (typeof conversationId === 'string') void activateConversation(conversationId);
  });
  return () => {
    void listener.unregister();
    if (openConversation === handler) openConversation = null;
  };
}

async function deliver(
  conversationId: string,
  group: {
    count: number;
    title: string;
    body: string;
    sound: boolean;
  },
): Promise<void> {
  const title = group.count > 1 ? tr(`${group.title}（${group.count} 条新消息）`) : group.title;
  if (isTauri()) {
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === 'granted';
    if (!granted) return;
    sendNotification({
      id: stableNotificationId(conversationId),
      title,
      body: group.body,
      group: conversationId,
      number: group.count,
      autoCancel: true,
      ...(group.sound ? { sound: 'message-new-instant' } : { silent: true }),
      extra: { conversationId },
    });
    return;
  }
  if (!('Notification' in window)) return;
  let permission = Notification.permission;
  if (permission === 'default') permission = await Notification.requestPermission();
  if (permission !== 'granted') return;
  const notification = new Notification(title, {
    body: group.body,
    tag: `iamrust-${conversationId}`,
    silent: !group.sound,
  });
  notification.onclick = () => {
    notification.close();
    window.focus();
    openConversation?.(conversationId);
  };
}

async function activateConversation(conversationId: string): Promise<void> {
  if (isTauri()) {
    const appWindow = getCurrentWindow();
    await appWindow.unminimize().catch(() => undefined);
    await appWindow.show().catch(() => undefined);
    await appWindow.setFocus().catch(() => undefined);
  } else {
    window.focus();
  }
  openConversation?.(conversationId);
}

function stableNotificationId(value: string): number {
  let hash = 2_166_136_261;
  for (const character of value) {
    hash ^= character.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 16_777_619);
  }
  return hash >>> 0;
}
