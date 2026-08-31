import { useCallback, useEffect, useRef } from 'react';

import { AppShell } from './components/AppShell';
import { LoadingScreen } from './components/LoadingScreen';
import { AuthScreen } from './features/auth/AuthScreen';
import { api } from './lib/api';
import { messageSummary } from './lib/format';
import {
  readAutostart,
  subscribeGlobalShortcut,
  subscribeNotificationMute,
} from './lib/desktop-plugins';
import {
  acknowledgeOutbox,
  cacheBootstrap,
  loadCachedBootstrap,
  readyOutbox,
  updateCloseBehavior,
  updateTrayUnread,
  type OutboxPayload,
} from './lib/local-cache';
import { notify, setupNotificationInteractions } from './lib/notifications';
import { startCrashReporting } from './lib/crash-reporting';
import { RealtimeClient } from './lib/realtime';
import type { AppSettings, Message, SyncEvent } from './lib/types';
import { useChatStore, userById } from './state/chat-store';
import { tr } from './lib/i18n';

const windowParams = new URLSearchParams(window.location.search);
const detachedConversationId =
  windowParams.get('detached') === '1' ? windowParams.get('conversation') : null;

export default function App() {
  const auth = useChatStore((state) => state.auth);
  const demo = useChatStore((state) => state.demo);
  const settings = useChatStore((state) => state.settings);
  const setAuth = useChatStore((state) => state.setAuth);
  const setBootstrap = useChatStore((state) => state.setBootstrap);
  const setConnection = useChatStore((state) => state.setConnection);
  const applyEvent = useChatStore((state) => state.applyEvent);
  const setSection = useChatStore((state) => state.setSection);
  const realtimeRef = useRef<RealtimeClient | null>(null);
  const bootstrapRefreshRef = useRef<Promise<void> | null>(null);

  useEffect(() => {
    let active = true;
    void (async () => {
      const cached = await loadCachedBootstrap().catch(() => null);
      try {
        const session = await api.restore();
        if (!active) return;
        if (!session) {
          setAuth('unauthenticated');
          return;
        }
        const bootstrap = await api.bootstrap();
        if (active) {
          setBootstrap(bootstrap);
          void cacheBootstrap(bootstrap);
        }
      } catch {
        if (!active) return;
        if (cached) {
          setBootstrap(cached);
          setConnection('offline');
        } else {
          setAuth('unauthenticated');
        }
      }
    })();
    return () => {
      active = false;
    };
  }, [setAuth, setBootstrap]);

  const handleEvent = useCallback(
    (event: SyncEvent) => {
      const before = useChatStore.getState();
      applyEvent(event);
      if (
        (event.kind === 'friendship_updated' || event.kind === 'group_membership_updated') &&
        !before.demo &&
        !bootstrapRefreshRef.current
      ) {
        bootstrapRefreshRef.current = api
          .bootstrap()
          .then((bootstrap) => {
            setBootstrap(bootstrap);
            return cacheBootstrap(bootstrap);
          })
          .catch(() => undefined)
          .finally(() => {
            bootstrapRefreshRef.current = null;
          });
      }
      if (event.kind !== 'message_created') return;
      const message = event.payload.message as Message | undefined;
      if (!message || message.sender_id === before.me?.id) return;
      if (
        document.visibilityState === 'visible' &&
        before.selectedConversationId === message.conversation_id
      )
        return;
      if (!before.settings.notifications) return;
      const conversation = before.conversations.find((item) => item.id === message.conversation_id);
      if (conversation?.muted) return;
      const sender = userById(before, message.sender_id);
      const title = before.settings.privacyMode
        ? tr('I Am Rust 新消息')
        : (sender?.nickname ?? tr('新消息'));
      const body =
        before.settings.privacyMode || !before.settings.notificationPreview
          ? tr('你收到了一条新消息')
          : messageSummary(message.content);
      if (isDoNotDisturbActive(before.settings, new Date())) return;
      notify({
        conversationId: message.conversation_id,
        title,
        body,
        sound: before.settings.notificationSound,
      });
    },
    [applyEvent, setBootstrap],
  );

  const startRealtime = useCallback(() => {
    realtimeRef.current?.stop();
    const client = new RealtimeClient(
      () => useChatStore.getState().cursor,
      handleEvent,
      setConnection,
      (conversationId, userId, active, expiresAt) =>
        useChatStore.getState().setTyping(conversationId, userId, active, expiresAt),
    );
    realtimeRef.current = client;
    client.start();
    void flushOutbox();
  }, [handleEvent, setConnection]);

  useEffect(() => {
    if (auth !== 'authenticated' || demo) return;
    startRealtime();
    return () => {
      realtimeRef.current?.stop();
      realtimeRef.current = null;
    };
  }, [auth, demo, startRealtime]);

  useEffect(() => {
    let active = true;
    let dispose: () => void = () => undefined;
    void setupNotificationInteractions((conversationId) => {
      const state = useChatStore.getState();
      if (!state.conversations.some((conversation) => conversation.id === conversationId)) return;
      state.selectConversation(conversationId);
      state.setSection('conversations');
    }).then((nextDispose) => {
      if (active) dispose = nextDispose;
      else nextDispose();
    });
    return () => {
      active = false;
      dispose();
    };
  }, []);

  useEffect(() => {
    let active = true;
    let dispose: () => void = () => undefined;
    void subscribeGlobalShortcut(settings.globalShortcutEnabled, () => {
      useChatStore.getState().setSection('conversations');
    })
      .then((nextDispose) => {
        if (active) dispose = nextDispose;
        else nextDispose();
      })
      .catch(() => useChatStore.getState().setAnnouncement(tr('全局快捷键注册失败。')));
    return () => {
      active = false;
      dispose();
    };
  }, [settings.globalShortcutEnabled]);

  useEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = settings.theme;
    root.dataset.compact = String(settings.compactMode);
    root.lang = settings.language;
    root.style.setProperty('--font-scale', String(settings.fontScale));
  }, [settings.compactMode, settings.fontScale, settings.language, settings.theme]);

  useEffect(() => startCrashReporting(settings.crashReporting), [settings.crashReporting]);

  useEffect(() => {
    if (auth !== 'authenticated') return;
    void readAutostart()
      .then((value) => {
        if (value !== null) useChatStore.getState().updateSettings({ autostart: value });
      })
      .catch(() => undefined);
  }, [auth]);

  useEffect(() => {
    if (auth !== 'authenticated' || !detachedConversationId) return;
    const state = useChatStore.getState();
    if (state.conversations.some((conversation) => conversation.id === detachedConversationId)) {
      state.selectConversation(detachedConversationId);
    }
  }, [auth]);

  useEffect(() => {
    let active = true;
    let dispose: () => void = () => undefined;
    void subscribeNotificationMute((muted) =>
      useChatStore.getState().updateSettings({ notifications: !muted }),
    ).then((nextDispose) => {
      if (active) dispose = nextDispose;
      else nextDispose();
    });
    return () => {
      active = false;
      dispose();
    };
  }, []);

  useEffect(() => {
    const unsubscribe = useChatStore.subscribe((state, previous) => {
      const total = Object.values(state.meta).reduce((sum, item) => sum + item.unread, 0);
      const previousTotal = Object.values(previous.meta).reduce(
        (sum, item) => sum + item.unread,
        0,
      );
      if (total !== previousTotal) void updateTrayUnread(total);
      if (state.settings.closeBehavior !== previous.settings.closeBehavior) {
        void updateCloseBehavior(state.settings.closeBehavior === 'tray');
      }
    });
    void updateCloseBehavior(settings.closeBehavior === 'tray');
    return unsubscribe;
  }, [settings.closeBehavior]);

  useEffect(() => {
    function keydown(event: KeyboardEvent) {
      const mod = event.ctrlKey || event.metaKey;
      if (mod && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        setSection('search');
      } else if (mod && event.key === ',') {
        event.preventDefault();
        setSection('settings');
      } else if (mod && event.key === '1') {
        event.preventDefault();
        setSection('conversations');
      } else if (mod && event.key === '2') {
        event.preventDefault();
        setSection('contacts');
      }
    }
    window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [setSection]);

  if (auth === 'restoring') return <LoadingScreen />;
  if (auth === 'unauthenticated') return <AuthScreen />;
  return <AppShell detached={Boolean(detachedConversationId)} onReconnect={startRealtime} />;
}

