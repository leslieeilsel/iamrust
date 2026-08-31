import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { useChatStore } from '../../state/chat-store';
import { MessageComposer } from './MessageComposer';

const conversationId = '0199b000-0000-7000-8000-000000000099';

describe('MessageComposer', () => {
  beforeEach(() => {
    localStorage.clear();
    useChatStore.setState({
      meta: {},
      settings: { ...useChatStore.getState().settings, sendShortcut: 'enter' },
    });
  });

  it('sends trimmed text with Enter and clears the draft', async () => {
    const onSend = vi.fn(() => Promise.resolve(true));
    const user = userEvent.setup();
    render(<MessageComposer conversationId={conversationId} onSend={onSend} />);
    const input = screen.getByLabelText('输入消息');
    await user.type(input, '  hello  ');
    fireEvent.keyDown(input, { key: 'Enter', code: 'Enter' });
    await waitFor(() =>
      expect(onSend).toHaveBeenCalledWith(
        'hello',
        [],
        false,
        false,
        [],
        null,
        expect.any(Function),
        expect.anything(),
      ),
    );
    expect(input).toHaveValue('');
  });

  it('does not send during an IME composition session', async () => {
    const onSend = vi.fn(() => Promise.resolve(true));
    const user = userEvent.setup();
    render(<MessageComposer conversationId={conversationId} onSend={onSend} />);
    const input = screen.getByLabelText('输入消息');
    await user.type(input, '你');
    fireEvent.compositionStart(input);
    fireEvent.keyDown(input, { key: 'Enter', code: 'Enter', isComposing: true });
    expect(onSend).not.toHaveBeenCalled();
    fireEvent.compositionEnd(input);
    fireEvent.keyDown(input, { key: 'Enter', code: 'Enter' });
    await waitFor(() => expect(onSend).toHaveBeenCalledOnce());
  });

  it('keeps Shift+Enter for line breaks', async () => {
    const onSend = vi.fn(() => Promise.resolve(true));
    const user = userEvent.setup();
    render(<MessageComposer conversationId={conversationId} onSend={onSend} />);
    const input = screen.getByLabelText('输入消息');
    await user.type(input, 'first');
    fireEvent.keyDown(input, { key: 'Enter', code: 'Enter', shiftKey: true });
    expect(onSend).not.toHaveBeenCalled();
  });
});
