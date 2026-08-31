import { tr } from '../lib/i18n';
export function LoadingScreen() {
  return (
    <main className="loading-screen" aria-busy="true" aria-label={tr('正在恢复会话')}>
      <img src="/brand-logo.png" alt="" />
      <div className="loading-screen__bar">
        <span />
      </div>
      <p>{tr('正在安全地恢复会话…')}</p>
    </main>
  );
}
