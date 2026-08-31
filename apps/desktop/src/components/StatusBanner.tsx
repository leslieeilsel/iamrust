import { CloudOff, LoaderCircle, RefreshCw, TriangleAlert } from 'lucide-react';

import type { ConnectionState } from '../lib/types';
import { tr } from '../lib/i18n';

interface StatusBannerProps {
  state: ConnectionState;
  onRetry: () => void;
}

export function StatusBanner({ state, onRetry }: StatusBannerProps) {
  if (state === 'online') return null;
  const content = {
    connecting: { icon: <LoaderCircle className="spin" />, text: tr('正在连接…') },
    syncing: { icon: <RefreshCw className="spin" />, text: tr('正在同步离线消息…') },
    offline: { icon: <CloudOff />, text: tr('当前离线，消息将在恢复连接后发送') },
    failed: { icon: <TriangleAlert />, text: tr('同步失败，请重试') },
  }[state];
  return (
    <div className="status-banner" role="status">
      {content.icon}
      <span>{content.text}</span>
      {(state === 'offline' || state === 'failed') && (
        <button type="button" onClick={onRetry}>
          {tr('重试')}
        </button>
      )}
    </div>
  );
}
