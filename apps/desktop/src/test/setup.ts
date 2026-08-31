import '@testing-library/jest-dom/vitest';

// Node 25 exposes an incomplete, flag-gated `localStorage` global. Replace it in
// tests so Zustand persistence behaves the same way as it does in the WebView.
const values = new Map<string, string>();
const storage: Storage = {
  get length() {
    return values.size;
  },
  clear() {
    values.clear();
  },
  getItem(key) {
    return values.get(key) ?? null;
  },
  key(index) {
    return [...values.keys()][index] ?? null;
  },
  removeItem(key) {
    values.delete(key);
  },
  setItem(key, value) {
    values.set(key, value);
  },
};

Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: storage });
