import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { MessageContentView } from './MessageContentView';

describe('MessageContentView', () => {
  it('renders untrusted markup as inert text', () => {
    render(
      <MessageContentView
        content={{ type: 'text', data: { text: '<img src=x onerror=alert(1)>' } }}
      />,
    );
    expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeInTheDocument();
    expect(document.querySelector('img')).toBeNull();
  });

  it('adds safe attributes to HTTP links', () => {
    render(
      <MessageContentView content={{ type: 'text', data: { text: 'https://example.com' } }} />,
    );
    const link = screen.getByRole('link');
    expect(link).toHaveAttribute('rel', expect.stringContaining('noopener'));
    expect(link).toHaveAttribute('target', '_blank');
  });

  it('highlights mentions without interpreting them as markup', () => {
    render(
      <MessageContentView
        content={{ type: 'text', data: { text: '@alice 请看 <script>' } }}
        hasMentions
      />,
    );
    expect(screen.getByText('@alice')).toHaveClass('message-mention');
    expect(screen.getByText(/<script>/u)).toBeInTheDocument();
    expect(document.querySelector('script')).toBeNull();
  });

  it('renders a safe inline Markdown subset', () => {
    render(
      <MessageContentView
        content={{
          type: 'text',
          data: { text: '**粗体** *斜体* `code` ~~删除~~ <b>普通文本</b>' },
        }}
      />,
    );
    expect(screen.getByText('粗体').tagName).toBe('STRONG');
    expect(screen.getByText('斜体').tagName).toBe('EM');
    expect(screen.getByText('code').tagName).toBe('CODE');
    expect(screen.getByText('删除').tagName).toBe('DEL');
    expect(document.querySelector('b')).toBeNull();
  });
});
