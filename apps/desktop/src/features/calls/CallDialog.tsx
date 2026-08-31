import * as Dialog from '@radix-ui/react-dialog';
import { Camera, CameraOff, Mic, MicOff, MonitorUp, Phone, PhoneOff, X } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';

import { Avatar } from '../../components/Avatar';
import { IconButton } from '../../components/IconButton';
import {
  sendCallSignal,
  subscribeCallSignals,
  subscribeCallStarts,
  type CallSignalFrame,
} from '../../lib/realtime';
import { notify } from '../../lib/notifications';
import type { AppSettings, UserProfile } from '../../lib/types';
import {
  conversationAvatarUser,
  conversationName,
  useChatStore,
  userById,
} from '../../state/chat-store';
import { tr } from '../../lib/i18n';

type CallPhase = 'incoming' | 'calling' | 'connecting' | 'connected' | 'ended' | 'error';

interface ActiveCall {
  conversationId: string;
  callId: string;
  peerId: string | null;
  group: boolean;
  video: boolean;
  incoming: boolean;
}

interface CallStatsReport {
  type?: string;
  state?: string;
  currentRoundTripTime?: unknown;
  isRemote?: unknown;
  packetsLost?: unknown;
  packetsReceived?: unknown;
}

export function CallDialog() {
  const conversations = useChatStore((state) => state.conversations);
  const friends = useChatStore((state) => state.friends);
  const me = useChatStore((state) => state.me);
  const friendSettings = useChatStore((state) => state.friendSettings);
  const setAnnouncement = useChatStore((state) => state.setAnnouncement);
  const [active, setActiveState] = useState<ActiveCall | null>(null);
  const [phase, setPhase] = useState<CallPhase>('ended');
  const [status, setStatus] = useState('');
  const [muted, setMuted] = useState(false);
  const [cameraEnabled, setCameraEnabled] = useState(true);
  const [sharingScreen, setSharingScreen] = useState(false);
  const [audioInputs, setAudioInputs] = useState<MediaDeviceInfo[]>([]);
  const [selectedAudioInput, setSelectedAudioInput] = useState('');
  const [videoInputs, setVideoInputs] = useState<MediaDeviceInfo[]>([]);
  const [selectedVideoInput, setSelectedVideoInput] = useState('');
  const [inputLevel, setInputLevel] = useState(0);
  const [networkQuality, setNetworkQuality] = useState('');
  const [duration, setDuration] = useState(0);
  const [localStream, setLocalStream] = useState<MediaStream | null>(null);
  const [remoteStreams, setRemoteStreams] = useState<Record<string, MediaStream>>({});
  const activeRef = useRef<ActiveCall | null>(null);
  const phaseRef = useRef<CallPhase>('ended');
  const selectedAudioInputRef = useRef('');
  const selectedVideoInputRef = useRef('');
  const peers = useRef<Map<string, RTCPeerConnection>>(new Map());
  const localStreamRef = useRef<MediaStream | null>(null);
  const pendingCandidates = useRef<Map<string, RTCIceCandidateInit[]>>(new Map());
  const localVideo = useRef<HTMLVideoElement>(null);
  const dismissTimer = useRef<number | null>(null);
  const screenAudioSenders = useRef<Map<string, RTCRtpSender>>(new Map());

  const conversation = conversations.find((item) => item.id === active?.conversationId);
  const avatarUser = conversation ? conversationAvatarUser(conversation, friends) : undefined;
  const displayName = conversation
    ? conversationName(conversation, friends, friendSettings)
    : tr('未知联系人');

  function setActive(call: ActiveCall | null) {
    activeRef.current = call;
    setActiveState(call);
  }

  function updatePhase(next: CallPhase) {
    phaseRef.current = next;
    setPhase(next);
  }

  useEffect(() => {
    const stopStarts = subscribeCallStarts((conversationId, video) => {
      void beginOutgoing(conversationId, video);
    });
    const stopSignals = subscribeCallSignals((frame) => {
      void handleSignal(frame);
    });
    return () => {
      stopStarts();
      stopSignals();
      cleanupMedia();
    };
  }, []);

  useEffect(() => {
    if (localVideo.current) localVideo.current.srcObject = localStream;
  }, [localStream]);

  useEffect(() => {
    if (phase !== 'calling' && phase !== 'incoming') return;
    const timer = window.setTimeout(() => {
      if (activeRef.current) endCall(tr('无人接听'), true);
    }, 30_000);
    return () => window.clearTimeout(timer);
  }, [active?.callId, phase]);

  useEffect(() => {
    if (phase !== 'connected') {
      setDuration(0);
      setNetworkQuality('');
      return;
    }
    const startedAt = Date.now();
    const timer = window.setInterval(() => setDuration(Date.now() - startedAt), 1_000);
    const qualityTimer = window.setInterval(() => void updateNetworkQuality(), 3_000);
    void updateNetworkQuality();
    return () => {
      window.clearInterval(timer);
      window.clearInterval(qualityTimer);
    };
  }, [phase]);

  useEffect(() => {
    const audioTrack = localStream?.getAudioTracks()[0];
    if (!audioTrack) {
      setInputLevel(0);
      return;
    }
    if (!window.AudioContext) return;
    const context = new window.AudioContext();
    const source = context.createMediaStreamSource(new MediaStream([audioTrack]));
    const analyser = context.createAnalyser();
    analyser.fftSize = 256;
    source.connect(analyser);
    const samples = new Uint8Array(analyser.frequencyBinCount);
    const timer = window.setInterval(() => {
      analyser.getByteTimeDomainData(samples);
      const peak = samples.reduce((value, sample) => Math.max(value, Math.abs(sample - 128)), 0);
      setInputLevel(Math.min(100, (peak / 64) * 100));
    }, 120);
    return () => {
      window.clearInterval(timer);
      void context.close();
    };
  }, [localStream]);

  useEffect(() => {
    if (phase !== 'incoming') return;
    const settings = useChatStore.getState().settings;
    if (!settings.notificationSound || isDoNotDisturbActive(settings)) return;
    if (!window.AudioContext) return;
    const context = new window.AudioContext();
    const ring = () => {
      if (context.state === 'suspended') void context.resume();
      const gain = context.createGain();
      gain.gain.setValueAtTime(0.0001, context.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.08, context.currentTime + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, context.currentTime + 0.55);
      gain.connect(context.destination);
      [440, 554].forEach((frequency, index) => {
        const oscillator = context.createOscillator();
        oscillator.frequency.value = frequency;
        oscillator.type = 'sine';
        oscillator.connect(gain);
        oscillator.start(context.currentTime + index * 0.12);
        oscillator.stop(context.currentTime + 0.55);
      });
    };
    ring();
    const timer = window.setInterval(ring, 1_800);
    return () => {
      window.clearInterval(timer);
      void context.close();
    };
  }, [phase]);

  async function beginOutgoing(conversationId: string, video: boolean) {
    if (activeRef.current) {
      setAnnouncement(tr('当前已有通话进行中。'));
      return;
    }
    const target = useChatStore.getState().conversations.find((item) => item.id === conversationId);
    if (!target) return;
    const group = target.kind.kind === 'group';
    const call: ActiveCall = {
      conversationId,
      callId: crypto.randomUUID(),
      peerId: target.kind.kind === 'direct' ? target.kind.peer_user_id : null,
      group,
      video,
      incoming: false,
    };
    setActive(call);
    updatePhase('calling');
    setStatus(group ? tr('正在邀请群成员…') : video ? tr('正在发起视频通话…') : tr('正在呼叫…'));
    try {
      await prepareLocalMedia(call);
      if (call.peerId) ensurePeer(call, call.peerId);
      if (!sendCallSignal(conversationId, call.callId, { kind: 'invite', video })) {
        throw new Error('realtime offline');
      }
    } catch {
      endCall(tr('无法使用麦克风或摄像头'), false, 'error');
    }
  }

  async function handleSignal(frame: CallSignalFrame) {
    const current = activeRef.current;
    if (frame.signal.kind === 'invite') {
      if (current) {
        sendCallSignal(frame.conversation_id, frame.call_id, { kind: 'busy' }, frame.from_user_id);
        return;
      }
      const state = useChatStore.getState();
      const incomingConversation = state.conversations.find(
        (item) => item.id === frame.conversation_id,
      );
      if (!incomingConversation) return;
      const call: ActiveCall = {
        conversationId: frame.conversation_id,
        callId: frame.call_id,
        peerId: frame.from_user_id,
        group: incomingConversation.kind.kind === 'group',
        video: frame.signal.video,
        incoming: true,
      };
      setActive(call);
      updatePhase('incoming');
      setStatus(
        incomingConversation.kind.kind === 'group'
          ? tr('邀请你加入群组通话')
          : frame.signal.video
            ? tr('邀请你进行视频通话')
            : tr('邀请你进行语音通话'),
      );
      if (
        incomingConversation &&
        state.settings.notifications &&
        !incomingConversation.muted &&
        !isDoNotDisturbActive(state.settings)
      ) {
        notify({
          conversationId: frame.conversation_id,
          title: tr(
            `${conversationName(incomingConversation, state.friends, state.friendSettings)} 来电`,
          ),
          body: frame.signal.video ? tr('邀请你进行视频通话') : tr('邀请你进行语音通话'),
          sound: state.settings.notificationSound,
        });
      }
      return;
    }
    if (!current || current.callId !== frame.call_id) return;
    switch (frame.signal.kind) {
      case 'accept':
        if (!current.group && !current.incoming) {
          await createAndSendOffer(current, frame.from_user_id);
        }
        break;
      case 'offer':
        await receiveOffer(current, frame.from_user_id, frame.signal.sdp);
        break;
      case 'answer': {
        const connection = peers.current.get(frame.from_user_id);
        await connection?.setRemoteDescription({ type: 'answer', sdp: frame.signal.sdp });
        await flushCandidates(frame.from_user_id);
        updatePhase('connecting');
        setStatus(tr('正在建立加密媒体连接…'));
        break;
      }
      case 'ice_candidate': {
        const candidate: RTCIceCandidateInit = {
          candidate: frame.signal.candidate,
          sdpMid: frame.signal.sdp_mid,
          sdpMLineIndex: frame.signal.sdp_mline_index,
        };
        const connection = peers.current.get(frame.from_user_id);
        if (connection?.remoteDescription) await connection.addIceCandidate(candidate);
        else {
          const pending = pendingCandidates.current.get(frame.from_user_id) ?? [];
          pendingCandidates.current.set(frame.from_user_id, [...pending, candidate]);
        }
        break;
      }
      case 'participants': {
        if (!current.group || phaseRef.current === 'incoming') break;
        const myId = useChatStore.getState().me?.id;
        if (!myId) break;
        await Promise.all(
          frame.signal.user_ids
            .filter((userId) => userId !== myId && myId.localeCompare(userId) < 0)
            .map((userId) => createAndSendOffer(current, userId)),
        );
        break;
      }
      case 'reject':
        if (current.group) setStatus(tr('有成员拒绝了邀请，通话仍可继续'));
        else endCall(tr('对方已拒绝'), false);
        break;
      case 'busy':
        if (current.group) setStatus(tr('有成员正在其他通话中'));
        else endCall(tr('对方正在通话中'), false);
        break;
      case 'hangup':
        if (current.group) {
          removePeer(frame.from_user_id);
          setStatus(tr('有成员退出了群组通话'));
        } else {
          endCall(tr('通话已结束'), false);
        }
        break;
    }
  }

  async function acceptCall() {
    const call = activeRef.current;
    if (!call || !call.incoming) return;
    updatePhase('connecting');
    setStatus(tr('正在准备设备…'));
    try {
      await prepareLocalMedia(call);
      sendCallSignal(
        call.conversationId,
        call.callId,
        { kind: 'accept' },
        call.group ? undefined : (call.peerId ?? undefined),
      );
    } catch {
      sendCallSignal(
        call.conversationId,
        call.callId,
        { kind: 'reject' },
        call.peerId ?? undefined,
      );
      endCall(tr('无法使用麦克风或摄像头'), false, 'error');
    }
  }

  function rejectCall() {
    const call = activeRef.current;
    if (!call) return;
    sendCallSignal(call.conversationId, call.callId, { kind: 'reject' }, call.peerId ?? undefined);
    endCall(tr('已拒绝'), false);
  }

  async function prepareLocalMedia(call: ActiveCall) {
    if (localStreamRef.current) return;
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: selectedAudioInputRef.current
        ? {
            deviceId: { exact: selectedAudioInputRef.current },
            echoCancellation: true,
            noiseSuppression: true,
          }
        : { echoCancellation: true, noiseSuppression: true },
      video: call.video
        ? {
            ...(selectedVideoInputRef.current
              ? { deviceId: { exact: selectedVideoInputRef.current } }
              : {}),
            width: { ideal: 1280 },
            height: { ideal: 720 },
          }
        : false,
    });
    localStreamRef.current = stream;
    setLocalStream(stream);
    setMuted(false);
    setCameraEnabled(call.video);
    const devices = await navigator.mediaDevices.enumerateDevices();
    setAudioInputs(devices.filter((device) => device.kind === 'audioinput'));
    setVideoInputs(devices.filter((device) => device.kind === 'videoinput'));
    const currentMic = stream.getAudioTracks()[0]?.getSettings().deviceId;
    if (currentMic) {
      selectedAudioInputRef.current = currentMic;
      setSelectedAudioInput(currentMic);
    }
    const currentCamera = stream.getVideoTracks()[0]?.getSettings().deviceId;
    if (currentCamera) {
      selectedVideoInputRef.current = currentCamera;
      setSelectedVideoInput(currentCamera);
    }
  }

  function ensurePeer(call: ActiveCall, remoteUserId: string): RTCPeerConnection {
    const existing = peers.current.get(remoteUserId);
    if (existing) return existing;
    const stream = localStreamRef.current;
    if (!stream) throw new Error('local media is not ready');
    const connection = new RTCPeerConnection({ iceServers: configuredIceServers() });
    peers.current.set(remoteUserId, connection);
    stream.getTracks().forEach((track) => connection.addTrack(track, stream));
    connection.onicecandidate = (event) => {
      if (!event.candidate || activeRef.current?.callId !== call.callId) return;
      sendCallSignal(
        call.conversationId,
        call.callId,
        {
          kind: 'ice_candidate',
          candidate: event.candidate.candidate,
          sdp_mid: event.candidate.sdpMid,
          sdp_mline_index: event.candidate.sdpMLineIndex,
        },
        remoteUserId,
      );
    };
    connection.ontrack = (event) => {
      const stream = event.streams[0] ?? new MediaStream([event.track]);
      setRemoteStreams((current) => ({ ...current, [remoteUserId]: stream }));
    };
    connection.onconnectionstatechange = () => {
      if (connection.connectionState === 'connected') {
        updatePhase('connected');
        setStatus(call.group ? tr('群组通话中') : tr('通话中'));
      }
      if (['failed', 'disconnected'].includes(connection.connectionState)) {
        if (call.group) {
          removePeer(remoteUserId);
          setStatus(tr('一位成员的媒体连接已断开'));
        } else {
          endCall(tr('媒体连接已断开'), true, 'error');
        }
      }
    };
    return connection;
  }

  async function createAndSendOffer(call: ActiveCall, remoteUserId: string) {
    const connection = ensurePeer(call, remoteUserId);
    if (connection.signalingState !== 'stable' || connection.localDescription) return;
    const offer = await connection.createOffer();
    await connection.setLocalDescription(offer);
    sendCallSignal(
      call.conversationId,
      call.callId,
      { kind: 'offer', sdp: offer.sdp ?? '' },
      remoteUserId,
    );
    updatePhase('connecting');
    setStatus(tr('正在建立加密媒体连接…'));
  }

  async function receiveOffer(call: ActiveCall, remoteUserId: string, sdp: string) {
    const connection = ensurePeer(call, remoteUserId);
    await connection.setRemoteDescription({ type: 'offer', sdp });
    await flushCandidates(remoteUserId);
    const answer = await connection.createAnswer();
    await connection.setLocalDescription(answer);
    sendCallSignal(
      call.conversationId,
      call.callId,
      { kind: 'answer', sdp: answer.sdp ?? '' },
      remoteUserId,
    );
  }

  async function flushCandidates(remoteUserId: string) {
    const connection = peers.current.get(remoteUserId);
    if (!connection?.remoteDescription) return;
    const candidates = pendingCandidates.current.get(remoteUserId) ?? [];
    pendingCandidates.current.delete(remoteUserId);
    for (const candidate of candidates) await connection.addIceCandidate(candidate);
  }

  function toggleMute() {
    const next = !muted;
    localStreamRef.current?.getAudioTracks().forEach((track) => {
      track.enabled = !next;
    });
    setMuted(next);
  }

  function toggleCamera() {
    const next = !cameraEnabled;
    localStreamRef.current?.getVideoTracks().forEach((track) => {
      track.enabled = next;
    });
    setCameraEnabled(next);
  }

  async function changeMicrophone(deviceId: string) {
    if (!localStreamRef.current) return;
    try {
      const replacement = await navigator.mediaDevices.getUserMedia({
        audio: { deviceId: { exact: deviceId }, echoCancellation: true, noiseSuppression: true },
      });
      const track = replacement.getAudioTracks()[0];
      if (!track) return;
      const replacements: Promise<void>[] = [];
      peers.current.forEach((connection) => {
        const sender = connection.getSenders().find((item) => item.track?.kind === 'audio');
        if (sender) replacements.push(sender.replaceTrack(track));
      });
      await Promise.all(replacements);
      localStreamRef.current.getAudioTracks().forEach((oldTrack) => oldTrack.stop());
      const next = new MediaStream([track, ...localStreamRef.current.getVideoTracks()]);
      localStreamRef.current = next;
      setLocalStream(next);
      selectedAudioInputRef.current = deviceId;
      setSelectedAudioInput(deviceId);
      setMuted(false);
    } catch {
      setAnnouncement(tr('无法切换麦克风。'));
    }
  }

  async function changeCamera(deviceId: string) {
    if (!localStreamRef.current) return;
    try {
      const replacement = await navigator.mediaDevices.getUserMedia({
        video: {
          deviceId: { exact: deviceId },
          width: { ideal: 1280 },
          height: { ideal: 720 },
        },
      });
      const track = replacement.getVideoTracks()[0];
      if (!track) return;
      const replacements: Promise<void>[] = [];
      peers.current.forEach((connection) => {
        const sender = connection.getSenders().find((item) => item.track?.kind === 'video');
        if (sender) replacements.push(sender.replaceTrack(track));
      });
      await Promise.all(replacements);
      localStreamRef.current.getVideoTracks().forEach((oldTrack) => oldTrack.stop());
      const next = new MediaStream([track, ...localStreamRef.current.getAudioTracks()]);
      localStreamRef.current = next;
      setLocalStream(next);
      selectedVideoInputRef.current = deviceId;
      setSelectedVideoInput(deviceId);
      setCameraEnabled(true);
    } catch {
      setAnnouncement(tr('无法切换摄像头。'));
    }
  }

  async function toggleScreenShare() {
    const call = activeRef.current;
    if (peers.current.size === 0 || !call?.video || !localStreamRef.current) return;
    if (sharingScreen) {
      await restoreCameraTrack();
      return;
    }
    try {
      const display = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: true });
      const track = display.getVideoTracks()[0];
      const systemAudio = display.getAudioTracks()[0];
      await Promise.all(
        [...peers.current.entries()].map(async ([userId, connection]) => {
          const sender = connection.getSenders().find((item) => item.track?.kind === 'video');
          await sender?.replaceTrack(track ?? null);
          if (systemAudio) {
            screenAudioSenders.current.set(userId, connection.addTrack(systemAudio, display));
          }
        }),
      );
      setSharingScreen(true);
      if (track) track.onended = () => void restoreCameraTrack();
    } catch {
      setAnnouncement(tr('屏幕共享未开始。'));
    }
  }

  async function restoreCameraTrack() {
    const camera = localStreamRef.current?.getVideoTracks()[0] ?? null;
    await Promise.all(
      [...peers.current.entries()].map(async ([userId, connection]) => {
        const sender = connection.getSenders().find((item) => item.track?.kind === 'video');
        await sender?.replaceTrack(camera);
        const audioSender = screenAudioSenders.current.get(userId);
        if (audioSender) {
          audioSender.track?.stop();
          connection.removeTrack(audioSender);
          screenAudioSenders.current.delete(userId);
        }
      }),
    );
    setSharingScreen(false);
  }

  async function updateNetworkQuality() {
    const connections = [...peers.current.values()].filter(
      (connection) => connection.connectionState === 'connected',
    );
    if (connections.length === 0) return;
    try {
      let roundTripSeconds: number | null = null;
      let packetsLost = 0;
      let packetsReceived = 0;
      const allReports = await Promise.all(connections.map((connection) => connection.getStats()));
      allReports.forEach((reports) =>
        reports.forEach((rawReport) => {
          const report = rawReport as unknown as CallStatsReport;
          if (report.type === 'candidate-pair' && report.state === 'succeeded') {
            const rtt = report.currentRoundTripTime;
            if (typeof rtt === 'number') {
              roundTripSeconds = Math.max(roundTripSeconds ?? 0, rtt);
            }
          }
          if (report.type === 'inbound-rtp' && !report.isRemote) {
            if (typeof report.packetsLost === 'number') packetsLost += report.packetsLost;
            if (typeof report.packetsReceived === 'number')
              packetsReceived += report.packetsReceived;
          }
        }),
      );
      const loss = packetsLost / Math.max(1, packetsLost + packetsReceived);
      const quality =
        loss > 0.08 || (roundTripSeconds ?? 0) > 0.45
          ? tr('网络较弱')
          : loss > 0.03 || (roundTripSeconds ?? 0) > 0.25
            ? tr('网络一般')
            : tr('网络良好');
      setNetworkQuality(quality);
    } catch {
      setNetworkQuality('');
    }
  }

  function endCall(
    nextStatus = tr('通话已结束'),
    notifyPeer = true,
    nextPhase: CallPhase = 'ended',
  ) {
    const call = activeRef.current;
    if (call && notifyPeer) {
      sendCallSignal(call.conversationId, call.callId, { kind: 'hangup' });
    }
    cleanupMedia();
    updatePhase(nextPhase);
    setStatus(nextStatus);
    if (dismissTimer.current !== null) window.clearTimeout(dismissTimer.current);
    dismissTimer.current = window.setTimeout(() => {
      if (activeRef.current?.callId === call?.callId) setActive(null);
    }, 1_400);
  }

  function cleanupMedia() {
    peers.current.forEach((connection) => connection.close());
    peers.current.clear();
    localStreamRef.current?.getTracks().forEach((track) => track.stop());
    localStreamRef.current = null;
    setLocalStream(null);
    setRemoteStreams({});
    setSharingScreen(false);
    screenAudioSenders.current.clear();
    pendingCandidates.current.clear();
  }

  function removePeer(userId: string) {
    peers.current.get(userId)?.close();
    peers.current.delete(userId);
    pendingCandidates.current.delete(userId);
    const audioSender = screenAudioSenders.current.get(userId);
    audioSender?.track?.stop();
    screenAudioSenders.current.delete(userId);
    setRemoteStreams((current) => {
      const next = { ...current };
      delete next[userId];
      return next;
    });
  }

  const formattedDuration = useMemo(() => {
    const seconds = Math.floor(duration / 1_000);
    return `${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`;
  }, [duration]);
  const remoteEntries = Object.entries(remoteStreams);

  return (
    <Dialog.Root
      open={active !== null}
      onOpenChange={(open) => {
        if (!open && activeRef.current) endCall(tr('通话已结束'));
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="call-overlay" />
        <Dialog.Content className={`call-dialog ${active?.video ? 'is-video' : ''}`}>
          <Dialog.Title className="sr-only">
            {tr('与')} {displayName} {tr('通话')}
          </Dialog.Title>
          <Dialog.Description className="sr-only">{status}</Dialog.Description>
          <IconButton label={tr('关闭通话窗口')} className="call-close" onClick={() => endCall()}>
            <X size={18} />
          </IconButton>
          {active?.video ? (
            <div className={`call-video-stage ${active.group ? 'is-group' : ''}`}>
              {remoteEntries.map(([userId, stream]) => (
                <div className="call-remote-video" key={userId}>
                  <RemoteMedia
                    stream={stream}
                    video
                    label={tr(`${participantName(userId, me, friends)} 的视频`)}
                  />
                  <span>{participantName(userId, me, friends)}</span>
                </div>
              ))}
              <video ref={localVideo} autoPlay playsInline muted aria-label={tr('我的视频预览')} />
              {remoteEntries.length === 0 ? (
                <div className="call-video-placeholder">
                  <Avatar
                    name={displayName}
                    src={conversation?.avatar_url ?? avatarUser?.avatar_url}
                    attachmentId={
                      conversation?.avatar_attachment_id ?? avatarUser?.avatar_attachment_id
                    }
                    size="large"
                  />
                </div>
              ) : null}
            </div>
          ) : (
            <div className="call-audio-hero">
              <Avatar
                name={displayName}
                src={conversation?.avatar_url ?? avatarUser?.avatar_url}
                attachmentId={
                  conversation?.avatar_attachment_id ?? avatarUser?.avatar_attachment_id
                }
                size="large"
              />
              <h2>{displayName}</h2>
              {active?.group && remoteEntries.length > 0 ? (
                <p>
                  {remoteEntries.length + 1} {tr('人正在通话')}
                </p>
              ) : null}
            </div>
          )}
          {!active?.video
            ? remoteEntries.map(([userId, stream]) => (
                <RemoteMedia
                  key={`audio-${userId}`}
                  stream={stream}
                  video={false}
                  label={tr(`${participantName(userId, me, friends)} 的音频`)}
                />
              ))
            : null}
          <div className="call-status" aria-live="polite">
            <strong>{status}</strong>
            {phase === 'connected' ? <time>{formattedDuration}</time> : null}
            {phase === 'connected' && networkQuality ? <span>{networkQuality}</span> : null}
          </div>
          {phase === 'incoming' ? (
            <div className="incoming-call-actions">
              <button className="call-action is-reject" type="button" onClick={rejectCall}>
                <PhoneOff size={22} /> {tr('拒绝')}
              </button>
              <button
                className="call-action is-accept"
                type="button"
                onClick={() => void acceptCall()}
              >
                <Phone size={22} /> {tr('接听')}
              </button>
            </div>
          ) : phase === 'ended' || phase === 'error' ? null : (
            <>
              <div className="call-device-row">
                <label>
                  <span className="sr-only">{tr('麦克风')}</span>
                  <select
                    value={selectedAudioInput}
                    onChange={(event) => void changeMicrophone(event.target.value)}
                  >
                    {audioInputs.map((device, index) => (
                      <option key={device.deviceId} value={device.deviceId}>
                        {device.label || tr(`麦克风 ${index + 1}`)}
                      </option>
                    ))}
                  </select>
                </label>
                {active?.video && videoInputs.length > 0 ? (
                  <label>
                    <span className="sr-only">{tr('摄像头')}</span>
                    <select
                      value={selectedVideoInput}
                      onChange={(event) => void changeCamera(event.target.value)}
                    >
                      {videoInputs.map((device, index) => (
                        <option key={device.deviceId} value={device.deviceId}>
                          {device.label || tr(`摄像头 ${index + 1}`)}
                        </option>
                      ))}
                    </select>
                  </label>
                ) : null}
                <span
                  className="input-level"
                  aria-label={tr(`麦克风输入电平 ${Math.round(inputLevel)}%`)}
                >
                  <i style={{ width: `${inputLevel}%` }} />
                </span>
              </div>
              <div className="call-controls">
                <button
                  className="call-action"
                  type="button"
                  aria-pressed={muted}
                  onClick={toggleMute}
                >
                  {muted ? <MicOff size={20} /> : <Mic size={20} />}
                  {muted ? tr('取消静音') : tr('静音')}
                </button>
                {active?.video ? (
                  <button
                    className="call-action"
                    type="button"
                    aria-pressed={!cameraEnabled}
                    onClick={toggleCamera}
                  >
                    {cameraEnabled ? <Camera size={20} /> : <CameraOff size={20} />}
                    {cameraEnabled ? tr('关闭视频') : tr('开启视频')}
                  </button>
                ) : null}
                {active?.video && phase === 'connected' ? (
                  <button
                    className="call-action"
                    type="button"
                    aria-pressed={sharingScreen}
                    onClick={() => void toggleScreenShare()}
                  >
                    <MonitorUp size={20} /> {sharingScreen ? tr('停止共享') : tr('共享屏幕')}
                  </button>
                ) : null}
                <button className="call-action is-reject" type="button" onClick={() => endCall()}>
                  <PhoneOff size={21} /> {tr('挂断')}
                </button>
              </div>
            </>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function RemoteMedia({
  stream,
  video,
  label,
}: {
  stream: MediaStream;
  video: boolean;
  label: string;
}) {
  const element = useRef<HTMLMediaElement>(null);
  useEffect(() => {
    if (element.current) element.current.srcObject = stream;
  }, [stream]);
  return video ? (
    <video
      ref={(node) => {
        element.current = node;
      }}
      autoPlay
      playsInline
      aria-label={label}
    />
  ) : (
    <audio
      ref={(node) => {
        element.current = node;
      }}
      autoPlay
      aria-label={label}
    />
  );
}

function participantName(userId: string, me: UserProfile | null, friends: UserProfile[]): string {
  return userById({ me, friends }, userId)?.nickname ?? tr(`成员 ${userId.slice(-6)}`);
}

function configuredIceServers(): RTCIceServer[] {
  const configured = import.meta.env.VITE_ICE_SERVERS;
  if (configured) {
    try {
      const parsed = JSON.parse(configured) as RTCIceServer[];
      if (Array.isArray(parsed) && parsed.length) return parsed;
    } catch {
      // Fall back to a public STUN server when local configuration is malformed.
    }
  }
  return [{ urls: 'stun:stun.l.google.com:19302' }];
}

function isDoNotDisturbActive(settings: AppSettings): boolean {
  if (!settings.doNotDisturbEnabled) return false;
  const now = new Date();
  const minutes = now.getHours() * 60 + now.getMinutes();
  const [startHours = 0, startMinutes = 0] = settings.doNotDisturbStart.split(':').map(Number);
  const [endHours = 0, endMinutes = 0] = settings.doNotDisturbEnd.split(':').map(Number);
  const start = startHours * 60 + startMinutes;
  const end = endHours * 60 + endMinutes;
  if (start === end) return true;
  return start < end ? minutes >= start && minutes < end : minutes >= start || minutes < end;
}
