import { disable, enable, isEnabled } from '@tauri-apps/plugin-autostart';
import { listen } from '@tauri-apps/api/event';
import { relaunch } from '@tauri-apps/plugin-process';
import { check } from '@tauri-apps/plugin-updater';
import { open } from '@tauri-apps/plugin-dialog';
import { register, unregister } from '@tauri-apps/plugin-global-shortcut';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';

const isTauri = (): boolean => '__TAURI_INTERNALS__' in window;

export async function readAutostart(): Promise<boolean | null> {
  return isTauri() ? isEnabled() : null;
}

export async function writeAutostart(value: boolean): Promise<void> {
  if (!isTauri()) return;
  if (value) await enable();
  else await disable();
}

export async function checkForUpdates(): Promise<'current' | 'installed'> {
  if (!isTauri()) return 'current';
  const update = await check();
  if (!update) return 'current';
  await update.downloadAndInstall();
  await relaunch();
  return 'installed';
}

export async function chooseDownloadDirectory(): Promise<string | null> {
  if (!isTauri()) return null;
  const selected = await open({ directory: true, multiple: false });
  return typeof selected === 'string' ? selected : null;
}

export async function subscribeNotificationMute(
  handler: (muted: boolean) => void,
): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  return listen<boolean>('notification-muted', (event) => handler(event.payload));
}

export async function subscribeGlobalShortcut(
  enabled: boolean,
  handler: () => void,
): Promise<() => void> {
  if (!isTauri() || !enabled) return () => undefined;
  const shortcut = 'CommandOrControl+Shift+I';
  await unregister(shortcut).catch(() => undefined);
  await register(shortcut, (event) => {
    if (event.state !== 'Pressed') return;
    const window = getCurrentWindow();
    void window.unminimize().catch(() => undefined);
    void window.show().catch(() => undefined);
    void window.setFocus().catch(() => undefined);
    handler();
  });
  return () => {
    void unregister(shortcut).catch(() => undefined);
  };
}

export async function openConversationWindow(
  conversationId: string,
  title: string,
): Promise<boolean> {
  if (!isTauri()) return false;
  const label = `chat-${conversationId.replaceAll(/[^a-zA-Z0-9-]/g, '')}`;
  const existing = await WebviewWindow.getByLabel(label);
  if (existing) {
    await existing.unminimize().catch(() => undefined);
    await existing.show();
    await existing.setFocus();
    return true;
  }
  const window = new WebviewWindow(label, {
    url: `/?detached=1&conversation=${encodeURIComponent(conversationId)}`,
    title: `${title} · I Am Rust`,
    width: 820,
    height: 680,
    minWidth: 560,
    minHeight: 480,
    center: true,
    resizable: true,
  });
  return new Promise((resolve, reject) => {
    void window.once('tauri://created', () => resolve(true));
    void window.once('tauri://error', (event) => reject(new Error(String(event.payload))));
  });
}
