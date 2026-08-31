import { HelpCircle, MessageCircle, Search, Settings, UserRound, UsersRound } from 'lucide-react';

import type { AppSection } from '../lib/types';
import { useChatStore } from '../state/chat-store';
import { Avatar } from './Avatar';
import { IconButton } from './IconButton';
import { tr } from '../lib/i18n';

interface NavigationRailProps {
  onHelp: () => void;
}

export function NavigationRail({ onHelp }: NavigationRailProps) {
  const section = useChatStore((state) => state.section);
  const setSection = useChatStore((state) => state.setSection);
  const me = useChatStore((state) => state.me);
  const totalUnread = useChatStore((state) =>
    Object.values(state.meta).reduce((sum, item) => sum + item.unread, 0),
  );

  const navItems: Array<{ id: AppSection; label: string; icon: typeof MessageCircle }> = [
    { id: 'conversations', label: tr('会话'), icon: MessageCircle },
    { id: 'contacts', label: tr('联系人'), icon: UsersRound },
    { id: 'search', label: tr('搜索'), icon: Search },
  ];

  return (
    <nav className="navigation-rail" aria-label={tr('主要导航')}>
      <button
        type="button"
        className="profile-trigger"
        aria-label={tr('查看我的资料')}
        onClick={() => setSection('settings')}
      >
        {me ? (
          <Avatar
            name={me.nickname}
            src={me.avatar_url}
            attachmentId={me.avatar_attachment_id}
            presence={me.presence}
          />
        ) : (
          <UserRound />
        )}
      </button>
      <div className="navigation-rail__main">
        {navItems.map(({ id, label, icon: Icon }) => (
          <div className="nav-item-wrap" key={id}>
            <IconButton label={label} active={section === id} onClick={() => setSection(id)}>
              <Icon size={22} />
            </IconButton>
            {id === 'conversations' && totalUnread > 0 ? (
              <span className="rail-badge" aria-label={tr(`${totalUnread} 条未读消息`)}>
                {totalUnread > 99 ? '99+' : totalUnread}
              </span>
            ) : null}
          </div>
        ))}
      </div>
      <div className="navigation-rail__bottom">
        <IconButton label={tr('快捷键帮助')} onClick={onHelp}>
          <HelpCircle size={21} />
        </IconButton>
        <IconButton
          label={tr('设置')}
          active={section === 'settings'}
          onClick={() => setSection('settings')}
        >
          <Settings size={21} />
        </IconButton>
      </div>
    </nav>
  );
}
