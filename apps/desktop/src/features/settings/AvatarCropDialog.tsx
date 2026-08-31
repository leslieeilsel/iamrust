import * as Dialog from '@radix-ui/react-dialog';
import { ImageUp, X } from 'lucide-react';
import { useEffect, useState } from 'react';

import { IconButton } from '../../components/IconButton';
import { tr } from '../../lib/i18n';

export function AvatarCropDialog({
  file,
  saving,
  progress,
  onCancel,
  onSave,
}: {
  file: File | null;
  saving: boolean;
  progress: number;
  onCancel: () => void;
  onSave: (blob: Blob) => Promise<void>;
}) {
  const [source, setSource] = useState('');
  const [zoom, setZoom] = useState(1);
  const [offsetX, setOffsetX] = useState(0);
  const [offsetY, setOffsetY] = useState(0);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!file) {
      setSource('');
      return;
    }
    const url = URL.createObjectURL(file);
    setSource(url);
    setZoom(1);
    setOffsetX(0);
    setOffsetY(0);
    setError('');
    return () => URL.revokeObjectURL(url);
  }, [file]);

  async function save() {
    if (!file || saving) return;
    try {
      setError('');
      await onSave(await cropAvatar(file, zoom, offsetX, offsetY));
    } catch {
      setError(tr('头像处理失败，请换一张图片重试。'));
    }
  }

  return (
    <Dialog.Root open={file !== null} onOpenChange={(open) => !open && !saving && onCancel()}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog-content avatar-crop-dialog">
          <header className="dialog-header">
            <div>
              <Dialog.Title>{tr('裁剪头像')}</Dialog.Title>
              <Dialog.Description>
                {tr('头像会压缩为 512 × 512，并移除原图元数据。')}
              </Dialog.Description>
            </div>
            <IconButton label={tr('关闭')} disabled={saving} onClick={onCancel}>
              <X size={18} />
            </IconButton>
          </header>
          <div className="avatar-crop-stage">
            {source ? (
              <img
                src={source}
                alt={tr('头像裁剪预览')}
                style={{
                  transform: `translate(${offsetX / 2}%, ${offsetY / 2}%) scale(${zoom})`,
                }}
              />
            ) : null}
            <span aria-hidden="true" />
          </div>
          <div className="avatar-crop-controls">
            <label>
              {tr('缩放')}
              <input
                type="range"
                min="1"
                max="3"
                step="0.05"
                value={zoom}
                onChange={(event) => setZoom(Number(event.target.value))}
              />
            </label>
            <label>
              {tr('水平位置')}
              <input
                type="range"
                min="-100"
                max="100"
                value={offsetX}
                onChange={(event) => setOffsetX(Number(event.target.value))}
              />
            </label>
            <label>
              {tr('垂直位置')}
              <input
                type="range"
                min="-100"
                max="100"
                value={offsetY}
                onChange={(event) => setOffsetY(Number(event.target.value))}
              />
            </label>
          </div>
          {saving ? (
            <div className="avatar-upload-progress" role="status">
              <span style={{ width: `${progress}%` }} />
              <small>
                {tr('正在上传')} {progress}%
              </small>
            </div>
          ) : null}
          {error ? <p className="field-error">{error}</p> : null}
          <footer className="dialog-actions">
            <button className="secondary-button" type="button" disabled={saving} onClick={onCancel}>
              {tr('取消')}
            </button>
            <button
              className="primary-button"
              type="button"
              disabled={saving}
              onClick={() => void save()}
            >
              <ImageUp size={16} /> {tr('使用此头像')}
            </button>
          </footer>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

async function cropAvatar(
  file: File,
  zoom: number,
  offsetX: number,
  offsetY: number,
): Promise<Blob> {
  const bitmap = await createImageBitmap(file, { imageOrientation: 'from-image' });
  const outputSize = 512;
  const canvas = document.createElement('canvas');
  canvas.width = outputSize;
  canvas.height = outputSize;
  const context = canvas.getContext('2d');
  if (!context) throw new Error('canvas unavailable');
  const baseScale = Math.max(outputSize / bitmap.width, outputSize / bitmap.height);
  const scale = baseScale * zoom;
  const width = bitmap.width * scale;
  const height = bitmap.height * scale;
  const spareX = Math.max(0, width - outputSize);
  const spareY = Math.max(0, height - outputSize);
  const x = (outputSize - width) / 2 + (offsetX / 100) * (spareX / 2);
  const y = (outputSize - height) / 2 + (offsetY / 100) * (spareY / 2);
  context.drawImage(bitmap, x, y, width, height);
  bitmap.close();
  const blob = await new Promise<Blob | null>((resolve) =>
    canvas.toBlob(resolve, 'image/webp', 0.86),
  );
  if (!blob) throw new Error('avatar encoding failed');
  return blob;
}
