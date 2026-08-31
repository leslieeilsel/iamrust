const STORAGE_KEY = 'iamrust-redacted-crashes-v1';
const MAX_REPORTS = 20;

export interface RedactedCrashReport {
  timestamp: string;
  kind: 'error' | 'unhandled_rejection';
  error_name: string;
  source_file: string | null;
  line: number | null;
  column: number | null;
}

export function startCrashReporting(enabled: boolean): () => void {
  if (!enabled) return () => undefined;

  const onError = (event: ErrorEvent) => {
    storeReport({
      timestamp: new Date().toISOString(),
      kind: 'error',
      error_name: safeErrorName(event.error),
      source_file: safeSourceFile(event.filename),
      line: finiteNumber(event.lineno),
      column: finiteNumber(event.colno),
    });
  };
  const onUnhandledRejection = (event: PromiseRejectionEvent) => {
    storeReport({
      timestamp: new Date().toISOString(),
      kind: 'unhandled_rejection',
      error_name: safeErrorName(event.reason),
      source_file: null,
      line: null,
      column: null,
    });
  };

  window.addEventListener('error', onError);
  window.addEventListener('unhandledrejection', onUnhandledRejection);
  return () => {
    window.removeEventListener('error', onError);
    window.removeEventListener('unhandledrejection', onUnhandledRejection);
  };
}

export function readCrashReports(): RedactedCrashReport[] {
  try {
    const parsed = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]') as unknown;
    return Array.isArray(parsed) ? parsed.filter(isCrashReport).slice(-MAX_REPORTS) : [];
  } catch {
    return [];
  }
}

function storeReport(report: RedactedCrashReport): void {
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify([...readCrashReports(), report].slice(-MAX_REPORTS)),
    );
  } catch {
    // Crash collection must never cause another crash.
  }
}

function safeErrorName(value: unknown): string {
  if (value instanceof Error && /^[A-Za-z][A-Za-z0-9_.-]{0,63}$/u.test(value.name)) {
    return value.name;
  }
  return 'UnknownError';
}

function safeSourceFile(source: string): string | null {
  if (!source) return null;
  try {
    const path = new URL(source, window.location.href).pathname;
    const fileName = path.split('/').filter(Boolean).at(-1);
    return fileName?.slice(0, 120) ?? null;
  } catch {
    return null;
  }
}

function finiteNumber(value: number): number | null {
  return Number.isSafeInteger(value) && value > 0 ? value : null;
}

function isCrashReport(value: unknown): value is RedactedCrashReport {
  if (!value || typeof value !== 'object') return false;
  const report = value as Partial<RedactedCrashReport>;
  return (
    typeof report.timestamp === 'string' &&
    (report.kind === 'error' || report.kind === 'unhandled_rejection') &&
    typeof report.error_name === 'string' &&
    (report.source_file === null || typeof report.source_file === 'string') &&
    (report.line === null || typeof report.line === 'number') &&
    (report.column === null || typeof report.column === 'number')
  );
}
