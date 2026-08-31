import { useEffect, useState } from 'react';
import { UsersRound } from 'lucide-react';

import { initials } from '../lib/format';
import { api } from '../lib/api';

interface AvatarProps {
  name: string;
  src?: string | null | undefined;
  attachmentId?: string | null | undefined;
  size?: 'small' | 'medium' | 'large';
  group?: boolean;
  presence?: 'online' | 'away' | 'busy' | 'invisible' | 'offline' | undefined;
}

export function Avatar({
  name,
  src = null,
  attachmentId = null,
  size = 'medium',
  group = false,
  presence,
}: AvatarProps) {
  const [attachmentSource, setAttachmentSource] = useState<string | null>(null);

  useEffect(() => {
    setAttachmentSource(null);
    if (src || !attachmentId) return;
    let active = true;
    void api
      .attachmentDownloadUrl(attachmentId)
      .then((url) => {
        if (active) setAttachmentSource(url);
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, [attachmentId, src]);

  const source = src ?? attachmentSource;
  return (
    <span className={`avatar avatar--${size}`} aria-hidden="true">
      {source ? (
        <img src={source} alt="" draggable={false} />
      ) : group ? (
        <UsersRound size={size === 'large' ? 30 : 19} />
      ) : (
        <span>{initials(name)}</span>
      )}
      {presence && presence !== 'invisible' ? (
        <span className={`presence presence--${presence}`} />
      ) : null}
    </span>
  );
}
