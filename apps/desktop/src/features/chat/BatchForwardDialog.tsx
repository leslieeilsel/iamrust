import * as Dialog from '@radix-ui/react-dialog';
import { Forward, Layers3, List, X } from 'lucide-react';
import { useEffect, useState } from 'react';

import { IconButton } from '../../components/IconButton';
import { api } from '../../lib/api';
import type { MessageId } from '../../lib/types';
import { conversationName, useChatStore } from '../../state/chat-store';
import { tr } from '../../lib/i18n';

export function BatchForwardDialog({
  messageIds,
  sourceConversationId,
  open,
  onOpenChange,
  onComplete,
}: {
  messageIds: MessageId[];
  sourceConversationId: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onComplete: () => void;
}) {
  const conversations = useChatStore((state) => state.conversations);
  const friends = useChatStore((state) => state.friends);
  const friendSettings = useChatStore((state) => state.friendSettings);
  const demo = useChatStore((state) => state.demo);
  const setMessages = useChatStore((state) => state.setMessages);
  const setAnnouncement = useChatStore((state) => state.setAnnouncement);
  const [target, setTarget] = useState('');
  const [mode, setMode] = useState<'individually' | 'merged'>('individually');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    setTarget(
      conversations.find((conversation) => conversation.id !== sourceConversationId)?.id ?? '',
    );
    setMode(messageIds.length > 1 ? 'merged' : 'individually');
  }, [conversations, messageIds.length, open, sourceConversationId]);

  async function forwardMessages() {
    if (!target || !messageIds.length || busy) return;
    setBusy(true);
    try {
      if (demo) {
        setAnnouncement(tr('演示模式不会转发到其他会话。'));
      } else {
        const forwarded = await api.forwardMessages(messageIds, target, mode);
        setMessages(target, forwarded);
        setAnnouncement(mode === 'merged' ? tr('聊天记录已合并转发。') : tr('消息已逐条转发。'));
      }
      onOpenChange(false);
      onComplete();
    } catch {
      setAnnouncement(tr('批量转发失败，请确认仍可访问这些消息。'));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog.Root open={open} onOpenChange={(next) => !busy && onOpenChange(next)}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog-content batch-forward-dialog">
          <header className="dialog-header">
            <div>
              <Dialog.Title>
                {tr('转发')} {messageIds.length} {tr('条消息')}
              </Dialog.Title>
              <Dialog.Description>{tr('选择目标会话和转发方式。')}</Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <IconButton label={tr('关闭')} disabled={busy}>
                <X size={18} />
              </IconButton>
            </Dialog.Close>
          </header>
          <label>
            {tr('目标会话')}
            <select value={target} onChange={(event) => setTarget(event.target.value)}>
              <option value="">{tr('选择会话')}</option>
              {conversations
                .filter((conversation) => conversation.id !== sourceConversationId)
                .map((conversation) => (
                  <option key={conversation.id} value={conversation.id}>
                    {conversationName(conversation, friends, friendSettings)}
                  </option>
                ))}
            </select>
          </label>
          <fieldset className="forward-mode-options">
            <legend>{tr('转发方式')}</legend>
            <label>
              <input
                type="radio"
                name="forward-mode"
                value="individually"
                checked={mode === 'individually'}
                onChange={() => setMode('individually')}
              />
              <List size={16} />
              <span>
                <strong>{tr('逐条转发')}</strong>
                <small>{tr('每条消息单独发送')}</small>
              </span>
            </label>
            <label>
              <input
                type="radio"
                name="forward-mode"
                value="merged"
                checked={mode === 'merged'}
                disabled={messageIds.length < 2}
                onChange={() => setMode('merged')}
              />
              <Layers3 size={16} />
              <span>
                <strong>{tr('合并转发')}</strong>
                <small>{tr('折叠为一条聊天记录')}</small>
              </span>
            </label>
          </fieldset>
          <footer className="dialog-actions">
            <Dialog.Close asChild>
              <button className="secondary-button" type="button" disabled={busy}>
                {tr('取消')}
              </button>
            </Dialog.Close>
            <button
              className="primary-button"
              type="button"
              disabled={!target || !messageIds.length || busy}
              onClick={() => void forwardMessages()}
            >
              <Forward size={16} /> {busy ? tr('正在转发…') : tr('转发')}
            </button>
          </footer>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
