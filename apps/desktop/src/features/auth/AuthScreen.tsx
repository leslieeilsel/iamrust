import { Eye, EyeOff, LoaderCircle } from 'lucide-react';
import QRCode from 'qrcode';
import { useEffect, useState, type FormEvent } from 'react';

import { api, ApiClientError } from '../../lib/api';
import { useChatStore } from '../../state/chat-store';
import { tr } from '../../lib/i18n';

type Mode = 'login' | 'register' | 'qr-login' | 'reset-request' | 'reset-confirm';

function friendlyError(error: unknown): string {
  if (error instanceof ApiClientError) {
    if (error.status === 0) return tr('无法连接到服务端。你可以先进入本地演示，或启动开发服务。');
    if (error.status === 429) return tr('尝试次数过多，请稍后再试。');
    if (error.status === 401) return tr('账号或密码不正确。');
    if (error.status === 409) return tr('注册信息不可用，请更换后重试。');
    if (error.field) return tr(`请检查 ${error.field}。`);
  }
  return tr('操作没有完成，请稍后重试。');
}

export function AuthScreen() {
  const [mode, setMode] = useState<Mode>('login');
  const [showPassword, setShowPassword] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [secondFactorRequired, setSecondFactorRequired] = useState(false);
  const [qrImage, setQrImage] = useState('');
  const [qrStatus, setQrStatus] = useState<'loading' | 'waiting' | 'error'>('loading');
  const [qrAttempt, setQrAttempt] = useState(0);
  const setBootstrap = useChatStore((state) => state.setBootstrap);
  const useDemo = useChatStore((state) => state.useDemo);

  useEffect(() => {
    if (mode !== 'qr-login') return;
    let active = true;
    let timer = 0;
    setQrImage('');
    setQrStatus('loading');
    void api
      .beginQrLogin()
      .then(async (challenge) => {
        const image = await QRCode.toDataURL(challenge.qr_payload, {
          width: 224,
          margin: 2,
          color: { dark: '#111111', light: '#ffffff' },
          errorCorrectionLevel: 'M',
        });
        if (!active) return;
        setQrImage(image);
        setQrStatus('waiting');
        const poll = async () => {
          if (!active) return;
          try {
            const result = await api.pollQrLogin(challenge.challenge_id, challenge.secret);
            if (result.status === 'ready' && result.session) {
              setBootstrap(await api.bootstrap());
              return;
            }
            timer = window.setTimeout(() => void poll(), 1_500);
          } catch {
            if (active) setQrStatus('error');
          }
        };
        timer = window.setTimeout(() => void poll(), 1_000);
      })
      .catch(() => active && setQrStatus('error'));
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [mode, qrAttempt, setBootstrap]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (submitting) return;
    const formElement = event.currentTarget;
    setSubmitting(true);
    setError('');
    setNotice('');
    const form = new FormData(formElement);
    try {
      if (mode === 'reset-request') {
        await api.requestPasswordReset(formText(form, 'email'));
        setMode('reset-confirm');
        setNotice(tr('如果该邮箱已注册，验证码已经发出。请检查收件箱。'));
        return;
      }
      if (mode === 'reset-confirm') {
        await api.confirmPasswordReset(
          formText(form, 'reset-token').trim(),
          formText(form, 'password'),
        );
        setMode('login');
        setNotice(tr('密码已更新，请使用新密码登录。'));
        return;
      }
      if (mode === 'login') {
        await api.login(
          formText(form, 'login'),
          formText(form, 'password'),
          optionalFormText(form, 'second-factor-code') ?? undefined,
        );
      } else {
        await api.register({
          email: formText(form, 'email'),
          username: formText(form, 'username'),
          nickname: formText(form, 'nickname'),
          password: formText(form, 'password'),
          device_name: navigator.userAgent.slice(0, 80),
        });
      }
      setBootstrap(await api.bootstrap());
    } catch (cause) {
      if (cause instanceof ApiClientError && cause.message === 'error.second_factor_required') {
        setSecondFactorRequired(true);
        setError(tr('请输入身份验证器中的 6 位验证码，或使用一枚恢复码。'));
        return;
      }
      setError(friendlyError(cause));
      const password = formElement.elements.namedItem('password');
      if (password instanceof HTMLInputElement) password.value = '';
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <main className="auth-screen">
      <h1 className="sr-only">I Am Rust</h1>
      <section className="auth-brand" aria-label="I Am Rust">
        <img src="/brand-logo.png" alt="I Am Rust Logo" />
        <div>
          <p className="auth-brand-title" aria-hidden="true">
            I Am Rust
          </p>
          <p>{tr('一个专注、可靠、跨平台的桌面即时通讯工具。')}</p>
        </div>
      </section>
      <section className="auth-card">
        <div className="segmented" role="tablist" aria-label={tr('账号操作')}>
          <button
            role="tab"
            aria-selected={mode === 'login'}
            className={mode === 'login' ? 'is-active' : ''}
            type="button"
            onClick={() => {
              setMode('login');
              setSecondFactorRequired(false);
              setError('');
              setNotice('');
            }}
          >
            {tr('登录')}
          </button>
          <button
            role="tab"
            aria-selected={mode === 'register'}
            className={mode === 'register' ? 'is-active' : ''}
            type="button"
            onClick={() => {
              setMode('register');
              setSecondFactorRequired(false);
              setError('');
              setNotice('');
            }}
          >
            {tr('注册')}
          </button>
          <button
            role="tab"
            aria-selected={mode === 'qr-login'}
            className={mode === 'qr-login' ? 'is-active' : ''}
            type="button"
            onClick={() => {
              setMode('qr-login');
              setSecondFactorRequired(false);
              setError('');
              setNotice('');
            }}
          >
            {tr('扫码登录')}
          </button>
        </div>
        <div className="auth-heading">
          <h2>
            {mode === 'login'
              ? tr('欢迎回来')
              : mode === 'register'
                ? tr('创建账号')
                : mode === 'qr-login'
                  ? tr('扫码登录')
                  : tr('重置密码')}
          </h2>
          <p>
            {mode === 'login'
              ? tr('继续你的对话。')
              : mode === 'register'
                ? tr('只需要几项基本信息。')
                : mode === 'qr-login'
                  ? tr('使用另一台已登录的 I Am Rust 设备扫描并确认。')
                  : mode === 'reset-request'
                    ? tr('我们会向注册邮箱发送一次性验证码。')
                    : tr('输入邮件中的验证码和新密码。')}
          </p>
        </div>
        {mode === 'qr-login' ? (
          <div className="qr-login-panel" aria-live="polite">
            {qrImage ? (
              <img src={qrImage} alt={tr('扫码登录二维码')} />
            ) : (
              <LoaderCircle className="spin" />
            )}
            <p>
              {qrStatus === 'loading'
                ? tr('正在创建安全登录请求…')
                : qrStatus === 'waiting'
                  ? tr('等待另一台设备确认。二维码将在 5 分钟后失效。')
                  : tr('二维码已失效或网络中断，请重新生成。')}
            </p>
            {qrStatus === 'error' ? (
              <button
                className="secondary-button"
                type="button"
                onClick={() => setQrAttempt((attempt) => attempt + 1)}
              >
                {tr('重新生成')}
              </button>
            ) : null}
          </div>
        ) : (
          <form onSubmit={(event) => void submit(event)}>
            {mode === 'register' ? (
              <>
                <label>
                  {tr('邮箱')}
                  <input name="email" type="email" autoComplete="email" required maxLength={254} />
                </label>
                <label>
                  {tr('用户名')}
                  <input
                    name="username"
                    autoComplete="username"
                    required
                    minLength={3}
                    maxLength={32}
                    pattern="[A-Za-z0-9_]+"
                    title={tr('3–32 位字母、数字或下划线')}
                  />
                </label>
                <label>
                  {tr('昵称')}
                  <input name="nickname" autoComplete="nickname" required maxLength={48} />
                </label>
              </>
            ) : mode === 'login' ? (
              <label>
                {tr('用户名或邮箱')}
                <input name="login" autoComplete="username" required autoFocus />
              </label>
            ) : mode === 'reset-request' ? (
              <label>
                {tr('注册邮箱')}
                <input name="email" type="email" autoComplete="email" required autoFocus />
              </label>
            ) : (
              <label>
                {tr('邮件验证码')}
                <input
                  name="reset-token"
                  autoComplete="one-time-code"
                  required
                  minLength={32}
                  maxLength={256}
                  autoFocus
                />
              </label>
            )}
            {mode !== 'reset-request' ? (
              <label>
                {mode === 'reset-confirm' ? tr('新密码') : tr('密码')}
                <span className="password-field">
                  <input
                    name="password"
                    type={showPassword ? 'text' : 'password'}
                    autoComplete={mode === 'login' ? 'current-password' : 'new-password'}
                    required
                    minLength={mode === 'login' ? 1 : 10}
                    maxLength={128}
                  />
                  <button
                    type="button"
                    aria-label={showPassword ? tr('隐藏密码') : tr('显示密码')}
                    onClick={() => setShowPassword((value) => !value)}
                  >
                    {showPassword ? <EyeOff size={18} /> : <Eye size={18} />}
                  </button>
                </span>
              </label>
            ) : null}
            {mode === 'login' && secondFactorRequired ? (
              <label>
                {tr('双因素验证码或恢复码')}
                <input
                  name="second-factor-code"
                  inputMode="numeric"
                  autoComplete="one-time-code"
                  minLength={6}
                  maxLength={19}
                  required
                  autoFocus
                />
              </label>
            ) : null}
            {mode === 'register' || mode === 'reset-confirm' ? (
              <p className="field-hint">{tr('至少 10 位，并包含大写字母、小写字母和数字。')}</p>
            ) : null}
            {mode === 'login' ? (
              <button
                className="auth-text-button"
                type="button"
                onClick={() => {
                  setMode('reset-request');
                  setError('');
                  setNotice('');
                }}
              >
                {tr('忘记密码？')}
              </button>
            ) : null}
            {mode === 'reset-request' || mode === 'reset-confirm' ? (
              <button
                className="auth-text-button"
                type="button"
                onClick={() => {
                  setMode('login');
                  setError('');
                  setNotice('');
                }}
              >
                {tr('返回登录')}
              </button>
            ) : null}
            <div className="form-notice" role="status" aria-live="polite">
              {notice}
            </div>
            <div className="form-error" role="alert" aria-live="polite">
              {error}
            </div>
            <button className="primary-button" type="submit" disabled={submitting}>
              {submitting ? <LoaderCircle className="spin" size={18} /> : null}
              {submitting
                ? tr('请稍候…')
                : mode === 'login'
                  ? tr('登录')
                  : mode === 'register'
                    ? tr('注册并登录')
                    : mode === 'reset-request'
                      ? tr('发送验证码')
                      : tr('更新密码')}
            </button>
          </form>
        )}
        <div className="auth-divider">
          <span>{tr('开发预览')}</span>
        </div>
        <button className="secondary-button" type="button" onClick={useDemo}>
          {tr('进入本地演示')}
        </button>
        <p className="legal-copy">{tr('继续即表示你同意用户协议与隐私政策。')}</p>
      </section>
    </main>
  );
}

function formText(form: FormData, name: string): string {
  const value = form.get(name);
  return typeof value === 'string' ? value : '';
}

function optionalFormText(form: FormData, name: string): string | null {
  const value = formText(form, name).trim();
  return value || null;
}
