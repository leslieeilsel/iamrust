import { describe, expect, it } from 'vitest';

import { hasSafeImageHeader } from './image-processing';

describe('image upload validation', () => {
  it.each([
    ['PNG', [0x89, 0x50, 0x4e, 0x47]],
    ['JPEG', [0xff, 0xd8, 0xff, 0xe0]],
    ['GIF', Array.from(new TextEncoder().encode('GIF89a'))],
    [
      'WebP',
      [
        ...Array.from(new TextEncoder().encode('RIFF')),
        0,
        0,
        0,
        0,
        ...Array.from(new TextEncoder().encode('WEBP')),
      ],
    ],
  ])('accepts a valid %s signature', async (_name, signature) => {
    const file = new File([new Uint8Array(signature)], 'image.bin');
    await expect(hasSafeImageHeader(file)).resolves.toBe(true);
  });

  it('rejects content that only claims to be an image', async () => {
    const file = new File(['not an image'], 'payload.png', { type: 'image/png' });
    await expect(hasSafeImageHeader(file)).resolves.toBe(false);
  });
});
