import { api, apiBaseUrl } from './api';
import type { ConnectionState, SyncEvent } from './types';

interface ServerEventFrame {
  type: 'event';
  event: SyncEvent;
}

export type CallSignal =
  | { kind: 'invite'; video: boolean }
  | { kind: 'accept' }
  | { kind: 'offer'; sdp: string }
  | { kind: 'answer'; sdp: string }
  | {
      kind: 'ice_candidate';
      candidate: string;
      sdp_mid: string | null;
      sdp_mline_index: number | null;
    }
  | { kind: 'participants'; user_ids: string[] }
  | { kind: 'reject' }
  | { kind: 'busy' }
  | { kind: 'hangup' };

export interface CallSignalFrame {
  conversation_id: string;
  call_id: string;
  from_user_id: string;
  signal: CallSignal;
}

type ServerFrame =
  | ServerEventFrame
  | { type: 'welcome'; protocol_version: number; latest_cursor: number }
  | {
      type: 'typing';
      conversation_id: string;
      user_id: string;
      active: boolean;
      expires_at: string;
    }
  | ({ type: 'call_signal' } & CallSignalFrame)
  | { type: 'pong'; nonce: string }
  | { type: 'close'; code: number; reason: string }
  | { type: 'error' };

export class RealtimeClient {
  private socket: WebSocket | null = null;
  private stopped = false;
  private retry = 0;
  private heartbeat: number | null = null;
  private reconnectTimer: number | null = null;
  private lastPongAt = 0;
  private connecting = false;

  constructor(
    private readonly cursor: () => number,
    private readonly onEvent: (event: SyncEvent) => void,
    private readonly onState: (state: ConnectionState) => void,
    private readonly onTyping: (
      conversationId: string,
      userId: string,
      active: boolean,
      expiresAt: string,
    ) => void,
  ) {}

  start(): void {
    this.stopped = false;
    activeTypingSender = this.typingSender;
    activeCallSender = this.callSender;
    window.addEventListener('online', this.handleOnline);
    window.addEventListener('offline', this.handleOffline);
    void this.connect();
  }

  stop(): void {
    this.stopped = true;
    this.clearHeartbeat();
    this.clearReconnect();
    window.removeEventListener('online', this.handleOnline);
    window.removeEventListener('offline', this.handleOffline);
    this.socket?.close(1000, 'client shutdown');
    this.socket = null;
    this.connecting = false;
    if (activeTypingSender === this.typingSender) activeTypingSender = null;
    if (activeCallSender === this.callSender) activeCallSender = null;
  }

