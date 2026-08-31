import { describe, expect, it } from 'vitest';

import { formatFileSize, messageSummary, safeHttpUrl, splitLinks } from './format';

describe('format helpers', () => {
  it('only treats HTTP and HTTPS values as external links', () => {
    expect(safeHttpUrl('https://example.com/path')?.hostname).toBe('example.com');
    expect(safeHttpUrl('javascript:alert(1)')).toBeNull();
    expect(splitLinks('打开 https://example.com 然后继续')).toEqual([
      { value: '打开 ', href: null },
      { value: 'https://example.com', href: 'https://example.com/' },
      { value: ' 然后继续', href: null },
    ]);
  });

  it('creates bounded summaries and readable sizes', () => {
    expect(messageSummary({ type: 'text', data: { text: 'hello\n  world' } })).toBe('hello world');
    expect(
      messageSummary({
        type: 'sticker',
        data: {
          name: 'Ferris',
          attachment: {
            id: 'attachment',
            kind: 'image',
            file_name: 'ferris.webp',
            mime_type: 'image/webp',
            byte_size: 12,
            sha256: null,
            storage_key: 'blob:test',
            thumbnail_key: null,
          },
        },
      }),
    ).toBe('[表情] Ferris');
    expect(formatFileSize(1_572_864)).toBe('1.5 MB');
  });
});
