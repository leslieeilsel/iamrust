import {
  Bell,
  Database,
  Download,
  Info,
  Keyboard,
  LogOut,
  Monitor,
  Palette,
  ShieldCheck,
  Trash2,
  UserRound,
} from 'lucide-react';
import QRCode from 'qrcode';
import { useEffect, useRef, useState } from 'react';

import type {
  AppSettings,
  DeviceInfo,
  ProfileVisibility,
  SecondFactorSetupResponse,
  SecondFactorStatus,
} from '../../lib/types';
import { api } from '../../lib/api';
import {
  clearLocalAccountCache,
  clearMediaCache,
  localCacheEncryptionStatus,
  readCacheStats,
  setLocalCacheEncryption,
  type CacheStats,
} from '../../lib/local-cache';
import { formatFileSize } from '../../lib/format';
import { readCrashReports } from '../../lib/crash-reporting';
import {
  checkForUpdates,
  chooseDownloadDirectory,
  writeAutostart,
} from '../../lib/desktop-plugins';
import { useChatStore } from '../../state/chat-store';
import { Avatar } from '../../components/Avatar';
import { AvatarCropDialog } from './AvatarCropDialog';
import { currentLanguage, tr } from '../../lib/i18n';

type SettingsSection =
  'profile' | 'appearance' | 'notifications' | 'keyboard' | 'storage' | 'privacy' | 'about';

const sections: Array<{ id: SettingsSection; label: string; icon: typeof UserRound }> = [
  { id: 'profile', label: '账号资料', icon: UserRound },
  { id: 'appearance', label: '外观', icon: Palette },
  { id: 'notifications', label: '通知与声音', icon: Bell },
  { id: 'keyboard', label: '快捷键', icon: Keyboard },
  { id: 'storage', label: '存储与下载', icon: Database },
  { id: 'privacy', label: '隐私与安全', icon: ShieldCheck },
  { id: 'about', label: '关于', icon: Info },
];

export function SettingsPane() {
  const [section, setSection] = useState<SettingsSection>('profile');
  return (
    <>
      <aside className="list-pane settings-nav">
        <header className="list-pane__header">
          <div>
            <h1>{tr('设置')}</h1>
          </div>
        </header>
        <nav aria-label={tr('设置分类')}>
          {sections.map(({ id, label, icon: Icon }) => (
            <button
              type="button"
              key={id}
              className={section === id ? 'is-selected' : ''}
              onClick={() => setSection(id)}
            >
              <Icon size={18} />
              {tr(label)}
            </button>
          ))}
        </nav>
      </aside>
      <SettingsContent section={section} />
    </>
  );
}

