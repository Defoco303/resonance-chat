import { browser } from '$app/environment';

export const DEFAULT_AIVIS_URL = 'http://127.0.0.1:10101';

export interface AivisStyle {
  id: number;
  name: string;
  speakerName: string;
  displayName: string;
}

interface AivisSpeakerResponse {
  name: string;
  styles?: Array<{
    id: number;
    name: string;
  }>;
}

type SpeakOptions = {
  baseUrl: string;
  text: string;
  speakerId: number;
  rate: number;
  volume: number;
};

let activeRequestId = 0;
let activeController: AbortController | null = null;
let activeAudio: HTMLAudioElement | null = null;
let activeAudioUrl: string | null = null;
let activePlaybackReject: ((reason?: unknown) => void) | null = null;

function normalizeBaseUrl(baseUrl: string) {
  const trimmed = (baseUrl || DEFAULT_AIVIS_URL).trim();
  return trimmed.replace(/\/+$/, '') || DEFAULT_AIVIS_URL;
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function cleanupAudio() {
  if (activeAudio) {
    activeAudio.onended = null;
    activeAudio.onerror = null;
    activeAudio.pause();
    activeAudio.src = '';
    activeAudio = null;
  }
  if (activeAudioUrl) {
    URL.revokeObjectURL(activeAudioUrl);
    activeAudioUrl = null;
  }
}

function makeErrorMessage(error: unknown) {
  if (error instanceof DOMException && error.name === 'AbortError') {
    return 'Speech request was cancelled.';
  }
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return 'Failed to reach AivisSpeech.';
}

async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, init);
  if (!res.ok) {
    throw new Error(`AivisSpeech returned an error. (${res.status})`);
  }
  return res.json() as Promise<T>;
}

export function stopAivisPlayback() {
  activeRequestId += 1;
  if (activeController) {
    activeController.abort();
    activeController = null;
  }
  if (activePlaybackReject) {
    activePlaybackReject(new DOMException('Speech request was cancelled.', 'AbortError'));
    activePlaybackReject = null;
  }
  cleanupAudio();
}

export async function fetchAivisStyles(baseUrl: string): Promise<AivisStyle[]> {
  if (!browser) return [];
  const speakers = await fetchJson<AivisSpeakerResponse[]>(
    `${normalizeBaseUrl(baseUrl)}/speakers`
  );
  return speakers.flatMap((speaker) =>
    (speaker.styles ?? []).map((style) => ({
      id: style.id,
      name: style.name,
      speakerName: speaker.name,
      displayName: `${speaker.name} / ${style.name}`,
    }))
  );
}

export async function speakWithAivis({
  baseUrl,
  text,
  speakerId,
  rate,
  volume,
}: SpeakOptions) {
  if (!browser) return;
  const requestId = activeRequestId + 1;
  activeRequestId = requestId;

  if (activeController) activeController.abort();
  cleanupAudio();

  const controller = new AbortController();
  activeController = controller;

  const root = normalizeBaseUrl(baseUrl);
  const speaker = String(speakerId);
  const queryParams = new URLSearchParams({ speaker, text });

  try {
    const audioQuery = await fetchJson<Record<string, unknown>>(
      `${root}/audio_query?${queryParams.toString()}`,
      {
        method: 'POST',
        signal: controller.signal,
      }
    );

    audioQuery.speedScale = clamp(rate, 0.5, 2.0);

    const synthRes = await fetch(`${root}/synthesis?speaker=${encodeURIComponent(speaker)}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(audioQuery),
      signal: controller.signal,
    });
    if (!synthRes.ok) {
      throw new Error(`AivisSpeech synthesis failed. (${synthRes.status})`);
    }
    const blob = await synthRes.blob();
    if (requestId !== activeRequestId) return;

    const audioUrl = URL.createObjectURL(blob);
    const audio = new Audio(audioUrl);
    audio.volume = clamp(volume, 0, 1);
    activeAudio = audio;
    activeAudioUrl = audioUrl;
    activeController = null;
    const playbackDone = new Promise<void>((resolve, reject) => {
      activePlaybackReject = reject;
      audio.onended = () => {
        if (activeAudio === audio) {
          activePlaybackReject = null;
          cleanupAudio();
        }
        resolve();
      };
      audio.onerror = () => {
        if (activeAudio === audio) {
          activePlaybackReject = null;
          cleanupAudio();
        }
        reject(new Error('AivisSpeech playback failed.'));
      };
    });
    await audio.play();
    await playbackDone;
  } catch (error) {
    if (requestId === activeRequestId) {
      activeController = null;
      activePlaybackReject = null;
      cleanupAudio();
    }
    if (error instanceof DOMException && error.name === 'AbortError') return;
    throw new Error(makeErrorMessage(error));
  }
}
