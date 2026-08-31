import * as Dialog from '@radix-ui/react-dialog';
import { AlertTriangle, X } from 'lucide-react';
import { useEffect, useState } from 'react';

import { IconButton } from './IconButton';
import { tr } from '../lib/i18n';

export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  danger = false,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description: string;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: () => Promise<boolean | void>;
}) {
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) setBusy(false);
  }, [open]);

  async function confirm() {
    if (busy) return;
    setBusy(true);
    try {
      const close = await onConfirm();
      if (close !== false) onOpenChange(false);
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog.Root open={open} onOpenChange={(next) => !busy && onOpenChange(next)}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog-content confirm-dialog">
          <header className="dialog-header">
            <div>
              <Dialog.Title>{title}</Dialog.Title>
              <Dialog.Description>{description}</Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <IconButton label={tr('取消')} disabled={busy}>
                <X size={18} />
              </IconButton>
            </Dialog.Close>
          </header>
          <div className="confirm-dialog__warning" aria-hidden="true">
            <AlertTriangle size={22} />
          </div>
          <footer className="dialog-actions">
            <Dialog.Close asChild>
              <button className="secondary-button" type="button" disabled={busy}>
                {tr('取消')}
              </button>
            </Dialog.Close>
            <button
              className={danger ? 'danger-button' : 'primary-button'}
              type="button"
              disabled={busy}
              onClick={() => void confirm()}
            >
              {busy ? tr('正在处理…') : confirmLabel}
            </button>
          </footer>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