function SettingsContent({ section }: { section: SettingsSection }) {
  const settings = useChatStore((state) => state.settings);
  const update = useChatStore((state) => state.updateSettings);
  const me = useChatStore((state) => state.me);
  const updateProfile = useChatStore((state) => state.updateProfile);
  const profilePrivacy = useChatStore((state) => state.profilePrivacy);
  const updateProfilePrivacy = useChatStore((state) => state.updateProfilePrivacy);
  const clearAccount = useChatStore((state) => state.clearAccount);
  const demo = useChatStore((state) => state.demo);
  const setAnnouncement = useChatStore((state) => state.setAnnouncement);
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [devicesLoading, setDevicesLoading] = useState(false);
  const [cacheStats, setCacheStats] = useState<CacheStats | null>(null);
  const [cacheEncryptionAvailable, setCacheEncryptionAvailable] = useState(false);
  const [cacheEncryptionWorking, setCacheEncryptionWorking] = useState(false);
  const [avatarFile, setAvatarFile] = useState<File | null>(null);
  const [avatarSaving, setAvatarSaving] = useState(false);
  const [avatarProgress, setAvatarProgress] = useState(0);
  const [privacyDraft, setPrivacyDraft] = useState(profilePrivacy);
  const [privacySaving, setPrivacySaving] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [secondFactor, setSecondFactor] = useState<SecondFactorStatus | null>(null);
  const [secondFactorSetup, setSecondFactorSetup] = useState<SecondFactorSetupResponse | null>(
    null,
  );
  const [secondFactorQr, setSecondFactorQr] = useState('');
  const [recoveryCodes, setRecoveryCodes] = useState<string[]>([]);
  const [securityWorking, setSecurityWorking] = useState(false);
  const avatarInput = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (section !== 'privacy' || demo) return;
    setDevicesLoading(true);
    void api
      .devices()
      .then(setDevices)
      .catch(() => setAnnouncement(tr('无法加载登录设备。')))
      .finally(() => setDevicesLoading(false));
    void api
      .secondFactorStatus()
      .then(setSecondFactor)
      .catch(() => setAnnouncement(tr('无法加载双因素认证状态。')));
  }, [demo, section, setAnnouncement]);

  useEffect(() => {
    if (section !== 'storage') return;
    void readCacheStats()
      .then(setCacheStats)
      .catch(() => setAnnouncement(tr('无法读取缓存占用。')));
    void localCacheEncryptionStatus()
      .then((enabled) => {
        setCacheEncryptionAvailable(enabled !== null);
        if (enabled !== null) update({ localDatabaseEncryption: enabled });
      })
      .catch(() => {
        setCacheEncryptionAvailable(false);
        setAnnouncement(tr('无法读取本地加密状态。'));
      });
  }, [section, setAnnouncement, update]);

  useEffect(() => setPrivacyDraft(profilePrivacy), [profilePrivacy]);

  async function logout() {
    if (!demo) await api.logout().catch(() => undefined);
    if (!settings.keepCacheOnLogout) await clearLocalAccountCache().catch(() => undefined);
    clearAccount(settings.keepCacheOnLogout);
  }

  async function saveProfile(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!me) return;
    const form = new FormData(event.currentTarget);
    const nickname = formText(form, 'nickname').trim();
    const signature = formText(form, 'signature');
    const gender = optionalFormText(form, 'gender');
    const birthday = optionalFormText(form, 'birthday');
    const region = optionalFormText(form, 'region');
    const presence = formText(form, 'presence') as typeof me.presence;
    if (demo) {
      updateProfile({ ...me, nickname, signature, gender, birthday, region, presence });
      setAnnouncement(tr('资料已在演示模式中更新。'));
      return;
    }
    try {
      updateProfile(
        await api.updateProfile({
          nickname,
          signature,
          avatar_url: me.avatar_url,
          avatar_attachment_id: me.avatar_attachment_id,
          gender,
          birthday,
          region,
          presence,
        }),
      );
      setAnnouncement(tr('资料已保存。'));
    } catch {
      setAnnouncement(tr('资料保存失败，原内容已保留。'));
    }
  }

  function chooseAvatar(file: File | undefined) {
    if (!file) return;
    if (
      !['image/png', 'image/jpeg', 'image/webp'].includes(file.type) ||
      file.size > 10 * 1024 * 1024
    ) {
      setAnnouncement(tr('头像仅支持 PNG、JPEG 或 WebP，且不能超过 10 MB。'));
      return;
    }
    setAvatarFile(file);
  }

  async function saveAvatar(blob: Blob) {
    if (!me) return;
    setAvatarSaving(true);
    setAvatarProgress(0);
    try {
      if (demo) {
        const avatarUrl = URL.createObjectURL(blob);
        updateProfile({ ...me, avatar_url: avatarUrl, avatar_attachment_id: null });
      } else {
        const uploaded = await api.upload(
          new File([blob], 'avatar.webp', { type: 'image/webp' }),
          setAvatarProgress,
        );
        updateProfile(
          await api.updateProfile({
            nickname: me.nickname,
            signature: me.signature,
            avatar_url: null,
            avatar_attachment_id: uploaded.attachment.id,
            gender: me.gender,
            birthday: me.birthday,
            region: me.region,
            presence: me.presence,
          }),
        );
      }
      setAvatarFile(null);
      setAnnouncement(tr('头像已更新。'));
    } catch {
      setAnnouncement(tr('头像上传失败，原头像已保留。'));
    } finally {
      setAvatarSaving(false);
    }
  }

  async function changePassword(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const currentPassword = formText(form, 'current-password');
    const newPassword = formText(form, 'new-password');
    const confirmation = formText(form, 'confirm-password');
    if (newPassword !== confirmation) {
      setAnnouncement(tr('两次输入的新密码不一致。'));
      return;
    }
    if (demo) {
      setAnnouncement(tr('演示模式不会修改密码。'));
      return;
    }
    try {
      await api.changePassword(currentPassword, newPassword);
      await logout();
    } catch {
      setAnnouncement(tr('密码修改失败，请检查当前密码与新密码强度。'));
    }
  }

  async function revokeDevice(device: DeviceInfo) {
    if (device.current || demo) return;
    try {
      await api.revokeDevice(device.id);
      setDevices((current) => current.filter((item) => item.id !== device.id));
      setAnnouncement(tr(`已退出设备“${device.name}”。`));
    } catch {
      setAnnouncement(tr('远程退出失败，请稍后重试。'));
    }
  }

  async function beginSecondFactorSetup() {
    if (demo) {
      setAnnouncement(tr('演示模式不会启用双因素认证。'));
      return;
    }
    setSecurityWorking(true);
    try {
      const setup = await api.beginSecondFactorSetup();
      setSecondFactorSetup(setup);
      setSecondFactorQr(
        await QRCode.toDataURL(setup.otpauth_uri, {
          width: 208,
          margin: 2,
          color: { dark: '#111111', light: '#ffffff' },
        }),
      );
    } catch {
      setAnnouncement(tr('无法开始双因素认证设置。'));
    } finally {
      setSecurityWorking(false);
    }
  }

  async function enableSecondFactor(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSecurityWorking(true);
    try {
      const form = new FormData(event.currentTarget);
      const response = await api.enableSecondFactor(formText(form, 'two-factor-code'));
      setRecoveryCodes(response.recovery_codes);
      setSecondFactor({ enabled: true, recovery_codes_remaining: response.recovery_codes.length });
      setSecondFactorSetup(null);
      setSecondFactorQr('');
      event.currentTarget.reset();
      setAnnouncement(tr('双因素认证已启用，请立即保存恢复码。'));
    } catch {
      setAnnouncement(tr('验证码无效或设置已过期。'));
    } finally {
      setSecurityWorking(false);
    }
  }

  async function disableSecondFactor(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSecurityWorking(true);
    try {
      const form = new FormData(event.currentTarget);
      await api.disableSecondFactor(
        formText(form, 'two-factor-password'),
        formText(form, 'two-factor-disable-code'),
      );
      setSecondFactor({ enabled: false, recovery_codes_remaining: 0 });
      setRecoveryCodes([]);
      event.currentTarget.reset();
      setAnnouncement(tr('双因素认证已关闭。'));
    } catch {
      setAnnouncement(tr('关闭失败，请检查密码与验证码。'));
    } finally {
      setSecurityWorking(false);
    }
  }

  async function regenerateRecoveryCodes(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSecurityWorking(true);
    try {
      const form = new FormData(event.currentTarget);
      const response = await api.regenerateRecoveryCodes(
        formText(form, 'recovery-password'),
        formText(form, 'recovery-code'),
      );
      setRecoveryCodes(response.recovery_codes);
      setSecondFactor((current) =>
        current
          ? { ...current, recovery_codes_remaining: response.recovery_codes.length }
          : current,
      );
      event.currentTarget.reset();
      setAnnouncement(tr('恢复码已重新生成，旧恢复码已失效。'));
    } catch {
      setAnnouncement(tr('恢复码生成失败，请检查密码与验证码。'));
    } finally {
      setSecurityWorking(false);
    }
  }

  async function approveQrLogin(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    try {
      const payload = new URL(formText(form, 'qr-login-payload').trim());
      if (payload.protocol !== 'iamrust:' || payload.hostname !== 'auth')
        throw new Error('invalid');
      const challengeId = payload.searchParams.get('challenge_id');
      const secret = payload.searchParams.get('secret');
      if (!challengeId || !secret) throw new Error('invalid');
      await api.approveQrLogin(challengeId, secret);
      event.currentTarget.reset();
      setAnnouncement(tr('已批准新设备登录。'));
    } catch {
      setAnnouncement(tr('二维码内容无效、已过期或已经使用。'));
    }
  }

  async function savePrivacy(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (demo) {
      updateProfilePrivacy(privacyDraft);
      setAnnouncement(tr('隐私设置已在演示模式中更新。'));
      return;
    }
    setPrivacySaving(true);
    try {
      updateProfilePrivacy(await api.updateProfilePrivacy(privacyDraft));
      setAnnouncement(tr('资料可见范围已保存。'));
    } catch {
      setPrivacyDraft(profilePrivacy);
      setAnnouncement(tr('隐私设置保存失败，原设置已保留。'));
    } finally {
      setPrivacySaving(false);
    }
  }

  async function exportPersonalData() {
    setExporting(true);
    try {
      const payload = demo
        ? {
            generated_at: new Date().toISOString(),
            profile: me,
            privacy: profilePrivacy,
            note: tr('演示模式不包含真实账号数据。'),
          }
        : await api.exportPersonalData();
      downloadJson(payload, `iamrust-account-export-${new Date().toISOString().slice(0, 10)}.json`);
      setAnnouncement(tr('账号数据已导出。'));
    } catch {
      setAnnouncement(tr('账号数据导出失败，请稍后重试。'));
    } finally {
      setExporting(false);
    }
  }

  async function deleteAccount(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (demo) {
      setAnnouncement(tr('演示模式不会注销账号。'));
      return;
    }
    const form = new FormData(event.currentTarget);
    const password = formText(form, 'delete-password');
    const confirmation = formText(form, 'delete-confirmation');
    if (confirmation !== 'DELETE') {
      setAnnouncement(tr('请输入 DELETE 确认注销。'));
      return;
    }
    setDeleting(true);
    try {
      await api.deleteAccount(password, confirmation);
      await Promise.allSettled([clearLocalAccountCache(), clearMediaCache()]);
      clearAccount(false);
    } catch {
      setAnnouncement(tr('账号注销失败，请检查密码与确认文字。'));
      setDeleting(false);
    }
  }

  async function toggleLocalCacheEncryption(enabled: boolean) {
    setCacheEncryptionWorking(true);
    try {
      const actual = await setLocalCacheEncryption(enabled);
      update({ localDatabaseEncryption: actual });
      setAnnouncement(actual ? tr('本地缓存已加密。') : tr('本地缓存已解密。'));
    } catch {
      const actual = await localCacheEncryptionStatus().catch(() => null);
      if (actual !== null) update({ localDatabaseEncryption: actual });
      setAnnouncement(tr('无法更改本地缓存加密设置。请确认系统凭据库可用。'));
    } finally {
      setCacheEncryptionWorking(false);
    }
  }

  return (
    <section className="content-pane settings-content">
      {section === 'profile' && me ? (
        <SettingsCard title={tr('账号资料')} description={tr('其他人可以在资料卡中看到这些信息。')}>
          <div className="profile-settings-hero">
            <Avatar
              name={me.nickname}
              src={me.avatar_url}
              attachmentId={me.avatar_attachment_id}
              size="large"
            />
            <div>
              <strong>{me.nickname}</strong>
              <span>@{me.username}</span>
            </div>
            <button
              className="secondary-button"
              type="button"
              onClick={() => avatarInput.current?.click()}
            >
              {tr('更换头像')}
            </button>
            <input
              ref={avatarInput}
              hidden
              type="file"
              accept="image/png,image/jpeg,image/webp"
              onChange={(event) => {
                chooseAvatar(event.target.files?.[0]);
                event.currentTarget.value = '';
              }}
            />
          </div>
          <form className="settings-form" onSubmit={(event) => void saveProfile(event)}>
            <label>
              {tr('昵称')}
              <input name="nickname" defaultValue={me.nickname} maxLength={48} required />
            </label>
            <label>
              {tr('个性签名')}
              <textarea name="signature" defaultValue={me.signature} maxLength={160} rows={3} />
            </label>
            <div className="settings-form-grid">
              <label>
                {tr('状态')}
                <select name="presence" defaultValue={me.presence}>
                  <option value="online">{tr('在线')}</option>
                  <option value="away">{tr('离开')}</option>
                  <option value="busy">{tr('忙碌')}</option>
                  <option value="invisible">{tr('隐身')}</option>
                </select>
              </label>
              <label>
                {tr('性别（可选）')}
                <input name="gender" defaultValue={me.gender ?? ''} maxLength={32} />
              </label>
              <label>
                {tr('生日（可选）')}
                <input name="birthday" type="date" defaultValue={me.birthday ?? ''} />
              </label>
              <label>
                {tr('地区（可选）')}
                <input name="region" defaultValue={me.region ?? ''} maxLength={96} />
              </label>
            </div>
            <button className="primary-button" type="submit">
              {tr('保存资料')}
            </button>
          </form>
          <hr />
          <div className="danger-row">
            <div>
              <strong>{tr('退出账号')}</strong>
              <p>{tr('退出应用不会退出账号，两者是不同操作。')}</p>
            </div>
            <button className="danger-button" type="button" onClick={() => void logout()}>
              <LogOut size={17} />
              {tr('退出账号')}
            </button>
          </div>
        </SettingsCard>
      ) : null}
      {section === 'appearance' ? (
        <SettingsCard title={tr('外观')} description={tr('更改只影响当前设备。')}>
          <ChoiceGrid
            label={tr('主题')}
            value={settings.theme}
            options={[
              ['system', tr('跟随系统')],
              ['light', tr('浅色')],
              ['dark', tr('深色')],
              ['high-contrast', tr('高对比度')],
            ]}
            onChange={(theme) => update({ theme: theme as AppSettings['theme'] })}
          />
          <ChoiceGrid
            label={tr('界面语言')}
            value={settings.language}
            options={[
              ['zh-CN', tr('简体中文')],
              ['en-US', 'English'],
            ]}
            onChange={(language) => update({ language: language as AppSettings['language'] })}
          />
          <SettingToggle
            label={tr('紧凑模式')}
            description={tr('减小列表与控件间距。')}
            checked={settings.compactMode}
            onChange={(compactMode) => update({ compactMode })}
          />
          <SettingToggle
            label={tr('开机启动')}
            description={tr('登录系统后自动启动 I Am Rust。')}
            checked={settings.autostart}
            onChange={(autostart) => {
              update({ autostart });
              void writeAutostart(autostart).catch(() => {
                update({ autostart: !autostart });
                setAnnouncement(tr('无法更改开机启动设置。'));
              });
            }}
          />
          <ChoiceGrid
            label={tr('关闭主窗口时')}
            value={settings.closeBehavior}
            options={[
              ['tray', tr('最小化到托盘')],
              ['quit', tr('退出应用')],
            ]}
            onChange={(closeBehavior) =>
              update({ closeBehavior: closeBehavior as AppSettings['closeBehavior'] })
            }
          />
          <label className="range-setting">
            <span>
              <strong>{tr('字体大小')}</strong>
              <small>{Math.round(settings.fontScale * 100)}%</small>
            </span>
            <input
              type="range"
              min="0.85"
              max="1.4"
              step="0.05"
              value={settings.fontScale}
              onChange={(event) => update({ fontScale: Number(event.target.value) })}
            />
          </label>
        </SettingsCard>
      ) : null}
      {section === 'notifications' ? (
        <SettingsCard title={tr('通知与声音')} description={tr('免打扰会话仍会累计未读数。')}>
          <SettingToggle
            label={tr('桌面通知')}
            description={tr('收到新消息时显示原生通知。')}
            checked={settings.notifications}
            onChange={(notifications) => update({ notifications })}
          />
          <SettingToggle
            label={tr('通知声音')}
            description={tr('新消息到达时播放提示音。')}
            checked={settings.notificationSound}
            onChange={(notificationSound) => update({ notificationSound })}
          />
          <SettingToggle
            label={tr('显示消息预览')}
            description={tr('在通知中显示发送者和正文。')}
            checked={settings.notificationPreview}
            onChange={(notificationPreview) => update({ notificationPreview })}
          />
          <SettingToggle
            label={tr('隐私模式')}
            description={tr('通知始终隐藏发送者和正文。')}
            checked={settings.privacyMode}
            onChange={(privacyMode) => update({ privacyMode })}
          />
          <SettingToggle
            label={tr('定时免打扰')}
            description={tr('设定时间内继续累计未读，但不发声音和弹窗。')}
            checked={settings.doNotDisturbEnabled}
            onChange={(doNotDisturbEnabled) => update({ doNotDisturbEnabled })}
          />
          {settings.doNotDisturbEnabled ? (
            <div className="settings-form-grid dnd-time-grid">
              <label>
                {tr('开始时间')}
                <input
                  type="time"
                  value={settings.doNotDisturbStart}
                  onChange={(event) => update({ doNotDisturbStart: event.target.value })}
                />
              </label>
              <label>
                {tr('结束时间')}
                <input
                  type="time"
                  value={settings.doNotDisturbEnd}
                  onChange={(event) => update({ doNotDisturbEnd: event.target.value })}
                />
              </label>
            </div>
          ) : null}
        </SettingsCard>
      ) : null}
      {section === 'keyboard' ? (
        <SettingsCard title={tr('键盘与操作')} description={tr('可以随时使用 ? 查看完整快捷键。')}>
          <ChoiceGrid
            label={tr('发送消息')}
            value={settings.sendShortcut}
            options={[
              ['enter', 'Enter'],
              ['mod-enter', '⌘/Ctrl + Enter'],
            ]}
            onChange={(sendShortcut) =>
              update({ sendShortcut: sendShortcut as AppSettings['sendShortcut'] })
            }
          />
          <div className="shortcut-preview">
            <Keyboard />
            <div>
              <strong>{tr('全局搜索')}</strong>
              <span>
                <kbd>⌘</kbd> <kbd>K</kbd>
              </span>
            </div>
            <div>
              <strong>{tr('设置')}</strong>
              <span>
                <kbd>⌘</kbd> <kbd>,</kbd>
              </span>
            </div>
          </div>
          <SettingToggle
            label={tr('全局唤起快捷键')}
            description={tr('在其他应用中按 ⌘/Ctrl + Shift + I 唤起主窗口。')}
            checked={settings.globalShortcutEnabled}
            onChange={(globalShortcutEnabled) => update({ globalShortcutEnabled })}
          />
        </SettingsCard>
      ) : null}
      {section === 'storage' ? (
        <SettingsCard
          title={tr('存储与下载')}
          description={tr('媒体缓存可以安全清理，消息索引与发送队列不会被删除。')}
        >
          <label className="settings-field">
            {tr('默认下载目录')}
            <div>
              <input value={settings.downloadDirectory} readOnly placeholder={tr('系统下载目录')} />
              <button
                type="button"
                className="secondary-button"
                onClick={() =>
                  void chooseDownloadDirectory()
                    .then((downloadDirectory) => {
                      if (downloadDirectory) update({ downloadDirectory });
                    })
                    .catch(() => setAnnouncement(tr('无法打开目录选择器。')))
                }
              >
                {tr('选择')}
              </button>
            </div>
          </label>
          <div className="storage-meter">
            <span>
              <Database size={18} />
              {tr('缓存占用')}
            </span>
            <strong>
              {cacheStats
                ? formatFileSize(cacheStats.database_bytes + cacheStats.media_bytes)
                : tr('正在计算…')}
            </strong>
            <div>
              <i
                style={{
                  width: cacheStats
                    ? `${Math.min(100, (cacheStats.media_bytes / Math.max(1, cacheStats.database_bytes + cacheStats.media_bytes)) * 100)}%`
                    : '0%',
                }}
              />
            </div>
            {cacheStats ? (
              <small>
                {cacheStats.message_count} {tr('条本地消息 ·')} {cacheStats.pending_outbox_count}{' '}
                {tr('条待发送 · 媒体')}
                {formatFileSize(cacheStats.media_bytes)}
              </small>
            ) : null}
          </div>
          <button
            className="secondary-button"
            type="button"
            onClick={() =>
              void clearMediaCache()
                .then(readCacheStats)
                .then((stats) => {
                  setCacheStats(stats);
                  setAnnouncement(tr('已清理可安全删除的媒体缓存。'));
                })
                .catch(() => setAnnouncement(tr('媒体缓存清理失败。')))
            }
          >
            {tr('清理媒体缓存')}
          </button>
          <SettingToggle
            label={tr('退出后保留本地缓存')}
            description={tr('关闭后，退出账号会删除本地消息缓存和草稿。')}
            checked={settings.keepCacheOnLogout}
            onChange={(keepCacheOnLogout) => update({ keepCacheOnLogout })}
          />
          <SettingToggle
            label={tr('加密本地缓存')}
            description={
              cacheEncryptionAvailable
                ? tr(
                    '使用操作系统凭据库中的独立密钥加密消息、草稿、发件箱和资料快照。启用时本地全文索引会被清除。',
                  )
                : tr('仅桌面应用支持；浏览器预览不保存本地消息数据库。')
            }
            checked={settings.localDatabaseEncryption}
            disabled={!cacheEncryptionAvailable || cacheEncryptionWorking}
            onChange={(enabled) => void toggleLocalCacheEncryption(enabled)}
          />
        </SettingsCard>
      ) : null}
      {section === 'privacy' ? (
        <SettingsCard title={tr('隐私与安全')} description={tr('凭据保存在操作系统安全凭据库中。')}>
          <div className="security-summary">
            <ShieldCheck size={28} />
            <div>
              <strong>{tr('本设备会话受保护')}</strong>
              <p>{tr('访问令牌仅保存在内存，刷新令牌由系统凭据库管理。')}</p>
            </div>
          </div>
          <div className="security-block two-factor-settings">
            <div>
              <strong>{tr('双因素认证')}</strong>
              <p>{tr('登录时要求身份验证器验证码；每枚恢复码只能使用一次。')}</p>
            </div>
            {secondFactor?.enabled ? (
              <>
                <p className="security-status is-enabled">
                  {tr('已启用 · 剩余')}
                  {secondFactor.recovery_codes_remaining} {tr('枚恢复码')}
                </p>
                <details>
                  <summary>{tr('重新生成恢复码')}</summary>
                  <form
                    className="inline-security-form"
                    onSubmit={(event) => void regenerateRecoveryCodes(event)}
                  >
                    <input
                      name="recovery-password"
                      type="password"
                      autoComplete="current-password"
                      placeholder={tr('当前密码')}
                      required
                    />
                    <input
                      name="recovery-code"
                      autoComplete="one-time-code"
                      placeholder={tr('验证码或恢复码')}
                      required
                    />
                    <button className="secondary-button" type="submit" disabled={securityWorking}>
                      {tr('生成新恢复码')}
                    </button>
                  </form>
                </details>
                <details>
                  <summary>{tr('关闭双因素认证')}</summary>
                  <form
                    className="inline-security-form"
                    onSubmit={(event) => void disableSecondFactor(event)}
                  >
                    <input
                      name="two-factor-password"
                      type="password"
                      autoComplete="current-password"
                      placeholder={tr('当前密码')}
                      required
                    />
                    <input
                      name="two-factor-disable-code"
                      autoComplete="one-time-code"
                      placeholder={tr('验证码或恢复码')}
                      required
                    />
                    <button className="danger-button" type="submit" disabled={securityWorking}>
                      {tr('关闭双因素认证')}
                    </button>
                  </form>
                </details>
              </>
            ) : secondFactorSetup ? (
              <form
                className="two-factor-setup"
                onSubmit={(event) => void enableSecondFactor(event)}
              >
                {secondFactorQr ? (
                  <img src={secondFactorQr} alt={tr('双因素认证设置二维码')} />
                ) : null}
                <p>{tr('使用身份验证器扫描二维码，或手动输入密钥：')}</p>
                <code>{secondFactorSetup.secret}</code>
                <label>
                  {tr('6 位验证码')}
                  <input
                    name="two-factor-code"
                    inputMode="numeric"
                    autoComplete="one-time-code"
                    pattern="[0-9]{6}"
                    required
                  />
                </label>
                <button className="primary-button" type="submit" disabled={securityWorking}>
                  {tr('验证并启用')}
                </button>
              </form>
            ) : (
              <button
                className="secondary-button"
                type="button"
                disabled={securityWorking || demo}
                onClick={() => void beginSecondFactorSetup()}
              >
                {tr('启用双因素认证')}
              </button>
            )}
            {recoveryCodes.length > 0 ? (
              <div className="recovery-codes" role="status">
                <strong>{tr('仅显示这一次，请离线保存')}</strong>
                <div>
                  {recoveryCodes.map((code) => (
                    <code key={code}>{code}</code>
                  ))}
                </div>
              </div>
            ) : null}
          </div>
          <div className="security-block">
            <div>
              <strong>{tr('批准扫码登录')}</strong>
              <p>{tr('在另一台设备登录页生成二维码后，扫描器可把安全载荷交给此表单确认。')}</p>
            </div>
            <form className="inline-security-form" onSubmit={(event) => void approveQrLogin(event)}>
              <label>
                {tr('二维码安全载荷')}
                <input
                  name="qr-login-payload"
                  autoComplete="off"
                  placeholder="iamrust://auth/qr-login?…"
                  required
                />
              </label>
              <button className="secondary-button" type="submit" disabled={demo}>
                {tr('批准登录')}
              </button>
            </form>
          </div>
          <SettingToggle
            label={tr('通知隐私模式')}
            description={tr('隐藏通知中的身份与正文。')}
            checked={settings.privacyMode}
            onChange={(privacyMode) => update({ privacyMode })}
          />
          <SettingToggle
            label={tr('发送脱敏崩溃报告')}
            description={tr('默认开启。报告不包含账号、令牌、消息正文或错误堆栈，可随时关闭。')}
            checked={settings.crashReporting}
            onChange={(crashReporting) => update({ crashReporting })}
          />
          <form
            className="settings-form security-form"
            onSubmit={(event) => void savePrivacy(event)}
          >
            <div>
              <strong>{tr('资料可见范围')}</strong>
              <p>{tr('昵称、头像与用户名用于识别账号；以下可选资料可以单独控制。')}</p>
            </div>
            <div className="settings-form-grid">
              <VisibilitySelect
                label={tr('性别')}
                value={privacyDraft.gender_visibility}
                onChange={(gender_visibility) =>
                  setPrivacyDraft((current) => ({ ...current, gender_visibility }))
                }
              />
              <VisibilitySelect
                label={tr('生日')}
                value={privacyDraft.birthday_visibility}
                onChange={(birthday_visibility) =>
                  setPrivacyDraft((current) => ({ ...current, birthday_visibility }))
                }
              />
              <VisibilitySelect
                label={tr('地区')}
                value={privacyDraft.region_visibility}
                onChange={(region_visibility) =>
                  setPrivacyDraft((current) => ({ ...current, region_visibility }))
                }
              />
              <VisibilitySelect
                label={tr('在线状态')}
                value={privacyDraft.presence_visibility}
                onChange={(presence_visibility) =>
                  setPrivacyDraft((current) => ({ ...current, presence_visibility }))
                }
              />
            </div>
            <SettingToggle
              label={tr('发送已读回执')}
              description={tr('关闭后，其他人看不到你是否已读消息。')}
              checked={privacyDraft.read_receipts_enabled}
              onChange={(read_receipts_enabled) =>
                setPrivacyDraft((current) => ({ ...current, read_receipts_enabled }))
              }
            />
            <button className="secondary-button" type="submit" disabled={privacySaving}>
              {privacySaving ? tr('正在保存…') : tr('保存隐私设置')}
            </button>
          </form>
          <div className="security-block">
            <div>
              <strong>{tr('登录设备')}</strong>
              <p>{tr('可以远程撤销其他设备上的会话。')}</p>
            </div>
            {demo ? (
              <p className="field-hint">{tr('演示模式没有远程设备。')}</p>
            ) : devicesLoading ? (
              <p className="field-hint">{tr('正在加载设备…')}</p>
            ) : devices.length ? (
              <ul className="device-list">
                {devices.map((device) => (
                  <li key={device.id}>
                    <span>
                      <strong>
                        {device.name} {device.current ? tr('（当前设备）') : ''}
                      </strong>
                      <small>
                        {device.platform} · {device.app_version} {tr('· 最近使用')}{' '}
                        {new Intl.DateTimeFormat(currentLanguage(), {
                          dateStyle: 'medium',
                          timeStyle: 'short',
                        }).format(new Date(device.last_seen_at))}
                      </small>
                    </span>
                    <button
                      className="secondary-button"
                      type="button"
                      disabled={device.current}
                      onClick={() => void revokeDevice(device)}
                    >
                      {device.current ? tr('当前') : tr('远程退出')}
                    </button>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="field-hint">{tr('没有可显示的设备。')}</p>
            )}
          </div>
          <form
            className="settings-form security-form"
            onSubmit={(event) => void changePassword(event)}
          >
            <div>
              <strong>{tr('修改密码')}</strong>
              <p>{tr('修改后所有设备都需要重新登录。')}</p>
            </div>
            <label>
              {tr('当前密码')}
              <input
                name="current-password"
                type="password"
                autoComplete="current-password"
                required
              />
            </label>
            <label>
              {tr('新密码')}
              <input
                name="new-password"
                type="password"
                autoComplete="new-password"
                minLength={10}
                maxLength={128}
                required
              />
            </label>
            <label>
              {tr('确认新密码')}
              <input
                name="confirm-password"
                type="password"
                autoComplete="new-password"
                minLength={10}
                maxLength={128}
                required
              />
            </label>
            <button className="secondary-button" type="submit">
              {tr('修改密码并退出')}
            </button>
          </form>
          <div className="security-block security-actions">
            <div>
              <strong>{tr('导出数据')}</strong>
              <p>{tr('下载资料、好友关系、会话和消息的 JSON 副本。')}</p>
            </div>
            <button
              className="secondary-button"
              type="button"
              disabled={exporting}
              onClick={() => void exportPersonalData()}
            >
              <Download size={17} />
              {exporting ? tr('正在导出…') : tr('导出账号数据')}
            </button>
            <button className="secondary-button" type="button" onClick={exportDiagnostics}>
              {tr('导出脱敏诊断日志')}
            </button>
          </div>
          <form
            className="settings-form security-form account-deletion-form"
            onSubmit={(event) => void deleteAccount(event)}
          >
            <div>
              <strong>{tr('永久注销账号')}</strong>
              <p>
                {tr('账号会立即停用并匿名化；为保障其他参与者的会话完整性，已发送消息会保留。')}
              </p>
            </div>
            <label>
              {tr('当前密码')}
              <input
                name="delete-password"
                type="password"
                autoComplete="current-password"
                required
              />
            </label>
            <label>
              {tr('输入 DELETE 确认')}
              <input name="delete-confirmation" autoComplete="off" pattern="DELETE" required />
            </label>
            <button className="danger-button" type="submit" disabled={deleting}>
              <Trash2 size={17} />
              {deleting ? tr('正在注销…') : tr('永久注销账号')}
            </button>
          </form>
        </SettingsCard>
      ) : null}
      {section === 'about' ? (
        <SettingsCard
          title={tr('关于 I Am Rust')}
          description={tr('用 Rust 构建的多平台桌面即时通讯应用。')}
        >
          <div className="about-logo">
            <img src="/brand-logo.png" alt="" />
            <div>
              <strong>I Am Rust</strong>
              <span>{tr('版本 0.1.0 · 协议 v1')}</span>
            </div>
          </div>
          <dl className="about-list">
            <div>
              <dt>{tr('桌面框架')}</dt>
              <dd>Tauri 2</dd>
            </div>
            <div>
              <dt>{tr('服务端')}</dt>
              <dd>Axum</dd>
            </div>
            <div>
              <dt>{tr('界面')}</dt>
              <dd>React 19</dd>
            </div>
            <div>
              <dt>{tr('许可')}</dt>
              <dd>MIT OR Apache-2.0</dd>
            </div>
          </dl>
          <div className="about-actions">
            <button
              className="secondary-button"
              type="button"
              onClick={() =>
                void checkForUpdates()
                  .then((result) => {
                    if (result === 'current') setAnnouncement(tr('当前已是最新版本。'));
                  })
                  .catch(() => setAnnouncement(tr('暂时无法检查更新，请稍后重试。')))
              }
            >
              <Monitor size={17} />
              {tr('检查更新')}
            </button>
            <button className="secondary-button" type="button">
              {tr('查看开源许可证')}
            </button>
          </div>
        </SettingsCard>
      ) : null}
      <AvatarCropDialog
        file={avatarFile}
        saving={avatarSaving}
        progress={avatarProgress}
        onCancel={() => !avatarSaving && setAvatarFile(null)}
        onSave={saveAvatar}
      />
    </section>
  );
}

function SettingsCard({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <div className="settings-card">
      <header>
        <h2>{title}</h2>
        <p>{description}</p>
      </header>
      {children}
    </div>
  );
}

function SettingToggle({
  label,
  description,
  checked,
  disabled = false,
  onChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label className="setting-toggle">
      <span>
        <strong>{label}</strong>
        <small>{description}</small>
      </span>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      <i aria-hidden="true" />
    </label>
  );
}

function ChoiceGrid({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: string[][];
  onChange: (value: string) => void;
}) {
  return (
    <fieldset className="choice-grid">
      <legend>{label}</legend>
      <div>
        {options.map(([id, name]) => (
          <label key={id}>
            <input
              type="radio"
              name={label}
              value={id}
              checked={value === id}
              onChange={() => id && onChange(id)}
            />
            <span>{name}</span>
          </label>
        ))}
      </div>
    </fieldset>
  );
}

function VisibilitySelect({
  label,
  value,
  onChange,
}: {
  label: string;
  value: ProfileVisibility;
  onChange: (value: ProfileVisibility) => void;
}) {
  return (
    <label>
      {label}
      <select value={value} onChange={(event) => onChange(event.target.value as ProfileVisibility)}>
        <option value="everyone">{tr('所有人')}</option>
        <option value="friends">{tr('仅好友')}</option>
        <option value="nobody">{tr('仅自己')}</option>
      </select>
    </label>
  );
}

function exportDiagnostics() {
  downloadJson(
    {
      app: 'I Am Rust',
      version: '0.1.0',
      protocol: 1,
      timestamp: new Date().toISOString(),
      userAgent: navigator.userAgent,
      connection: useChatStore.getState().connection,
      crashReports: readCrashReports(),
      note: 'Tokens and message bodies intentionally omitted.',
    },
    `iamrust-diagnostics-${new Date().toISOString().slice(0, 10)}.json`,
  );
}

function downloadJson(value: unknown, fileName: string) {
  const payload = JSON.stringify(value, null, 2);
  const url = URL.createObjectURL(new Blob([payload], { type: 'application/json' }));
  const link = document.createElement('a');
  link.href = url;
  link.download = fileName;
  link.click();
  URL.revokeObjectURL(url);
}

function formText(form: FormData, name: string): string {
  const value = form.get(name);
  return typeof value === 'string' ? value : '';
}

function optionalFormText(form: FormData, name: string): string | null {
  return formText(form, name).trim() || null;
}
