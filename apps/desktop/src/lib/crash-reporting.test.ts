import { beforeEach, describe, expect, it } from 'vitest';

import { readCrashReports, startCrashReporting } from './crash-reporting';

describe('redacted crash reporting', () => {
  beforeEach(() => localStorage.clear());

  it('stores only bounded non-sensitive error metadata', () => {
    const dispose = startCrashReporting(true);
    const error = new Error('secret token and message body');
    window.dispatchEvent(
      new ErrorEvent('error', {
        error,
        message: error.message,
        filename: 'file:///Users/example/private/project/app.js?token=secret',
        lineno: 42,
        colno: 7,
      }),
    );
    dispose();

    expect(readCrashReports()).toMatchObject([
      {
        kind: 'error',
        error_name: 'Error',
        source_file: 'app.js',
        line: 42,
        column: 7,
      },
    ]);
    expect(JSON.stringify(readCrashReports())).not.toContain('secret');
    expect(JSON.stringify(readCrashReports())).not.toContain('/Users/example');
  });

  it('does not collect when disabled', () => {
    const dispose = startCrashReporting(false);
    const suppressForTest = (event: ErrorEvent) => event.preventDefault();
    window.addEventListener('error', suppressForTest);
    window.dispatchEvent(new ErrorEvent('error', { error: new Error('ignored') }));
    window.removeEventListener('error', suppressForTest);
    dispose();
    expect(readCrashReports()).toEqual([]);
  });
});
