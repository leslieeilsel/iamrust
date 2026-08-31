let activeAudio: HTMLAudioElement | null = null;

export function claimAudio(audio: HTMLAudioElement): void {
  if (activeAudio && activeAudio !== audio) activeAudio.pause();
  activeAudio = audio;
}

export function releaseAudio(audio: HTMLAudioElement): void {
  if (activeAudio === audio) activeAudio = null;
}
