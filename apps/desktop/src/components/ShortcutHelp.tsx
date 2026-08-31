import * as Dialog from '@radix-ui/react-dialog';
import { X } from 'lucide-react';

import { IconButton } from './IconButton';
import { tr } from '../lib/i18n';

interface ShortcutHelpProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

const shortcuts: ReadonlyArray<readonly [string, string]> = [
  ['⌘/Ctrl + K', '全局搜索'],
  ['⌘/Ctrl + ,', '打开设置'],
  ['⌘/Ctrl + 1', '会话'],
  ['⌘/Ctrl + 2', '联系人'],
  ['Alt + ↑ / ↓', '切换会话'],
  ['Enter', '发送消息'],
  ['Shift + Enter', '消息换行'],
  ['Escape', '关闭弹窗或菜单'],
];

export function ShortcutHelp({ open, onOpenChange }: ShortcutHelpProps) {
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog-content" aria-describedby="shortcut-description">
          <div className="dialog-title-row">
            <Dialog.Title>{tr('快捷键')}</Dialog.Title>
            <Dialog.Close asChild>
              <IconButton label={tr('关闭快捷键帮助')}>
                <X size={18} />
              </IconButton>
            </Dialog.Close>
          </div>
          <Dialog.Description id="shortcut-description">
            {tr('核心操作无需鼠标也可以完成。')}
          </Dialog.Description>
          <dl className="shortcut-list">
            {shortcuts.map(([keys, label]) => (
              <div key={keys}>
                <dt>
                  <kbd>{keys}</kbd>
                </dt>
                <dd>{tr(label)}</dd>
              </div>
            ))}
          </dl>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
