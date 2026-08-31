import { Mic, Pause, Play, SendHorizontal, Square, Trash2 } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { IconButton } from '../../components/IconButton';
import { claimAudio, releaseAudio } from '../../lib/audio-playback';
import { tr } from '../../lib/i18n';

const MAX_DURATION_MS = 120_000;

interface VoiceRecorderProps {
  disabled: boolean;
  onSend: (blob: Blob, durationMs: number, mimeType: string) => Promise<boolean>;
}

export function VoiceRecorder({ disabled, onSend }: VoiceRecorderProps) {
  const [status, setStatus] = useState<'idle' | 'requesting' | 'recording' | 'preview' | 'sending'>(
    'idle',
  );
  const [elapsedMs, setElapsedMs] = useState(0);
  const [blob, setBlob] = useState<Blob | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [playing, setPlaying] = useState(false);
  const [error, setError] = useState('');
  const recorder = useRef<MediaRecorder | null>(null);
  const stream = useRef<MediaStream | null>(null);
  const chunks = useRef<Blob[]>([]);
  const startedAt = useRef(0);
  const timer = useRef<number | null>(null);
  const discardOnStop = useRef(false);
  const audio = useRef<HTMLAudioElement>(null);

  useEffect(
    () => () => {
      discardOnStop.current = true;
      if (recorder.current?.state === 'recording') recorder.current.stop();
      stopStream(stream.current);
      if (timer.current !== null) window.clearInterval(timer.current);
      if (previewUrl) URL.revokeObjectURL(previewUrl);
      if (audio.current) releaseAudio(audio.current);
    },
    [previewUrl],
  );

  async function start() {
    if (disabled || status !== 'idle') return;
    if (!navigator.mediaDevices?.getUserMedia || !('MediaRecorder' in window)) {
      setError(tr('当前系统不支持语音录制。'));
      return;
    }
    setStatus('requesting');
    setError(tr('正在请求麦克风权限…'));
    try {
      const mediaStream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
      });
      const mimeType = preferredMimeType();
      const mediaRecorder = mimeType
        ? new MediaRecorder(mediaStream, { mimeType })
        : new MediaRecorder(mediaStream);
      stream.current = mediaStream;
      recorder.current = mediaRecorder;
      chunks.current = [];
      discardOnStop.current = false;
      startedAt.current = performance.now();
      setElapsedMs(0);
      setError('');
      mediaRecorder.addEventListener('dataavailable', (event) => {
        if (event.data.size > 0) chunks.current.push(event.data);
      });
      mediaRecorder.addEventListener('stop', () => {
        stopStream(mediaStream);
        stream.current = null;
        if (timer.current !== null) window.clearInterval(timer.current);
        timer.current = null;
        if (discardOnStop.current) {
          setStatus('idle');
          return;
        }
        const duration = Math.min(MAX_DURATION_MS, performance.now() - startedAt.current);
        const normalizedMime = normalizeAudioMime(mediaRecorder.mimeType);
        const recorded = new Blob(chunks.current, { type: normalizedMime });
        if (recorded.size === 0 || duration < 350) {
          setError(tr('录音过短，请重试。'));
          setStatus('idle');
          return;
        }
        const url = URL.createObjectURL(recorded);
        setElapsedMs(duration);
        setBlob(recorded);
        setPreviewUrl(url);
        setStatus('preview');
      });
      mediaRecorder.start(250);
      setStatus('recording');
      timer.current = window.setInterval(() => {
        const elapsed = performance.now() - startedAt.current;
        setElapsedMs(Math.min(MAX_DURATION_MS, elapsed));
        if (elapsed >= MAX_DURATION_MS && mediaRecorder.state === 'recording') mediaRecorder.stop();
      }, 100);
    } catch {
      stopStream(stream.current);
      setError(tr('无法使用麦克风，请在系统设置中授予权限。'));
      setStatus('idle');
    }
  }

  function stop() {
    if (recorder.current?.state === 'recording') recorder.current.stop();
  }

  function reset() {
    discardOnStop.current = true;
    if (recorder.current?.state === 'recording') recorder.current.stop();
    stopStream(stream.current);
    if (timer.current !== null) window.clearInterval(timer.current);
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    if (audio.current) releaseAudio(audio.current);
    setPreviewUrl(null);
    setBlob(null);
    setElapsedMs(0);
    setPlaying(false);
    setError('');
    setStatus('idle');
  }

  async function send() {
    if (!blob || status !== 'preview') return;
    setStatus('sending');
    const ok = await onSend(blob, Math.round(elapsedMs), normalizeAudioMime(blob.type));
    if (ok) reset();
    else {
      setStatus('preview');
      setError(tr('语音发送失败，可以重试或取消。'));
    }
  }

  if (status === 'idle' && !error) {
    return (
      <IconButton label={tr('录制语音消息')} disabled={disabled} onClick={() => void start()}>
        <Mic size={19} />
      </IconButton>
    );
  }

  return (
    <div className="voice-recorder" role="status" aria-live="polite">
      {status === 'requesting' ? <span>{tr('正在请求麦克风权限…')}</span> : null}
      {status === 'recording' ? (
        <>
          <i className="recording-dot" aria-hidden="true" />
          <strong>{formatDuration(elapsedMs)}</strong>
          <span>{tr('最长 2:00')}</span>
          <IconButton label={tr('停止录音')} onClick={stop}>
            <Square size={16} />
          </IconButton>
          <IconButton label={tr('取消录音')} onClick={reset}>
            <Trash2 size={16} />
          </IconButton>
        </>
      ) : null}
      {(status === 'preview' || status === 'sending') && previewUrl ? (
        <>
          <audio
            ref={audio}
            src={previewUrl}
            onPlay={(event) => {
              claimAudio(event.currentTarget);
              setPlaying(true);
            }}
            onPause={() => setPlaying(false)}
            onEnded={(event) => {
              releaseAudio(event.currentTarget);
              setPlaying(false);
            }}
          />
          <IconButton
            label={playing ? tr('暂停试听') : tr('试听录音')}
            onClick={() => {
              const player = audio.current;
              if (!player) return;
              if (player.paused) void player.play();
              else player.pause();
            }}
          >
            {playing ? <Pause size={17} /> : <Play size={17} />}
          </IconButton>
          <strong>{formatDuration(elapsedMs)}</strong>
          <IconButton label={tr('删除录音')} disabled={status === 'sending'} onClick={reset}>
            <Trash2 size={16} />
          </IconButton>
          <IconButton
            label={tr('发送语音')}
            disabled={status === 'sending'}
            onClick={() => void send()}
          >
            <SendHorizontal size={17} />
          </IconButton>
        </>
      ) : null}
      {error ? <small>{error}</small> : null}
      {status === 'idle' && error ? (
        <button className="secondary-button" type="button" onClick={() => void start()}>
          {tr('重试')}
        </button>
      ) : null}
    </div>
  );
}

function preferredMimeType(): string {
  return (
    ['audio/webm;codecs=opus', 'audio/ogg;codecs=opus', 'audio/webm'].find((mime) =>
      MediaRecorder.isTypeSupported(mime),
    ) ?? ''
  );
}

function normalizeAudioMime(value: string): string {
  const mime = value.split(';')[0]?.toLowerCase();
  return mime === 'audio/ogg' ? 'audio/ogg' : 'audio/webm';
}

function stopStream(value: MediaStream | null) {
  value?.getTracks().forEach((track) => track.stop());
}

function formatDuration(milliseconds: number): string {
  const seconds = Math.ceil(milliseconds / 1_000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}
