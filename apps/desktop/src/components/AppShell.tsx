import { useState } from 'react';

import { useChatStore } from '../state/chat-store';
import { NavigationRail } from './NavigationRail';
import { ShortcutHelp } from './ShortcutHelp';
import { StatusBanner } from './StatusBanner';
import { ConversationList } from '../features/chat/ConversationList';
import { ChatView } from '../features/chat/ChatView';
import { ContactsList } from '../features/contacts/ContactsList';
import { ContactProfile } from '../features/contacts/ContactProfile';
import { AddFriendDialog } from '../features/contacts/AddFriendDialog';
import { CreateGroupDialog } from '../features/chat/CreateGroupDialog';
import { GlobalSearch } from '../features/chat/GlobalSearch';
import { SettingsPane } from '../features/settings/SettingsPane';
import { CallDialog } from '../features/calls/CallDialog';
import type { UserId } from '../lib/types';
import { tr } from '../lib/i18n';

interface AppShellProps {
  onReconnect: () => void;
  detached?: boolean;
}

export function AppShell({ onReconnect, detached = false }: AppShellProps) {
  const section = useChatStore((state) => state.section);
  const connection = useChatStore((state) => state.connection);
  const announcement = useChatStore((state) => state.announcement);
  const setAnnouncement = useChatStore((state) => state.setAnnouncement);
  const friends = useChatStore((state) => state.friends);
  const [helpOpen, setHelpOpen] = useState(false);
  const [addFriendOpen, setAddFriendOpen] = useState(false);
  const [groupOpen, setGroupOpen] = useState(false);
  const [selectedContact, setSelectedContact] = useState<UserId | null>(friends[0]?.id ?? null);

  if (detached) {
    return (
      <main className="app-frame is-detached">
        <StatusBanner state={connection} onRetry={onReconnect} />
        <div className="detached-chat-layout">
          <ChatView />
        </div>
        <CallDialog />
        <Announcement announcement={announcement} onClose={() => setAnnouncement('')} />
      </main>
    );
  }

  return (
    <main className="app-frame">
      <StatusBanner state={connection} onRetry={onReconnect} />
      <div className="app-layout">
        <NavigationRail onHelp={() => setHelpOpen(true)} />
        {section === 'conversations' ? (
          <>
            <ConversationList onCreateGroup={() => setGroupOpen(true)} />
            <ChatView />
          </>
        ) : null}
        {section === 'contacts' ? (
          <>
            <ContactsList
              selectedId={selectedContact}
              onSelect={setSelectedContact}
              onAddFriend={() => setAddFriendOpen(true)}
            />
            <ContactProfile userId={selectedContact} />
          </>
        ) : null}
        {section === 'search' ? <GlobalSearch /> : null}
        {section === 'settings' ? <SettingsPane /> : null}
      </div>
      <ShortcutHelp open={helpOpen} onOpenChange={setHelpOpen} />
      <AddFriendDialog open={addFriendOpen} onOpenChange={setAddFriendOpen} />
      <CreateGroupDialog open={groupOpen} onOpenChange={setGroupOpen} />
      <CallDialog />
      <Announcement announcement={announcement} onClose={() => setAnnouncement('')} />
    </main>
  );
}

function Announcement({ announcement, onClose }: { announcement: string; onClose: () => void }) {
  return (
    <>
      <div className="sr-only" aria-live="polite" aria-atomic="true">
        {announcement}
      </div>
      {announcement ? (
        <button className="toast" type="button" onClick={onClose}>
          {announcement}
          <span>{tr('关闭')}</span>
        </button>
      ) : null}
    </>
  );
}