async function flushOutbox(): Promise<void> {
  const items = await readyOutbox().catch(() => []);
  for (const item of items) {
    try {
      const payload = JSON.parse(item.payload_json) as OutboxPayload;
      const ack = await api.sendMessage(
        item.conversation_id,
        item.client_message_id,
        payload.content,
        payload.reply_to,
        payload.expires_in_seconds ?? null,
        payload.mentions ?? [],
        payload.mention_all ?? false,
      );
      useChatStore.getState().resolveMessage(item.client_message_id, {
        id: ack.message_id,
        sequence: ack.sequence,
        server_created_at: ack.server_time,
        status: 'sent',
      });
      await acknowledgeOutbox(item.client_message_id);
    } catch {
      useChatStore.getState().failMessage(item.client_message_id);
      break;
    }
  }
}

function isDoNotDisturbActive(settings: AppSettings, now: Date): boolean {
  if (!settings.doNotDisturbEnabled) return false;
  const minutes = now.getHours() * 60 + now.getMinutes();
  const start = clockMinutes(settings.doNotDisturbStart);
  const end = clockMinutes(settings.doNotDisturbEnd);
  if (start === end) return true;
  return start < end ? minutes >= start && minutes < end : minutes >= start || minutes < end;
}

function clockMinutes(value: string): number {
  const [hours = 0, minutes = 0] = value.split(':').map(Number);
  return hours * 60 + minutes;
}
