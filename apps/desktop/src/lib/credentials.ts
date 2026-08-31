// Browser-only development fallback. Packaged Tauri builds keep credentials in
// Rust and the operating-system credential vault, never in WebView state.
let browserFallback: string | null = null;

export function saveRefreshToken(refreshToken: string): void {
  browserFallback = refreshToken;
}

export function loadRefreshToken(): string | null {
  return browserFallback;
}

export function clearRefreshToken(): void {
  browserFallback = null;
}