  sendTyping(conversationId: string, active: boolean): void {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) return;
    this.socket.send(JSON.stringify({ type: 'typing', conversation_id: conversationId, active }));
  }

  private async connect(): Promise<void> {
    if (this.stopped || this.connecting || this.socket) return;
    if (!navigator.onLine) {
      this.onState('offline');
      return;
    }
    this.connecting = true;
    this.onState('connecting');
    try {
      const ticket = await api.websocketTicket();
      if (this.stopped) return;
      const url = new URL('/api/v1/ws', apiBaseUrl);
      url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
      url.searchParams.set('ticket', ticket);
      const socket = new WebSocket(url);
      this.socket = socket;
      socket.addEventListener('open', () => {
        this.connecting = false;
        this.retry = 0;
        this.lastPongAt = Date.now();
        this.onState('online');
        socket.send(
          JSON.stringify({
            type: 'hello',
            protocol_version: 1,
            client_version: '0.1.0',
            access_token: '',
            last_cursor: this.cursor(),
          }),
        );
        this.startHeartbeat();
      });
      socket.addEventListener('message', (event) => this.handleFrame(String(event.data)));
      socket.addEventListener('close', () => {
        if (this.socket === socket) this.socket = null;
        this.connecting = false;
        this.reconnect();
      });
      socket.addEventListener('error', () => socket.close());
    } catch {
      this.connecting = false;
      this.reconnect();
    }
  }

  private handleFrame(raw: string): void {
    let frame: ServerFrame;
    try {
      frame = JSON.parse(raw) as ServerFrame;
    } catch {
      return;
    }
    if (frame.type === 'event') this.onEvent(frame.event);
    if (frame.type === 'typing') {
      this.onTyping(frame.conversation_id, frame.user_id, frame.active, frame.expires_at);
    }
    if (frame.type === 'call_signal') {
      callListeners.forEach((listener) => listener(frame));
    }
    if (frame.type === 'pong') this.lastPongAt = Date.now();
    if (frame.type === 'welcome') {
      if (frame.protocol_version !== 1) {
        this.onState('failed');
        this.socket?.close(4002, 'protocol mismatch');
      } else if (frame.latest_cursor > this.cursor()) {
        this.onState('syncing');
        void this.catchUp()
          .then(() => this.onState('online'))
          .catch(() => this.onState('failed'));
      }
    }
  }

  private async catchUp(): Promise<void> {
    let hasMore = true;
    while (hasMore && !this.stopped) {
      const response = await api.sync(this.cursor());
      response.events.forEach(this.onEvent);
      hasMore = response.has_more;
    }
  }

  private startHeartbeat(): void {
    this.clearHeartbeat();
    this.heartbeat = window.setInterval(() => {
      if (!this.socket || this.socket.readyState !== WebSocket.OPEN) return;
      if (Date.now() - this.lastPongAt > 45_000) {
        this.socket.close(4000, 'heartbeat timeout');
        return;
      }
      this.socket.send(JSON.stringify({ type: 'ping', nonce: crypto.randomUUID() }));
    }, 20_000);
  }

  private reconnect(): void {
    this.clearHeartbeat();
    if (this.stopped) return;
    this.onState('offline');
    const base = Math.min(30_000, 800 * 2 ** Math.min(this.retry, 6));
    const delay = base * (0.75 + Math.random() * 0.5);
    this.retry += 1;
    this.clearReconnect();
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null;
      void this.connect();
    }, delay);
  }

  private clearHeartbeat(): void {
    if (this.heartbeat !== null) window.clearInterval(this.heartbeat);
    this.heartbeat = null;
  }

  private clearReconnect(): void {
    if (this.reconnectTimer !== null) window.clearTimeout(this.reconnectTimer);
    this.reconnectTimer = null;
  }

  private readonly handleOnline = () => {
    this.clearReconnect();
    void this.connect();
  };

  private readonly handleOffline = () => {
    this.clearReconnect();
    this.socket?.close(4001, 'network offline');
    this.onState('offline');
  };

  private readonly typingSender = (conversationId: string, active: boolean) => {
    this.sendTyping(conversationId, active);
  };

  private readonly callSender = (
    conversationId: string,
    callId: string,
    signal: CallSignal,
    targetUserId?: string,
  ) => {
    if (!this.socket || this.socket.readyState !== WebSocket.OPEN) return false;
    this.socket.send(
      JSON.stringify({
        type: 'call_signal',
        conversation_id: conversationId,
        call_id: callId,
        target_user_id: targetUserId ?? null,
        signal,
      }),
    );
    return true;
  };
}

let activeTypingSender: ((conversationId: string, active: boolean) => void) | null = null;
let activeCallSender:
  | ((conversationId: string, callId: string, signal: CallSignal, targetUserId?: string) => boolean)
  | null = null;
const callListeners = new Set<(frame: CallSignalFrame) => void>();
const callStartListeners = new Set<(conversationId: string, video: boolean) => void>();

export function sendTyping(conversationId: string, active: boolean): void {
  activeTypingSender?.(conversationId, active);
}

export function sendCallSignal(
  conversationId: string,
  callId: string,
  signal: CallSignal,
  targetUserId?: string,
): boolean {
  return activeCallSender?.(conversationId, callId, signal, targetUserId) ?? false;
}

export function subscribeCallSignals(listener: (frame: CallSignalFrame) => void): () => void {
  callListeners.add(listener);
  return () => callListeners.delete(listener);
}

export function startCall(conversationId: string, video: boolean): void {
  callStartListeners.forEach((listener) => listener(conversationId, video));
}

export function subscribeCallStarts(
  listener: (conversationId: string, video: boolean) => void,
): () => void {
  callStartListeners.add(listener);
  return () => callStartListeners.delete(listener);
}
