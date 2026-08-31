const STATIC_IMAGE_TYPES = new Set(['image/png', 'image/jpeg', 'image/webp']);
const MAX_IMAGE_EDGE = 4096;

export async function prepareImageForUpload(file: File, sendOriginal: boolean): Promise<File> {
  if (sendOriginal || file.type === 'image/gif') return file;
  if (!STATIC_IMAGE_TYPES.has(file.type)) throw new Error('unsupported image type');

  if (file.type === 'image/webp' && (await isAnimatedWebp(file))) {
    return stripWebpMetadata(file);
  }

  const bitmap = await createImageBitmap(file, { imageOrientation: 'from-image' });
  try {
    const scale = Math.min(1, MAX_IMAGE_EDGE / Math.max(bitmap.width, bitmap.height));
    const width = Math.max(1, Math.round(bitmap.width * scale));
    const height = Math.max(1, Math.round(bitmap.height * scale));
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext('2d');
    if (!context) throw new Error('canvas unavailable');
    context.drawImage(bitmap, 0, 0, width, height);
    const blob = await new Promise<Blob | null>((resolve) =>
      canvas.toBlob(resolve, 'image/webp', 0.88),
    );
    if (!blob) throw new Error('image encoding failed');
    return new File([blob], replaceExtension(file.name, 'webp'), {
      type: 'image/webp',
      lastModified: file.lastModified,
    });
  } finally {
    bitmap.close();
  }
}

export async function hasSafeImageHeader(file: File): Promise<boolean> {
  const bytes = new Uint8Array(await file.slice(0, 16).arrayBuffer());
  const png = bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47;
  const jpeg = bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff;
  const gif = ascii(bytes, 0, 6) === 'GIF87a' || ascii(bytes, 0, 6) === 'GIF89a';
  const webp = ascii(bytes, 0, 4) === 'RIFF' && ascii(bytes, 8, 12) === 'WEBP';
  return png || jpeg || gif || webp;
}

async function isAnimatedWebp(file: File): Promise<boolean> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  return webpChunks(bytes).some((chunk) => chunk.kind === 'ANIM' || chunk.kind === 'ANMF');
}

async function stripWebpMetadata(file: File): Promise<File> {
  const source = new Uint8Array(await file.arrayBuffer());
  const kept = webpChunks(source).filter((chunk) => !['EXIF', 'XMP ', 'ICCP'].includes(chunk.kind));
  const size = 12 + kept.reduce((total, chunk) => total + chunk.bytes.length, 0);
  const output = new Uint8Array(size);
  output.set(source.slice(0, 12), 0);
  new DataView(output.buffer).setUint32(4, size - 8, true);
  let offset = 12;
  kept.forEach((chunk) => {
    output.set(chunk.bytes, offset);
    offset += chunk.bytes.length;
  });
  return new File([output], file.name, { type: file.type, lastModified: file.lastModified });
}

function webpChunks(bytes: Uint8Array): Array<{ kind: string; bytes: Uint8Array }> {
  if (ascii(bytes, 0, 4) !== 'RIFF' || ascii(bytes, 8, 12) !== 'WEBP') return [];
  const chunks: Array<{ kind: string; bytes: Uint8Array }> = [];
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let offset = 12;
  while (offset + 8 <= bytes.length) {
    const length = view.getUint32(offset + 4, true);
    const paddedLength = length + (length % 2);
    const end = offset + 8 + paddedLength;
    if (end > bytes.length) break;
    chunks.push({ kind: ascii(bytes, offset, offset + 4), bytes: bytes.slice(offset, end) });
    offset = end;
  }
  return chunks;
}

function ascii(bytes: Uint8Array, start: number, end: number): string {
  return String.fromCharCode(...bytes.slice(start, end));
}

function replaceExtension(fileName: string, extension: string): string {
  const stem = fileName.replace(/\.[^.]+$/u, '') || 'image';
  return `${stem}.${extension}`;
}
