<script lang="ts">
  import {
    DEFAULT_AIVIS_URL,
    fetchAivisStyles,
    speakWithAivis,
    stopAivisPlayback,
    type AivisStyle,
  } from '$lib/aivis';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
  import { onMount } from 'svelte';

  interface ChatMessage {
    channel: number; channel_name: string; sender_id: number;
    sender_name: string; text: string; timestamp: number;
  }

  interface CaptureStatus {
    code: string;
    level: string;
    message: string;
  }

  // TTS pills: ワールド -> ギルド -> パーティ -> チャンネル
  const CH_PILLS = [
    { id: 1, name: 'ワールド', css: '--c-world' },
    { id: 4, name: 'ギルド', css: '--c-guild' },
    { id: 3, name: 'パーティ', css: '--c-party' },
    { id: 2, name: 'チャンネル', css: '--c-channel' },
  ] as const;
  const DISPLAY_CHANNEL_IDS = CH_PILLS.map(({ id }) => id);

  const THEMES = [
    { id: 'dark',     name: 'ダーク',       colors: ['#0d0d17', '#16163a', '#a78bfa'] },
    { id: 'light',    name: 'ライト',       colors: ['#f8fafc', '#e2e8f0', '#7c3aed'] },
    { id: 'oled',     name: 'OLED',        colors: ['#000000', '#080808', '#d8b4fe'] },
    { id: 'cyber',    name: 'サイバー',     colors: ['#00050f', '#000c1c', '#00ff88'] },
    { id: 'sakura',   name: 'サクラ',       colors: ['#12050c', '#1e0814', '#f472b6'] },
    { id: 'midnight', name: 'ミッドナイト', colors: ['#050819', '#0a0f2d', '#c4b5fd'] },
    { id: 'nord',     name: 'ノルド',       colors: ['#242938', '#2e3448', '#b48ead'] },
  ] as const;

  // Custom theme
  interface CustomTheme {
    bg: string; surface: string; border: string;
    text: string; textDim: string; accent: string;
    cWorld: string; cGuild: string; cParty: string; cChannel: string;
  }
  const DEFAULT_CUSTOM: CustomTheme = {
    bg: '#0d0d17', surface: '#16163a', border: '#2d2d4e',
    text: '#e2e8f0', textDim: '#4a5568', accent: '#a78bfa',
    cWorld: '#a78bfa', cGuild: '#34d399', cParty: '#60a5fa', cChannel: '#e2e8f0',
  };

  // Settings
  type SplitMode = 1 | 2 | 3;
  type PaneId = 'a' | 'b' | 'c';
  type TtsProvider = 'aivis' | 'system';
  type TtsPlaybackMode = 'interrupt' | 'queue' | 'latest_after_current';
  type PanelKind = 'general' | 'tts' | 'speaker' | 'dictionary';

  type TtsDictionaryEntry = {
    id: string;
    from: string;
    to: string;
  };

  type TtsSpeakerVoiceEntry = {
    id: string;
    speakerName: string;
    styleId: string;
    styleLabel: string;
  };

  type QueuedSpeech = {
    text: string;
    speakerName?: string;
  };

  interface Settings {
    theme: string; fontSize: number; bgOpacity: number; alwaysOnTop: boolean;
    ttsProvider: TtsProvider;
    ttsPlaybackMode: TtsPlaybackMode;
    ttsEnabled: boolean; ttsRate: number; ttsVolume: number;
    ttsChannels: number[]; ttsVoice: string; ttsSystemVoice: string; ttsEngineUrl: string;
    ttsDictionary: TtsDictionaryEntry[];
    ttsSpeakerVoices: TtsSpeakerVoiceEntry[];
    splitMode: SplitMode;
    displayChannelsA: number[];
    displayChannelsB: number[];
    displayChannelsC: number[];
    splitRatios2: number[];
    splitRatios3: number[];
    notifyEnabled: boolean; notifyKeywords: string;
    notifyVolume: number; notifyHasCustomSound: boolean;
    customTheme: CustomTheme;
    blacklist: string[];
  }
  const DEFAULTS: Settings = {
    theme: 'dark', fontSize: 13, bgOpacity: 0.92, alwaysOnTop: true,
    ttsProvider: 'system',
    ttsPlaybackMode: 'interrupt',
    ttsEnabled: false, ttsRate: 1.0, ttsVolume: 0.8,
    ttsChannels: [1, 2, 3, 4], ttsVoice: '', ttsSystemVoice: '', ttsEngineUrl: DEFAULT_AIVIS_URL,
    ttsDictionary: [],
    ttsSpeakerVoices: [],
    splitMode: 1,
    displayChannelsA: [1, 2, 3, 4],
    displayChannelsB: [1, 2, 3, 4],
    displayChannelsC: [1, 2, 3, 4],
    splitRatios2: [0.5, 0.5],
    splitRatios3: [0.34, 0.33, 0.33],
    notifyEnabled: false, notifyKeywords: '', notifyVolume: 0.8, notifyHasCustomSound: false,
    customTheme: { ...DEFAULT_CUSTOM },
    blacklist: [],
  };

  // State
  let s          = $state<Settings>({ ...DEFAULTS, customTheme: { ...DEFAULT_CUSTOM } });
  let messages   = $state<ChatMessage[]>([]);
  let panelOpen  = $state(false);
  let panelKind  = $state<PanelKind>('general');
  let chatLayoutEl = $state<HTMLElement | null>(null);
  let listElA    = $state<HTMLElement | null>(null);
  let listElB    = $state<HTMLElement | null>(null);
  let listElC    = $state<HTMLElement | null>(null);
  let autoScrollA = $state(true);
  let autoScrollB = $state(true);
  let autoScrollC = $state(true);
  let aivisStyles = $state<AivisStyle[]>([]);
  let systemVoices = $state<SpeechSynthesisVoice[]>([]);
  let ttsStatusMessage = $state('');
  let ttsStatusError = $state(false);
  let ttsLoadingVoices = $state(false);
  let ttsQueue = $state<QueuedSpeech[]>([]);
  let ttsQueueRunning = $state(false);
  let ttsQueueRevision = $state(0);
  let ttsDictionaryDraftFrom = $state('');
  let ttsDictionaryDraftTo = $state('');
  let ttsSpeakerDraftName = $state('');
  let ttsSpeakerDraftStyleId = $state('');
  let soundFileName = $state('');
  let blockPopup = $state<{ name: string; x: number; y: number } | null>(null);

  let notifyAudio: HTMLAudioElement | null = null;
  let audioCtx: AudioContext | null = null;
  let captureStatus = $state<CaptureStatus | null>(null);

  function togglePanel(kind: PanelKind) {
    if (panelOpen && panelKind === kind) {
      panelOpen = false;
      return;
    }
    panelKind = kind;
    panelOpen = true;
    blockPopup = null;
    if (kind === 'speaker') {
      if (!aivisStyles.length) {
        void refreshAivisVoices();
      } else if (!ttsSpeakerDraftStyleId || !hasAivisStyle(ttsSpeakerDraftStyleId)) {
        ttsSpeakerDraftStyleId = String(aivisStyles[0].id);
      }
    }
  }

  function closePanel() {
    panelOpen = false;
  }

  function createTtsDictionaryId() {
    if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
      return crypto.randomUUID();
    }
    return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  }

  function normalizeTtsDictionary(raw: unknown): TtsDictionaryEntry[] {
    if (!Array.isArray(raw)) return [];
    return raw.flatMap((entry) => {
      if (!entry || typeof entry !== 'object') return [];
      const item = entry as Partial<TtsDictionaryEntry>;
      return [{
        id: typeof item.id === 'string' && item.id ? item.id : createTtsDictionaryId(),
        from: typeof item.from === 'string' ? item.from : '',
        to: typeof item.to === 'string' ? item.to : '',
      }];
    });
  }

  function normalizeTtsSpeakerVoices(raw: unknown): TtsSpeakerVoiceEntry[] {
    if (!Array.isArray(raw)) return [];
    return raw.flatMap((entry) => {
      if (!entry || typeof entry !== 'object') return [];
      const item = entry as Partial<TtsSpeakerVoiceEntry>;
      return [{
        id: typeof item.id === 'string' && item.id ? item.id : createTtsDictionaryId(),
        speakerName: typeof item.speakerName === 'string' ? item.speakerName : '',
        styleId: typeof item.styleId === 'string' ? item.styleId : '',
        styleLabel: typeof item.styleLabel === 'string' ? item.styleLabel : '',
      }];
    });
  }

  const SPLITTER_SIZE_PX = 10;
  const MIN_PANE_HEIGHT_PX = 74;
  let resizeDrag = $state<{
    mode: 2 | 3;
    splitterIndex: 0 | 1;
    startY: number;
    startRatios: number[];
    availableHeight: number;
  } | null>(null);


  // Theme helpers
  function hexToRgb(hex: string) {
    const n = parseInt(hex.replace('#', ''), 16);
    return `${(n >> 16) & 255} ${(n >> 8) & 255} ${n & 255}`;
  }

  function applyCustomVars(ct: CustomTheme) {
    const r = document.documentElement;
    r.style.setProperty('--bg',      hexToRgb(ct.bg));
    r.style.setProperty('--surface', hexToRgb(ct.surface));
    r.style.setProperty('--border',  ct.border);
    r.style.setProperty('--text',    ct.text);
    r.style.setProperty('--text-dim',ct.textDim);
    r.style.setProperty('--accent',  ct.accent);
    r.style.setProperty('--thumb',   ct.accent);
    const m = ct.accent.replace('#','').match(/.{2}/g)!.map(x => parseInt(x,16));
    r.style.setProperty('--accent-glow', `rgba(${m[0]},${m[1]},${m[2]},0.35)`);
    r.style.setProperty('--input-bg', 'rgba(255 255 255 / 0.05)');
    r.style.setProperty('--c-world',   ct.cWorld);
    r.style.setProperty('--c-guild',   ct.cGuild);
    r.style.setProperty('--c-party',   ct.cParty);
    r.style.setProperty('--c-channel', ct.cChannel);
  }

  const CUSTOM_PROPS = [
    '--bg','--surface','--border','--text','--text-dim',
    '--accent','--accent-glow','--input-bg','--thumb',
    '--c-world','--c-guild','--c-party','--c-channel',
  ];

  function applyCSS(cfg: Settings) {
    const root = document.documentElement;
    root.setAttribute('data-theme', cfg.theme);
    root.style.setProperty('--font-size',  `${cfg.fontSize}px`);
    root.style.setProperty('--bg-opacity', String(cfg.bgOpacity));
    if (cfg.theme === 'custom') {
      applyCustomVars(cfg.customTheme);
    } else {
      // Remove inline vars left by custom theme so stylesheet rules take effect again
      for (const p of CUSTOM_PROPS) root.style.removeProperty(p);
    }
  }

  function save() { localStorage.setItem('rchat', JSON.stringify(s)); applyCSS(s); }

  function selectTheme(id: string) { s.theme = id; save(); }

  function chColorById(id: number) {
    switch (id) {
      case 1: return 'var(--c-world)';
      case 2: return 'var(--c-channel)';
      case 3: return 'var(--c-party)';
      case 4: return 'var(--c-guild)';
      default: return 'var(--text-dim)';
    }
  }

  function fmt(ts: number) {
    return new Date(ts * 1000).toLocaleTimeString('ja-JP', {
      hour: '2-digit', minute: '2-digit', second: '2-digit'
    });
  }

  // Audio
  function playBeep(volume: number) {
    try {
      if (!audioCtx) audioCtx = new AudioContext();
      const osc = audioCtx.createOscillator();
      const gain = audioCtx.createGain();
      osc.connect(gain); gain.connect(audioCtx.destination);
      osc.type = 'sine'; osc.frequency.value = 1050;
      gain.gain.setValueAtTime(volume * 0.5, audioCtx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.0001, audioCtx.currentTime + 0.35);
      osc.start(audioCtx.currentTime); osc.stop(audioCtx.currentTime + 0.35);
    } catch {}
  }

  function playNotification() {
    if (notifyAudio) {
      notifyAudio.volume = s.notifyVolume;
      notifyAudio.currentTime = 0;
      notifyAudio.play().catch(() => playBeep(s.notifyVolume));
    } else { playBeep(s.notifyVolume); }
  }

  function handleSoundFile(e: Event) {
    const file = (e.target as HTMLInputElement).files?.[0];
    if (!file) return;
    if (file.size > 3 * 1024 * 1024) { alert('ファイルサイズが大きすぎます。3MB以下にしてください。'); return; }
    const reader = new FileReader();
    reader.onload = () => {
      try {
        const data = reader.result as string;
        localStorage.setItem('rchat-notify-sound', data);
        notifyAudio = new Audio(data);
        soundFileName = file.name;
        s.notifyHasCustomSound = true; save();
      } catch { alert('読み込みに失敗しました。'); }
    };
    reader.readAsDataURL(file);
  }

  function clearCustomSound() {
    localStorage.removeItem('rchat-notify-sound');
    notifyAudio = null; soundFileName = '';
    s.notifyHasCustomSound = false; save();
  }

  // Text helpers
  function isStamp(text: string) { return text.startsWith('emojiPic='); }
  function stripSprites(text: string) { return text.replace(/<sprite=\d+>/g, '').trim(); }
  function displayText(text: string) { return isStamp(text) ? null : stripSprites(text); }

  function getKeywords(): string[] {
    if (!s.notifyEnabled || !s.notifyKeywords.trim()) return [];
    return s.notifyKeywords.split(/[\n,、]/).map(k => k.trim()).filter(k => k);
  }

  function escapeHtml(text: string): string {
    return text.replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]!));
  }

  function highlightKeywords(text: string): string {
    const kws = getKeywords();
    if (!kws.length) return escapeHtml(text);
    const pattern = kws.map(k => k.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|');
    const re = new RegExp(`(${pattern})`, 'gi');
    return text.split(re).map((part, i) =>
      i % 2 === 1 ? `<mark class="kw-hl">${escapeHtml(part)}</mark>` : escapeHtml(part)
    ).join('');
  }

  function checkKeywords(msg: ChatMessage) {
    if (!s.notifyEnabled || !s.notifyKeywords.trim()) return;
    if (isStamp(msg.text)) return;
    const kws = getKeywords();
    const clean = stripSprites(msg.text);
    if (kws.some(kw => clean.includes(kw))) playNotification();
  }

  // TTS
  function getSelectedTtsStyleId(speakerName?: string) {
    const speakerStyleId = getSpeakerAivisStyleId(speakerName);
    if (speakerStyleId != null) return speakerStyleId;
    const styleId = Number.parseInt(s.ttsVoice, 10);
    if (Number.isInteger(styleId)) return styleId;
    return aivisStyles[0]?.id ?? null;
  }

  function syncSystemVoices() {
    if (!('speechSynthesis' in window)) {
      systemVoices = [];
      return false;
    }
    const all = window.speechSynthesis.getVoices();
    const japanese = all.filter((voice) =>
      voice.lang.startsWith('ja') || voice.name.toLowerCase().includes('japan')
    );
    systemVoices = japanese.length ? japanese : all;
    if (systemVoices.length && !systemVoices.some((voice) => voice.name === s.ttsSystemVoice)) {
      s.ttsSystemVoice = systemVoices[0].name;
      save();
    }
    return systemVoices.length > 0;
  }

  async function refreshAivisVoices(showSuccess = false) {
    ttsLoadingVoices = true;
    ttsStatusError = false;
    ttsStatusMessage = 'AivisSpeech の音声一覧を取得しています...';
    try {
      const styles = await fetchAivisStyles(s.ttsEngineUrl);
      aivisStyles = styles;
      if (!styles.length) {
        ttsStatusError = true;
        ttsStatusMessage = 'AivisSpeech に音声モデルが見つかりません。モデルを追加してから再読み込みしてください。';
        return;
      }
      if (!styles.some((style) => String(style.id) === s.ttsVoice)) {
        s.ttsVoice = String(styles[0].id);
        save();
      }
      if (!ttsSpeakerDraftStyleId || !styles.some((style) => String(style.id) === ttsSpeakerDraftStyleId)) {
        ttsSpeakerDraftStyleId = String(styles[0].id);
      }
      ttsStatusMessage = showSuccess ? `AivisSpeech の音声を ${styles.length} 件読み込みました。` : '';
    } catch {
      aivisStyles = [];
      ttsSpeakerDraftStyleId = '';
      ttsStatusError = true;
      ttsStatusMessage = 'AivisSpeech に接続できません。AivisSpeech を起動した状態で再読み込みしてください。';
    } finally {
      ttsLoadingVoices = false;
    }
  }

  async function refreshSystemVoices(showSuccess = false) {
    ttsLoadingVoices = true;
    ttsStatusError = false;
    ttsStatusMessage = 'Windows 標準音声を確認しています...';
    try {
      if (!('speechSynthesis' in window)) {
        ttsStatusError = true;
        ttsStatusMessage = 'この環境では Windows 標準の読み上げを利用できません。';
        return;
      }
      syncSystemVoices();
      if (!systemVoices.length) {
        ttsStatusError = true;
        ttsStatusMessage = '利用可能な Windows 音声が見つかりません。';
        return;
      }
      ttsStatusMessage = showSuccess ? `Windows 標準音声を ${systemVoices.length} 件読み込みました。` : '';
    } finally {
      ttsLoadingVoices = false;
    }
  }

  function stopCurrentTts() {
    stopAivisPlayback();
    if ('speechSynthesis' in window) window.speechSynthesis.cancel();
  }

  function clearTtsQueue() {
    ttsQueue = [];
    ttsQueueRevision += 1;
    ttsQueueRunning = false;
    stopCurrentTts();
  }

  function getTtsPendingCount() {
    return ttsQueue.length;
  }

  function normalizeSpeakerKey(name: string) {
    return name.trim().toLocaleLowerCase('ja-JP');
  }

  function hasAivisStyle(styleId: string) {
    return aivisStyles.some((style) => String(style.id) === styleId);
  }

  function getAivisStyleLabel(styleId: string, fallback = '') {
    return aivisStyles.find((style) => String(style.id) === styleId)?.displayName
      ?? fallback
      ?? '';
  }

  function getActiveTtsDictionary() {
    return s.ttsDictionary
      .filter((entry) => entry.from.trim() && entry.to.trim())
      .slice()
      .sort((a, b) => b.from.trim().length - a.from.trim().length);
  }

  function getActiveTtsDictionaryCount() {
    return getActiveTtsDictionary().length;
  }

  function applyTtsDictionary(text: string) {
    let result = text;
    for (const entry of getActiveTtsDictionary()) {
      result = result.split(entry.from.trim()).join(entry.to.trim());
    }
    return result;
  }

  function updateTtsDictionaryEntry(id: string, key: 'from' | 'to', value: string) {
    s.ttsDictionary = s.ttsDictionary.map((entry) =>
      entry.id === id ? { ...entry, [key]: value } : entry
    );
    save();
  }

  function addTtsDictionaryEntry() {
    const from = ttsDictionaryDraftFrom.trim();
    const to = ttsDictionaryDraftTo.trim();
    if (!from || !to) return;
    s.ttsDictionary = [...s.ttsDictionary, {
      id: createTtsDictionaryId(),
      from,
      to,
    }];
    ttsDictionaryDraftFrom = '';
    ttsDictionaryDraftTo = '';
    save();
  }

  function removeTtsDictionaryEntry(id: string) {
    s.ttsDictionary = s.ttsDictionary.filter((entry) => entry.id !== id);
    save();
  }

  function getActiveTtsSpeakerVoices() {
    return s.ttsSpeakerVoices.filter((entry) => entry.speakerName.trim() && entry.styleId);
  }

  function getActiveTtsSpeakerVoiceCount() {
    return getActiveTtsSpeakerVoices().length;
  }

  function updateTtsSpeakerVoiceName(id: string, value: string) {
    s.ttsSpeakerVoices = s.ttsSpeakerVoices.map((entry) =>
      entry.id === id ? { ...entry, speakerName: value } : entry
    );
    save();
  }

  function updateTtsSpeakerVoiceStyle(id: string, styleId: string) {
    s.ttsSpeakerVoices = s.ttsSpeakerVoices.map((entry) =>
      entry.id === id
        ? { ...entry, styleId, styleLabel: getAivisStyleLabel(styleId, entry.styleLabel) }
        : entry
    );
    save();
  }

  function addTtsSpeakerVoiceEntry() {
    const speakerName = ttsSpeakerDraftName.trim();
    const styleId = ttsSpeakerDraftStyleId;
    if (!speakerName || !styleId) return;
    s.ttsSpeakerVoices = [...s.ttsSpeakerVoices, {
      id: createTtsDictionaryId(),
      speakerName,
      styleId,
      styleLabel: getAivisStyleLabel(styleId),
    }];
    ttsSpeakerDraftName = '';
    save();
  }

  function removeTtsSpeakerVoiceEntry(id: string) {
    s.ttsSpeakerVoices = s.ttsSpeakerVoices.filter((entry) => entry.id !== id);
    save();
  }

  function getSpeakerAivisStyleId(speakerName?: string) {
    if (!speakerName) return null;
    const key = normalizeSpeakerKey(speakerName);
    if (!key) return null;
    const match = getActiveTtsSpeakerVoices().find((entry) => normalizeSpeakerKey(entry.speakerName) === key);
    if (!match || !hasAivisStyle(match.styleId)) return null;
    const styleId = Number.parseInt(match.styleId, 10);
    return Number.isInteger(styleId) ? styleId : null;
  }

  function setTtsProvider(provider: TtsProvider) {
    s.ttsProvider = provider;
    ttsStatusError = false;
    ttsStatusMessage = '';
    save();
    clearTtsQueue();
    if (provider === 'aivis') {
      void refreshAivisVoices();
    } else {
      void refreshSystemVoices();
    }
  }

  function setTtsPlaybackMode(mode: TtsPlaybackMode) {
    s.ttsPlaybackMode = mode;
    save();
    clearTtsQueue();
  }

  function isTtsCancelError(error: unknown) {
    return error instanceof DOMException && error.name === 'AbortError';
  }

  function speakWithSystem(text: string): Promise<void> {
    if (!('speechSynthesis' in window)) {
      throw new Error('この環境では Windows 標準の読み上げを利用できません。');
    }
    if (!systemVoices.length) syncSystemVoices();
    if (!systemVoices.length) {
      throw new Error('利用可能な Windows 音声が見つかりません。');
    }
    return new Promise((resolve, reject) => {
      const utterance = new SpeechSynthesisUtterance(text);
      utterance.lang = 'ja-JP';
      utterance.rate = s.ttsRate;
      utterance.volume = s.ttsVolume;
      const voice = systemVoices.find((item) => item.name === s.ttsSystemVoice);
      if (voice) utterance.voice = voice;
      utterance.onend = () => resolve();
      utterance.onerror = (event) => {
        if (event.error === 'canceled' || event.error === 'interrupted') {
          reject(new DOMException('Speech request was cancelled.', 'AbortError'));
          return;
        }
        reject(new Error('Windows 標準音声の再生に失敗しました。'));
      };
      window.speechSynthesis.cancel();
      window.speechSynthesis.speak(utterance);
    });
  }

  async function speakText(text: string, speakerName?: string) {
    const normalizedText = applyTtsDictionary(text);
    if (s.ttsProvider === 'system') {
      try {
        await speakWithSystem(normalizedText);
        ttsStatusError = false;
        ttsStatusMessage = '';
      } catch (error) {
        if (isTtsCancelError(error)) {
          ttsStatusError = false;
          ttsStatusMessage = '';
          return;
        }
        ttsStatusError = true;
        ttsStatusMessage = error instanceof Error
          ? error.message
          : 'この環境では Windows 標準の読み上げを利用できません。';
      }
      return;
    }

    const styleId = getSelectedTtsStyleId(speakerName);
    if (styleId == null) {
      if (!ttsLoadingVoices) await refreshAivisVoices();
    }
    const nextStyleId = getSelectedTtsStyleId(speakerName);
    if (nextStyleId == null) {
      try {
        await speakWithSystem(normalizedText);
        ttsStatusError = false;
        ttsStatusMessage = 'AivisSpeech の音声が使えないため、Windows 標準音声で読み上げています。';
      } catch (fallbackError) {
        ttsStatusError = true;
        if (!ttsStatusMessage) {
          ttsStatusMessage = fallbackError instanceof Error
            ? `AivisSpeech を利用できず、${fallbackError.message}`
            : '読み上げに使う音声がまだ選べていません。AivisSpeech を確認してください。';
        }
      }
      return;
    }
    try {
      await speakWithAivis({
        baseUrl: s.ttsEngineUrl,
        text: normalizedText,
        speakerId: nextStyleId,
        rate: s.ttsRate,
        volume: s.ttsVolume,
      });
      ttsStatusError = false;
      ttsStatusMessage = '';
    } catch (error) {
      if (isTtsCancelError(error) || (error instanceof Error && error.message.includes('cancelled'))) {
        ttsStatusError = false;
        ttsStatusMessage = '';
        return;
      }
      try {
        await speakWithSystem(normalizedText);
        ttsStatusError = false;
        ttsStatusMessage = 'AivisSpeech に接続できないため、Windows 標準音声で読み上げています。';
      } catch (fallbackError) {
        ttsStatusError = true;
        ttsStatusMessage = fallbackError instanceof Error
          ? `AivisSpeech に接続できず、${fallbackError.message}`
          : 'AivisSpeech での読み上げに失敗しました。エンジンの起動状態を確認してください。';
      }
    }
  }

  async function runTtsQueue(revision: number) {
    if (ttsQueueRunning) return;
    ttsQueueRunning = true;
    try {
      while (ttsQueue.length > 0 && revision === ttsQueueRevision) {
        const [next, ...rest] = ttsQueue;
        ttsQueue = rest;
        await speakText(next.text, next.speakerName);
      }
    } finally {
      if (revision === ttsQueueRevision) {
        ttsQueueRunning = false;
      }
    }
  }

  function queueSpeech(text: string, speakerName?: string) {
    if (s.ttsPlaybackMode === 'interrupt') {
      ttsQueue = [];
      ttsQueueRevision += 1;
      stopCurrentTts();
      void speakText(text, speakerName);
      return;
    }
    if (s.ttsPlaybackMode === 'latest_after_current') {
      ttsQueue = [{ text, speakerName }];
      const revision = ttsQueueRevision;
      if (!ttsQueueRunning) {
        void runTtsQueue(revision);
      }
      return;
    }
    ttsQueue = [...ttsQueue, { text, speakerName }];
    const revision = ttsQueueRevision;
    if (!ttsQueueRunning) {
      void runTtsQueue(revision);
    }
  }

  function speak(msg: ChatMessage) {
    if (!s.ttsEnabled || !s.ttsChannels.includes(msg.channel)) return;
    if (isStamp(msg.text)) return;
    const clean = stripSprites(msg.text);
    if (!clean) return;
    queueSpeech(`${msg.sender_name}、${clean}`, msg.sender_name);
  }

  function testTts() {
    queueSpeech('テスト、ワールドチャット');
  }

  function toggleTtsCh(id: number) {
    s.ttsChannels = s.ttsChannels.includes(id)
      ? s.ttsChannels.filter(c => c !== id) : [...s.ttsChannels, id];
    save();
  }

  function filterMessages(channels: number[]) {
    return messages
      .filter(m => channels.includes(m.channel))
      .filter(m => !s.blacklist.includes(m.sender_name));
  }

  const filteredPrimary = $derived(filterMessages(s.displayChannelsA));
  const filteredSecondary = $derived(filterMessages(s.displayChannelsB));
  const filteredTertiary = $derived(filterMessages(s.displayChannelsC));

  function normalizeDisplayChannels(raw: number[] | undefined) {
    if (!Array.isArray(raw)) return [...DISPLAY_CHANNEL_IDS];
    return DISPLAY_CHANNEL_IDS.filter(id => raw.includes(id));
  }

  function normalizeRatios(raw: number[], paneCount: 2 | 3) {
    const fallback = paneCount === 2 ? DEFAULTS.splitRatios2 : DEFAULTS.splitRatios3;
    const source = Array.isArray(raw) && raw.length === paneCount ? raw : fallback;
    const values = source.map(value => Number.isFinite(value) && value > 0 ? value : 0);
    const sum = values.reduce((acc, value) => acc + value, 0);
    if (sum <= 0) return [...fallback];
    return values.map(value => value / sum);
  }

  function getPaneRatios(mode: 2 | 3) {
    return mode === 2 ? normalizeRatios(s.splitRatios2, 2) : normalizeRatios(s.splitRatios3, 3);
  }

  function setPaneRatios(mode: 2 | 3, ratios: number[]) {
    const normalized = normalizeRatios(ratios, mode);
    if (mode === 2) {
      s.splitRatios2 = normalized;
    } else {
      s.splitRatios3 = normalized;
    }
  }

  const gridTemplateRows = $derived.by(() => {
    if (s.splitMode === 1) return 'minmax(0, 1fr)';
    const mode = s.splitMode === 3 ? 3 : 2;
    const ratios = getPaneRatios(mode);
    return ratios
      .map((ratio, index) => (index > 0 ? String(SPLITTER_SIZE_PX) + 'px ' : '') + 'minmax(' + MIN_PANE_HEIGHT_PX + 'px, ' + ratio + 'fr)')
      .join(' ');
  });

  function getDisplayChannels(pane: PaneId) {
    switch (pane) {
      case 'a': return s.displayChannelsA;
      case 'b': return s.displayChannelsB;
      case 'c': return s.displayChannelsC;
    }
  }

  function setDisplayChannels(pane: PaneId, channels: number[]) {
    const ordered = normalizeDisplayChannels(channels);
    if (pane === 'a') {
      s.displayChannelsA = ordered;
    } else if (pane === 'b') {
      s.displayChannelsB = ordered;
    } else {
      s.displayChannelsC = ordered;
    }
  }

  function isAllDisplaySelected(channels: number[]) {
    return DISPLAY_CHANNEL_IDS.every(id => channels.includes(id));
  }

  function selectAllDisplayChannels(pane: PaneId) {
    setDisplayChannels(pane, [...DISPLAY_CHANNEL_IDS]);
    save();
  }

  function toggleDisplayChannel(pane: PaneId, id: number) {
    const current = getDisplayChannels(pane);
    const next = current.includes(id)
      ? current.filter(c => c !== id)
      : [...current, id];
    setDisplayChannels(pane, next);
    save();
  }

  function cycleSplitMode() {
    endResize();
    const nextMode: SplitMode = s.splitMode === 1 ? 2 : s.splitMode === 2 ? 3 : 1;
    if (nextMode >= 2 && s.displayChannelsB.length === 0) {
      s.displayChannelsB = [...DISPLAY_CHANNEL_IDS];
    }
    if (nextMode === 3 && s.displayChannelsC.length === 0) {
      s.displayChannelsC = [...DISPLAY_CHANNEL_IDS];
    }
    if (nextMode === 2) {
      setPaneRatios(2, s.splitRatios2);
    } else if (nextMode === 3) {
      setPaneRatios(3, s.splitRatios3);
    }
    s.splitMode = nextMode;
    save();
  }

  function beginResize(splitterIndex: 0 | 1, event: PointerEvent) {
    const mode = s.splitMode;
    if ((mode !== 2 && mode !== 3) || !chatLayoutEl) return;
    event.preventDefault();
    const availableHeight = chatLayoutEl.clientHeight - (mode - 1) * SPLITTER_SIZE_PX;
    if (availableHeight <= MIN_PANE_HEIGHT_PX * mode) return;
    resizeDrag = {
      mode,
      splitterIndex,
      startY: event.clientY,
      startRatios: getPaneRatios(mode),
      availableHeight,
    };
    document.body.classList.add('split-resizing');
    window.addEventListener('pointermove', handleResizeMove);
    window.addEventListener('pointerup', endResize);
    window.addEventListener('pointercancel', endResize);
  }

  function handleResizeMove(event: PointerEvent) {
    if (!resizeDrag) return;
    event.preventDefault();
    const next = [...resizeDrag.startRatios];
    const leftIndex = resizeDrag.splitterIndex;
    const rightIndex = leftIndex + 1;
    const pairTotal = resizeDrag.startRatios[leftIndex] + resizeDrag.startRatios[rightIndex];
    const minRatio = MIN_PANE_HEIGHT_PX / resizeDrag.availableHeight;
    const nextLeft = Math.min(
      pairTotal - minRatio,
      Math.max(minRatio, resizeDrag.startRatios[leftIndex] + (event.clientY - resizeDrag.startY) / resizeDrag.availableHeight)
    );
    next[leftIndex] = nextLeft;
    next[rightIndex] = pairTotal - nextLeft;
    setPaneRatios(resizeDrag.mode, next);
  }

  function endResize() {
    if (!resizeDrag) return;
    resizeDrag = null;
    document.body.classList.remove('split-resizing');
    window.removeEventListener('pointermove', handleResizeMove);
    window.removeEventListener('pointerup', endResize);
    window.removeEventListener('pointercancel', endResize);
    save();
  }
  function showBlockPopup(e: MouseEvent, name: string) {
    e.stopPropagation();
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    blockPopup = { name, x: rect.left, y: rect.bottom + 4 };
  }

  function blockUser(name: string) {
    if (!s.blacklist.includes(name)) {
      s.blacklist = [...s.blacklist, name];
      save();
    }
    blockPopup = null;
  }

  function unblockUser(name: string) {
    s.blacklist = s.blacklist.filter(n => n !== name);
    save();
  }

  async function setAlwaysOnTop(val: boolean) {
    s.alwaysOnTop = val;
    try { await getCurrentWebviewWindow().setAlwaysOnTop(val); } catch {}
    save();
  }

  function getListEl(which: PaneId) {
    if (which === 'a') return listElA;
    if (which === 'b') return listElB;
    return listElC;
  }

  function setAutoScroll(which: PaneId, value: boolean) {
    if (which === 'a') {
      autoScrollA = value;
    } else if (which === 'b') {
      autoScrollB = value;
    } else {
      autoScrollC = value;
    }
  }

  function onScroll(which: PaneId) {
    const el = getListEl(which);
    if (!el) return;
    setAutoScroll(which, el.scrollHeight - el.scrollTop - el.clientHeight < 80);
  }

  function scrollBottom(which: PaneId) {
    const el = getListEl(which);
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    setAutoScroll(which, true);
  }

  // Mount
  onMount(async () => {
    try {
      const saved = localStorage.getItem('rchat');
      if (saved) {
        const parsed = JSON.parse(saved);
        const legacyVoice = typeof parsed.ttsVoice === 'string' ? parsed.ttsVoice : '';
        const legacyLooksSystemVoice = !!legacyVoice && !/^-?\d+$/.test(legacyVoice);
        const parsedProvider: TtsProvider =
          parsed.ttsProvider === 'aivis' || parsed.ttsProvider === 'system'
            ? parsed.ttsProvider
            : legacyLooksSystemVoice
              ? 'system'
              : parsed.ttsEngineUrl || legacyVoice
                ? 'aivis'
                : DEFAULTS.ttsProvider;
        const parsedPlaybackMode: TtsPlaybackMode =
          parsed.ttsPlaybackMode === 'queue'
            || parsed.ttsPlaybackMode === 'interrupt'
            || parsed.ttsPlaybackMode === 'latest_after_current'
            ? parsed.ttsPlaybackMode
            : DEFAULTS.ttsPlaybackMode;
        const splitMode: SplitMode = parsed.splitMode === 2 || parsed.splitMode === 3 || parsed.splitMode === 1
          ? parsed.splitMode
          : parsed.splitView ? 2 : 1;
        s = {
          ...DEFAULTS,
          customTheme: { ...DEFAULT_CUSTOM },
          ...parsed,
          ttsProvider: parsedProvider,
          ttsPlaybackMode: parsedPlaybackMode,
          ttsDictionary: normalizeTtsDictionary(parsed.ttsDictionary),
          ttsSpeakerVoices: normalizeTtsSpeakerVoices(parsed.ttsSpeakerVoices),
          ttsVoice: legacyLooksSystemVoice ? '' : legacyVoice,
          ttsSystemVoice: typeof parsed.ttsSystemVoice === 'string'
            ? parsed.ttsSystemVoice
            : legacyLooksSystemVoice
              ? legacyVoice
              : DEFAULTS.ttsSystemVoice,
          splitMode,
          displayChannelsA: normalizeDisplayChannels(parsed.displayChannelsA),
          displayChannelsB: normalizeDisplayChannels(parsed.displayChannelsB),
          displayChannelsC: normalizeDisplayChannels(parsed.displayChannelsC),
          splitRatios2: normalizeRatios(parsed.splitRatios2, 2),
          splitRatios3: normalizeRatios(parsed.splitRatios3, 3),
          customTheme: { ...DEFAULT_CUSTOM, ...(parsed.customTheme ?? {}) },
        };
      }
    } catch {}
    if (!s.ttsEngineUrl?.trim()) s.ttsEngineUrl = DEFAULT_AIVIS_URL;
    if (s.ttsVoice && !/^-?\d+$/.test(s.ttsVoice)) s.ttsVoice = '';
    applyCSS(s);
    try { await getCurrentWebviewWindow().setAlwaysOnTop(s.alwaysOnTop); } catch {}
    if (s.notifyHasCustomSound) {
      const data = localStorage.getItem('rchat-notify-sound');
      if (data) { notifyAudio = new Audio(data); soundFileName = '(保存済み)'; }
      else { s.notifyHasCustomSound = false; }
    }
    if ('speechSynthesis' in window) {
      const syncVoices = () => {
        const hasVoices = syncSystemVoices();
        if (s.ttsProvider === 'system' && hasVoices && ttsStatusError) {
          ttsStatusError = false;
          ttsStatusMessage = '';
        }
      };
      syncVoices();
      window.speechSynthesis.onvoiceschanged = syncVoices;
    }
    if (s.ttsProvider === 'aivis' && (s.ttsEnabled || !!s.ttsVoice)) {
      void refreshAivisVoices();
    }
    if (s.ttsProvider === 'system' && (s.ttsEnabled || !!s.ttsSystemVoice)) {
      void refreshSystemVoices();
    }
    const unlistenChat = await listen<ChatMessage>('chat-message', ({ payload: msg }) => {
      messages = [...messages.slice(-999), msg];
      if (autoScrollA && listElA) setTimeout(() => { listElA!.scrollTop = listElA!.scrollHeight; }, 0);
      if (autoScrollB && listElB) setTimeout(() => { listElB!.scrollTop = listElB!.scrollHeight; }, 0);
      if (autoScrollC && listElC) setTimeout(() => { listElC!.scrollTop = listElC!.scrollHeight; }, 0);
      if (!s.blacklist.includes(msg.sender_name)) { void speak(msg); checkKeywords(msg); }
    });
    const unlistenStatus = await listen<CaptureStatus>('capture-status', ({ payload }) => {
      captureStatus = payload;
    });
    return () => {
      clearTtsQueue();
      if ('speechSynthesis' in window) {
        window.speechSynthesis.onvoiceschanged = null;
      }
      unlistenChat();
      unlistenStatus();
    };
  });
</script>

<!-- MARKUP -->
<div class="app" onclick={() => { blockPopup = null; }}>
  <header class="header">
          <div class="header-title">Resonance Chat</div>
    <div class="header-actions">
      <button
        class="pill split-toggle"
        class:on={s.splitMode > 1}
        style:--pc={'var(--accent)'}
        onclick={cycleSplitMode}
      >分割</button>
      <button
        class="panel-icon voice-icon"
        class:active={panelOpen && panelKind === 'tts'}
        onclick={() => togglePanel('tts')}
        title="読み上げ設定"
        aria-label="読み上げ設定"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polygon points="11 5 6 9 3 9 3 15 6 15 11 19 11 5"></polygon>
          <path d="M15.5 8.5a5 5 0 0 1 0 7"></path>
          <path d="M18.5 6a8.5 8.5 0 0 1 0 12"></path>
        </svg>
      </button>
      <button
        class="panel-icon speaker-icon"
        class:active={panelOpen && panelKind === 'speaker'}
        onclick={() => togglePanel('speaker')}
        title="話者別音声"
        aria-label="話者別音声"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M17 21v-2a4 4 0 0 0-4-4H7a4 4 0 0 0-4 4v2"></path>
          <circle cx="10" cy="7" r="4"></circle>
          <path d="M19 8l2 2-2 2"></path>
          <path d="M15 10h6"></path>
        </svg>
      </button>
      <button
        class="panel-icon dictionary-icon"
        class:active={panelOpen && panelKind === 'dictionary'}
        onclick={() => togglePanel('dictionary')}
        title="読み上げ辞書"
        aria-label="読み上げ辞書"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"></path>
          <path d="M6.5 2H20v20H6.5a2.5 2.5 0 0 1 0-5H20"></path>
          <path d="M10 7h5"></path>
          <path d="M10 11h6"></path>
        </svg>
      </button>
      <button
        class="panel-icon gear"
        class:active={panelOpen && panelKind === 'general'}
        onclick={() => togglePanel('general')}
        title="設定"
        aria-label="設定"
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3"/>
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
        </svg>
      </button>
    </div>
  </header>

  {#if captureStatus}
    <div class="status-banner" class:error={captureStatus.level === 'error'}>
      <div class="status-copy">
        <strong>チャット取得を開始できませんでした</strong>
        <div>{captureStatus.message}</div>
      </div>
      <button
        class="status-close"
        type="button"
        aria-label="閉じる"
        onclick={() => { captureStatus = null; }}
      >閉じる</button>
    </div>
  {/if}

  <div
    class="chat-layout"
    class:split={s.splitMode > 1}
    class:triple={s.splitMode === 3}
    bind:this={chatLayoutEl}
    style:grid-template-rows={gridTemplateRows}
  >
    <section class="message-pane">
      <div class="filter-bar">
        <div class="ch-pills">
          <button class="pill" class:on={isAllDisplaySelected(s.displayChannelsA)} style:--pc={'var(--accent)'}
            onclick={() => selectAllDisplayChannels('a')}>すべて</button>
          {#each CH_PILLS as ch}
            <button class="pill" class:on={s.displayChannelsA.includes(ch.id)}
              style:--pc={`var(${ch.css})`}
              onclick={() => toggleDisplayChannel('a', ch.id)}>{ch.name}</button>
          {/each}
        </div>
      </div>

      <div class="messages" bind:this={listElA} onscroll={() => onScroll('a')}>
        {#each filteredPrimary as msg (`${msg.timestamp}-${msg.sender_id}-${msg.text}`)}
          <div class="msg">
            <span class="time">{fmt(msg.timestamp)}</span>
            <span class="badge" style:color={chColorById(msg.channel)}>[{msg.channel_name}]</span>
            <span class="name clickable-name" onclick={(e) => showBlockPopup(e, msg.sender_name)}>{msg.sender_name}</span>
            <span class="sep">›</span>
            {#if isStamp(msg.text)}
              <span class="body stamp">スタンプ</span>
            {:else}
              <span class="body">{@html highlightKeywords(stripSprites(msg.text))}</span>
            {/if}
          </div>
        {:else}
          <div class="empty">
            <div class="empty-icon">💬</div>
            <div>チャットを受信中...</div>
            <div class="empty-sub">表示対象に入ったメッセージが届くとここに流れます</div>
          </div>
        {/each}
      </div>

      {#if !autoScrollA}
        <button class="scroll-fab" onclick={() => scrollBottom('a')}>最新へ</button>
      {/if}
    </section>

    {#if s.splitMode > 1}
      <div
        class="splitter"
        role="separator"
        aria-orientation="horizontal"
        aria-label="ペインの高さを調整"
        onpointerdown={(e) => beginResize(0, e)}
      ></div>

      <section class="message-pane split">
        <div class="filter-bar">
          <div class="ch-pills">
            <button class="pill" class:on={isAllDisplaySelected(s.displayChannelsB)} style:--pc={'var(--accent)'}
              onclick={() => selectAllDisplayChannels('b')}>すべて</button>
            {#each CH_PILLS as ch}
              <button class="pill" class:on={s.displayChannelsB.includes(ch.id)}
                style:--pc={`var(${ch.css})`}
                onclick={() => toggleDisplayChannel('b', ch.id)}>{ch.name}</button>
            {/each}
          </div>
        </div>

        <div class="messages" bind:this={listElB} onscroll={() => onScroll('b')}>
          {#each filteredSecondary as msg (`${msg.timestamp}-${msg.sender_id}-${msg.text}`)}
            <div class="msg">
              <span class="time">{fmt(msg.timestamp)}</span>
              <span class="badge" style:color={chColorById(msg.channel)}>[{msg.channel_name}]</span>
              <span class="name clickable-name" onclick={(e) => showBlockPopup(e, msg.sender_name)}>{msg.sender_name}</span>
              <span class="sep">›</span>
              {#if isStamp(msg.text)}
                <span class="body stamp">スタンプ</span>
              {:else}
                <span class="body">{@html highlightKeywords(stripSprites(msg.text))}</span>
              {/if}
            </div>
          {:else}
            <div class="empty">
              <div class="empty-icon">💬</div>
              <div>チャットを受信中...</div>
              <div class="empty-sub">表示対象に入ったメッセージが届くとここに流れます</div>
            </div>
          {/each}
        </div>

        {#if !autoScrollB}
          <button class="scroll-fab" onclick={() => scrollBottom('b')}>最新へ</button>
        {/if}
      </section>
    {/if}

    {#if s.splitMode === 3}
      <div
        class="splitter"
        role="separator"
        aria-orientation="horizontal"
        aria-label="ペインの高さを調整"
        onpointerdown={(e) => beginResize(1, e)}
      ></div>

      <section class="message-pane split">
        <div class="filter-bar">
          <div class="ch-pills">
            <button class="pill" class:on={isAllDisplaySelected(s.displayChannelsC)} style:--pc={'var(--accent)'}
              onclick={() => selectAllDisplayChannels('c')}>すべて</button>
            {#each CH_PILLS as ch}
              <button class="pill" class:on={s.displayChannelsC.includes(ch.id)}
                style:--pc={`var(${ch.css})`}
                onclick={() => toggleDisplayChannel('c', ch.id)}>{ch.name}</button>
            {/each}
          </div>
        </div>

        <div class="messages" bind:this={listElC} onscroll={() => onScroll('c')}>
          {#each filteredTertiary as msg (`${msg.timestamp}-${msg.sender_id}-${msg.text}`)}
            <div class="msg">
              <span class="time">{fmt(msg.timestamp)}</span>
              <span class="badge" style:color={chColorById(msg.channel)}>[{msg.channel_name}]</span>
              <span class="name clickable-name" onclick={(e) => showBlockPopup(e, msg.sender_name)}>{msg.sender_name}</span>
              <span class="sep">›</span>
              {#if isStamp(msg.text)}
                <span class="body stamp">スタンプ</span>
              {:else}
                <span class="body">{@html highlightKeywords(stripSprites(msg.text))}</span>
              {/if}
            </div>
          {:else}
            <div class="empty">
              <div class="empty-icon">💬</div>
              <div>チャットを受信中...</div>
              <div class="empty-sub">表示対象に入ったメッセージが届くとここに流れます</div>
            </div>
          {/each}
        </div>

        {#if !autoScrollC}
          <button class="scroll-fab" onclick={() => scrollBottom('c')}>最新へ</button>
        {/if}
      </section>
    {/if}
  </div>
  {#if blockPopup}
    <div class="block-popup" style:left="{blockPopup.x}px" style:top="{blockPopup.y}px"
      onclick={(e) => e.stopPropagation()}>
      <span class="block-popup-name">{blockPopup.name}</span>
      <button class="block-btn" onclick={() => blockUser(blockPopup!.name)}>ブロック</button>
    </div>
  {/if}


  {#if panelOpen}
    <div class="backdrop" onclick={closePanel}></div>
  {/if}

  <aside class="panel" class:open={panelOpen}>
    <div class="panel-head">
      <span>
        {panelKind === 'general'
          ? '設定'
          : panelKind === 'tts'
            ? '読み上げ設定'
            : panelKind === 'speaker'
              ? '話者別音声'
              : '読み上げ辞書'}
      </span>
      <button class="close-btn" onclick={closePanel}>×</button>
    </div>
    <div class="panel-body">

      <!-- Theme -->
      {#if panelKind === 'general'}
      <section class="sect">
        <h4 class="sect-title">テーマ</h4>
        <!-- 4x2 grid: 7 presets + 1 custom -->
        <div class="theme-grid">
          {#each THEMES as th}
            <button class="theme-btn" class:sel={s.theme === th.id}
              onclick={() => selectTheme(th.id)} title={th.name}>
              <div class="th-preview">
                <div class="th-bg" style:background={th.colors[0]}>
                  <div class="th-bar" style:background={th.colors[1]}>
                    <div class="th-dot" style:background={th.colors[2]}></div>
                  </div>
                </div>
              </div>
              <span class="th-name">{th.name}</span>
            </button>
          {/each}
          <!-- 8th: Custom -->
          <button class="theme-btn custom-btn" class:sel={s.theme === 'custom'}
            onclick={() => selectTheme('custom')} title="カスタム">
            <div class="th-preview custom-preview">
              <div class="custom-icon">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none"
                  stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <circle cx="13.5" cy="6.5" r="2.5"/>
                  <circle cx="6.5" cy="14.5" r="2.5"/>
                  <circle cx="17.5" cy="17.5" r="2.5"/>
                  <path d="M13.5 9v5.5M6.5 12V6M17.5 15V9"/>
                </svg>
              </div>
            </div>
            <span class="th-name">カスタム</span>
          </button>
        </div>

        <!-- Custom color pickers (shown when custom selected) -->
        {#if s.theme === 'custom'}
          <div class="custom-pickers">
            <div class="cp-grid">
              <label class="cp-row">
                <span>背景色</span>
                <input type="color" bind:value={s.customTheme.bg}
                  oninput={() => save()} class="cp-input" />
              </label>
              <label class="cp-row">
                <span>パネル色</span>
                <input type="color" bind:value={s.customTheme.surface}
                  oninput={() => save()} class="cp-input" />
              </label>
              <label class="cp-row">
                <span>テキスト</span>
                <input type="color" bind:value={s.customTheme.text}
                  oninput={() => save()} class="cp-input" />
              </label>
              <label class="cp-row">
                <span>サブテキスト</span>
                <input type="color" bind:value={s.customTheme.textDim}
                  oninput={() => save()} class="cp-input" />
              </label>
              <label class="cp-row">
                <span>ボーダー</span>
                <input type="color" bind:value={s.customTheme.border}
                  oninput={() => save()} class="cp-input" />
              </label>
              <label class="cp-row">
                <span>アクセント</span>
                <input type="color" bind:value={s.customTheme.accent}
                  oninput={() => save()} class="cp-input" />
              </label>
            </div>
            <div class="cp-divider">チャンネルカラー</div>
            <div class="cp-grid">
              <label class="cp-row">
                <span style:color="var(--c-world)">ワールド</span>
                <input type="color" bind:value={s.customTheme.cWorld}
                  oninput={() => save()} class="cp-input" />
              </label>
              <label class="cp-row">
                <span style:color="var(--c-guild)">ギルド</span>
                <input type="color" bind:value={s.customTheme.cGuild}
                  oninput={() => save()} class="cp-input" />
              </label>
              <label class="cp-row">
                <span style:color="var(--c-party)">パーティ</span>
                <input type="color" bind:value={s.customTheme.cParty}
                  oninput={() => save()} class="cp-input" />
              </label>
              <label class="cp-row">
                <span style:color="var(--c-channel)">チャンネル</span>
                <input type="color" bind:value={s.customTheme.cChannel}
                  oninput={() => save()} class="cp-input" />
              </label>
            </div>
            <button class="reset-btn" onclick={() => {
              s.customTheme = { ...DEFAULT_CUSTOM }; save();
            }}>リセット</button>
          </div>
        {/if}
      </section>
      {/if}

      <!-- Font size -->
      {#if panelKind === 'general'}
      <section class="sect">
        <h4 class="sect-title">フォントサイズ <em>{s.fontSize}px</em></h4>
        <input type="range" min="10" max="22" step="1"
          class="slider" bind:value={s.fontSize} oninput={save} />
        <div class="range-labels"><span>小</span><span>大</span></div>
      </section>
      {/if}

      <!-- Opacity -->
      {#if panelKind === 'general'}
      <section class="sect">
        <h4 class="sect-title">背景の透明度 <em>{Math.round(s.bgOpacity * 100)}%</em></h4>
        <input type="range" min="0.05" max="1.0" step="0.05"
          class="slider" bind:value={s.bgOpacity} oninput={save} />
        <div class="range-labels"><span>透明</span><span>不透明</span></div>
      </section>
      {/if}

      <!-- Always on top -->
      {#if panelKind === 'general'}
      <section class="sect">
        <label class="toggle-row">
          <span class="toggle-label">常に最前面に表示</span>
          <button class="tog" class:on={s.alwaysOnTop}
            onclick={() => setAlwaysOnTop(!s.alwaysOnTop)}>
            <span class="tog-knob"></span>
          </button>
        </label>
      </section>
      {/if}

      <!-- Keyword notify -->
      {#if panelKind === 'general'}
      <section class="sect">
        <label class="toggle-row">
          <span class="toggle-label">キーワード通知音</span>
          <button class="tog" class:on={s.notifyEnabled}
            onclick={() => { s.notifyEnabled = !s.notifyEnabled; save(); }}>
            <span class="tog-knob"></span>
          </button>
        </label>
        {#if s.notifyEnabled}
          <div class="tts-sub">
            <div class="subsect">
              <span class="sublabel">通知キーワードを入力してください。改行かカンマで区切れます。</span>
              <textarea class="kw-area" placeholder={"ボス\nアリーナ\n通知ワード"}
                bind:value={s.notifyKeywords} oninput={save} rows="4"></textarea>
            </div>
            <div class="subsect">
              <span class="sublabel">通知音量 <em>{Math.round(s.notifyVolume * 100)}%</em></span>
              <input type="range" min="0.0" max="1.0" step="0.05"
                class="slider" bind:value={s.notifyVolume} oninput={save} />
            </div>
            <div class="subsect">
              <span class="sublabel">通知音ファイル</span>
              <div class="sound-row">
                {#if s.notifyHasCustomSound}
                  <span class="sound-name">{soundFileName}</span>
                  <button class="mini-btn danger" onclick={clearCustomSound}>削除</button>
                {:else}
                  <span class="sound-name dim">デフォルト音を使用します</span>
                {/if}
                <label class="mini-btn file-label">ファイルを選択
                  <input type="file" accept="audio/*" class="hidden-file" onchange={handleSoundFile} />
                </label>
              </div>
            </div>
            <div class="btn-row">
              <button class="test-btn" onclick={playNotification}>テスト再生</button>
            </div>
          </div>
        {/if}
      </section>
      {/if}

      <!-- TTS -->
      {#if panelKind === 'tts'}
      <section class="sect">
        <label class="toggle-row">
          <span class="toggle-label">チャット読み上げ</span>
          <button class="tog" class:on={s.ttsEnabled}
            onclick={() => {
              s.ttsEnabled = !s.ttsEnabled;
              save();
              if (s.ttsEnabled) {
                if (s.ttsProvider === 'aivis') {
                  void refreshAivisVoices();
                } else {
                  void refreshSystemVoices();
                }
              } else {
                clearTtsQueue();
              }
            }}>
            <span class="tog-knob"></span>
          </button>
        </label>
        {#if s.ttsEnabled}
          <div class="tts-sub">
            <div class="subsect">
              <span class="sublabel">読み上げ方式</span>
              <div class="ch-pills">
                <button class="pill" class:on={s.ttsProvider === 'aivis'}
                  style:--pc={'var(--accent)'}
                  onclick={() => setTtsProvider('aivis')}>AivisSpeech</button>
                <button class="pill" class:on={s.ttsProvider === 'system'}
                  style:--pc={'var(--accent)'}
                  onclick={() => setTtsProvider('system')}>Windows 標準</button>
              </div>
            </div>
            {#if s.ttsProvider === 'aivis'}
              <div class="subsect">
                <span class="sublabel">AivisSpeech エンジン URL</span>
                <div class="inline-row">
                  <input
                    class="text-input"
                    type="text"
                    bind:value={s.ttsEngineUrl}
                    placeholder={DEFAULT_AIVIS_URL}
                    onblur={save}
                  />
                  <button class="mini-btn" onclick={() => void refreshAivisVoices(true)}>
                    {ttsLoadingVoices ? '更新中...' : '再読み込み'}
                  </button>
                </div>
                <p class="helper-text">
                  AivisSpeech を起動しておけば、そのまま `http://127.0.0.1:10101` で使えます。
                </p>
                <p class="helper-text">
                  AivisSpeech に接続できないときは、読み上げ時に Windows 標準音声へ自動で切り替えます。
                </p>
              </div>
            {:else}
              <div class="subsect">
                <span class="sublabel">Windows 標準音声</span>
                <div class="inline-row">
                  <p class="helper-text helper-inline">
                    Windows に入っている音声をそのまま使います。追加インストールは不要です。
                  </p>
                  <button class="mini-btn" onclick={() => void refreshSystemVoices(true)}>
                    {ttsLoadingVoices ? '更新中...' : '再読み込み'}
                  </button>
                </div>
              </div>
            {/if}
            <div class="subsect">
              {#if ttsStatusMessage}
                <p class="helper-text" class:helper-error={ttsStatusError}>{ttsStatusMessage}</p>
              {/if}
              {#if s.ttsPlaybackMode !== 'interrupt' && getTtsPendingCount() > 0}
                <p class="helper-text">読み上げ待ち: {getTtsPendingCount()} 件</p>
              {/if}
            </div>
            <div class="subsect">
              <span class="sublabel">新着チャットの扱い</span>
              <div class="ch-pills">
                <button class="pill" class:on={s.ttsPlaybackMode === 'interrupt'}
                  style:--pc={'var(--accent)'}
                  onclick={() => setTtsPlaybackMode('interrupt')}>中断して最新</button>
                <button class="pill" class:on={s.ttsPlaybackMode === 'latest_after_current'}
                  style:--pc={'var(--accent)'}
                  onclick={() => setTtsPlaybackMode('latest_after_current')}>今の1件+最新1件</button>
                <button class="pill" class:on={s.ttsPlaybackMode === 'queue'}
                  style:--pc={'var(--accent)'}
                  onclick={() => setTtsPlaybackMode('queue')}>順番に全部読む</button>
              </div>
              {#if s.ttsPlaybackMode === 'interrupt'}
                <p class="helper-text">
                  新着が来たらすぐ切り替えます。常に最新を優先したいとき向けです。
                </p>
              {:else if s.ttsPlaybackMode === 'latest_after_current'}
                <p class="helper-text">
                  今読んでいる1件は最後まで読み、その間に来た新着は最新1件だけ残します。混みやすい時間帯にいちばん扱いやすい設定です。
                </p>
              {:else}
                <p class="helper-text">
                  `順番に全部読む` は見逃しにくい反面、チャットが多いと読み上げ待ちが溜まりやすくなります。
                </p>
              {/if}
            </div>
            <div class="subsect">
              <span class="sublabel">読み上げるチャンネル</span>
              <div class="ch-pills">
                {#each CH_PILLS as ch}
                  <button class="pill" class:on={s.ttsChannels.includes(ch.id)}
                    style:--pc={`var(${ch.css})`}
                    onclick={() => toggleTtsCh(ch.id)}>{ch.name}</button>
                {/each}
              </div>
            </div>
            <div class="subsect">
              <span class="sublabel">話速 <em>{s.ttsRate.toFixed(1)}x</em></span>
              <input type="range" min="0.5" max="2.0" step="0.1"
                class="slider" bind:value={s.ttsRate} oninput={save} />
              <div class="range-labels"><span>遅い</span><span>速い</span></div>
            </div>
            <div class="subsect">
              <span class="sublabel">音量 <em>{Math.round(s.ttsVolume * 100)}%</em></span>
              <input type="range" min="0.0" max="1.0" step="0.05"
                class="slider" bind:value={s.ttsVolume} oninput={save} />
            </div>
            {#if s.ttsProvider === 'aivis' && aivisStyles.length > 0}
              <div class="subsect">
                <span class="sublabel">音声</span>
                <select class="sel-voice" bind:value={s.ttsVoice} onchange={save}>
                  {#each aivisStyles as style}
                    <option value={String(style.id)}>{style.displayName}</option>
                  {/each}
                </select>
              </div>
            {/if}
            {#if s.ttsProvider === 'system' && systemVoices.length > 0}
              <div class="subsect">
                <span class="sublabel">音声</span>
                <select class="sel-voice" bind:value={s.ttsSystemVoice} onchange={save}>
                  {#each systemVoices as voice}
                    <option value={voice.name}>{voice.name}</option>
                  {/each}
                </select>
              </div>
            {/if}
            <div class="btn-row">
              <button class="test-btn" onclick={testTts}>テスト再生</button>
            </div>
          </div>
        {/if}
      </section>
      {/if}

      {#if panelKind === 'speaker'}
      <section class="sect">
        <h4 class="sect-title">話者別音声 <em>{getActiveTtsSpeakerVoiceCount()}人</em></h4>
        <p class="helper-text">
          ギルメンなど一部の人だけ、AivisSpeech の別モデルで読み上げできます。
        </p>
        <p class="helper-text">
          モデルを増やしすぎると、AivisSpeech 側のメモリ使用量が増えることがあります。
        </p>
        <div class="tts-sub">
          {#if s.ttsProvider !== 'aivis'}
            <div class="subsect">
              <p class="helper-text">
                現在の読み上げ方式は `Windows 標準` です。話者別音声は `AivisSpeech` 利用時に有効になります。
              </p>
            </div>
          {/if}
          <div class="subsect">
            <div class="inline-row">
              <span class="sublabel">AivisSpeech のモデル一覧</span>
              <button class="mini-btn" onclick={() => void refreshAivisVoices(true)}>
                {ttsLoadingVoices ? '更新中...' : '再読み込み'}
              </button>
            </div>
            {#if ttsStatusMessage && (!aivisStyles.length || ttsStatusError)}
              <p class="helper-text" class:helper-error={ttsStatusError}>{ttsStatusMessage}</p>
            {/if}
          </div>
          {#if aivisStyles.length > 0}
            <div class="subsect">
              <span class="sublabel">新しく追加</span>
              <div class="dict-row dict-add-row">
                <input
                  class="text-input"
                  type="text"
                  bind:value={ttsSpeakerDraftName}
                    placeholder="defoco"
                />
                <span class="dict-arrow">→</span>
                <select class="sel-voice" bind:value={ttsSpeakerDraftStyleId}>
                  {#each aivisStyles as style}
                    <option value={String(style.id)}>{style.displayName}</option>
                  {/each}
                </select>
                <button
                  class="mini-btn"
                  onclick={addTtsSpeakerVoiceEntry}
                  disabled={!ttsSpeakerDraftName.trim() || !ttsSpeakerDraftStyleId}
                >追加</button>
              </div>
            </div>
          {:else}
            <div class="subsect">
              <p class="bl-empty">AivisSpeech に接続できると、ここでモデルを選べます。</p>
            </div>
          {/if}
          <div class="subsect">
            <span class="sublabel">登録済み</span>
            {#if s.ttsSpeakerVoices.length === 0}
              <p class="bl-empty">まだ登録がありません。必要な人だけ個別音声にできます。</p>
            {:else}
              <div class="dict-list">
                {#each s.ttsSpeakerVoices as entry (entry.id)}
                  <div class="dict-row">
                    <input
                      class="text-input"
                      type="text"
                      value={entry.speakerName}
                      placeholder="名前"
                      oninput={(e) => updateTtsSpeakerVoiceName(entry.id, (e.currentTarget as HTMLInputElement).value)}
                    />
                    <span class="dict-arrow">→</span>
                    <select
                      class="sel-voice"
                      value={entry.styleId}
                      onchange={(e) => updateTtsSpeakerVoiceStyle(entry.id, (e.currentTarget as HTMLSelectElement).value)}
                    >
                      {#if entry.styleId && !hasAivisStyle(entry.styleId)}
                        <option value={entry.styleId}>{getAivisStyleLabel(entry.styleId, entry.styleLabel) || '未取得のモデル'}</option>
                      {/if}
                      {#each aivisStyles as style}
                        <option value={String(style.id)}>{style.displayName}</option>
                      {/each}
                    </select>
                    <button class="mini-btn danger" onclick={() => removeTtsSpeakerVoiceEntry(entry.id)}>削除</button>
                  </div>
                {/each}
              </div>
            {/if}
            <p class="helper-text">
              一致した名前の発言だけ個別音声を使います。見つからないモデルは共通音声に戻ります。
            </p>
          </div>
        </div>
      </section>
      {/if}

      {#if panelKind === 'dictionary'}
      <section class="sect">
        <h4 class="sect-title">読み上げ辞書 <em>{getActiveTtsDictionaryCount()}件</em></h4>
        <p class="helper-text">
          ここで登録した語句は、AivisSpeech や Windows 標準音声へ渡す前に置換します。
        </p>
        <div class="tts-sub">
          <div class="subsect">
            <span class="sublabel">新しく追加</span>
            <div class="dict-row dict-add-row">
              <input
                class="text-input"
                type="text"
                bind:value={ttsDictionaryDraftFrom}
                placeholder="DPS"
              />
              <span class="dict-arrow">→</span>
              <input
                class="text-input"
                type="text"
                bind:value={ttsDictionaryDraftTo}
                placeholder="ディーピーエス"
              />
              <button
                class="mini-btn"
                onclick={addTtsDictionaryEntry}
                disabled={!ttsDictionaryDraftFrom.trim() || !ttsDictionaryDraftTo.trim()}
              >追加</button>
            </div>
          </div>

          <div class="subsect">
            <span class="sublabel">登録済み</span>
            {#if s.ttsDictionary.length === 0}
              <p class="bl-empty">まだ登録がありません。読みが崩れやすい単語をここで直せます。</p>
            {:else}
              <div class="dict-list">
                {#each s.ttsDictionary as entry (entry.id)}
                  <div class="dict-row">
                    <input
                      class="text-input"
                      type="text"
                      value={entry.from}
                      placeholder="語句"
                      oninput={(e) => updateTtsDictionaryEntry(entry.id, 'from', (e.currentTarget as HTMLInputElement).value)}
                    />
                    <span class="dict-arrow">→</span>
                    <input
                      class="text-input"
                      type="text"
                      value={entry.to}
                      placeholder="読み"
                      oninput={(e) => updateTtsDictionaryEntry(entry.id, 'to', (e.currentTarget as HTMLInputElement).value)}
                    />
                    <button class="mini-btn danger" onclick={() => removeTtsDictionaryEntry(entry.id)}>削除</button>
                  </div>
                {/each}
              </div>
            {/if}
            <p class="helper-text">
              長い単語を優先して置換します。空欄の行は読み上げ時に無視されます。
            </p>
          </div>
        </div>
      </section>
      {/if}

      <!-- Blacklist -->
      {#if panelKind === 'general'}
      <section class="sect">
        <h4 class="sect-title">ブラックリスト <em>{s.blacklist.length}</em></h4>
        {#if s.blacklist.length === 0}
          <p class="bl-empty">非表示ユーザーはいません。名前をクリックすると追加できます。</p>
        {:else}
          <div class="bl-tags">
            {#each s.blacklist as name}
              <span class="bl-tag">
                {name}
                <button class="bl-remove" onclick={() => unblockUser(name)} title="解除">×</button>
              </span>
            {/each}
          </div>
        {/if}
      </section>
      {/if}

    </div>
  </aside>
</div>

<!-- STYLES -->
<style>
  .app {
    display: flex; flex-direction: column; height: 100vh;
    background: rgb(var(--bg) / var(--bg-opacity, 0.92));
    position: relative; overflow: hidden;
  }

  /* Header */
  .header {
    display: flex; align-items: center;
    background: rgb(var(--surface) / 0.97);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0; padding-right: 4px;
  }
  .tabs { display: flex; flex: 1; overflow: hidden; }
  .tab {
    padding: 9px 11px; border: none; background: transparent;
    color: var(--text-dim); cursor: pointer; font: inherit; font-size: 0.9em;
    white-space: nowrap; border-bottom: 2px solid transparent;
    transition: color 0.15s, border-color 0.15s, background 0.15s;
  }
  .tab:hover  { color: var(--text); background: rgb(var(--bg) / 0.5); }
  .tab.active { color: var(--tc, var(--accent)); border-bottom-color: var(--tc, var(--accent)); }

  .panel-icon {
    width: 34px; height: 34px; display: flex; align-items: center; justify-content: center;
    border: none; background: transparent; color: var(--text-dim);
    cursor: pointer; border-radius: 6px; flex-shrink: 0;
    transition: color 0.15s, background 0.15s, transform 0.2s;
  }
  .panel-icon:hover  { color: var(--accent); background: rgb(var(--bg) / 0.5); }
  .panel-icon.active { color: var(--accent); background: rgb(var(--bg) / 0.55); }
  .gear.active { transform: rotate(60deg); }
  .voice-icon.active { transform: translateY(-1px); }
  .speaker-icon.active { transform: translateY(-1px); }
  .dictionary-icon.active { transform: translateY(-1px); }

  /* Messages */
  .messages {
    flex: 1; overflow-y: auto; padding: 5px 8px;
    display: flex; flex-direction: column; gap: 2px;
  }

  .status-banner {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
    background: rgba(120, 22, 22, 0.92);
    color: #fff4f4;
  }
  .status-copy {
    display: flex;
    flex-direction: column;
    gap: 4px;
    line-height: 1.45;
    font-size: 0.88em;
  }
  .status-copy strong {
    font-size: 0.95em;
  }
  .status-close {
    flex-shrink: 0;
    border: 1px solid rgba(255, 255, 255, 0.28);
    background: rgba(255, 255, 255, 0.08);
    color: inherit;
    border-radius: 6px;
    cursor: pointer;
    font: inherit;
    padding: 5px 10px;
  }
  .status-close:hover {
    background: rgba(255, 255, 255, 0.16);
  }
  .messages::-webkit-scrollbar { width: 3px; }
  .messages::-webkit-scrollbar-thumb { background: var(--border); border-radius: 2px; }

  .msg {
    display: flex; gap: 5px; align-items: baseline; flex-wrap: wrap;
    line-height: 1.55; padding: 1px 3px; border-radius: 3px;
    transition: background 0.1s;
  }
  .msg:hover { background: rgb(var(--surface) / 0.5); }
  .time  { color: var(--text-dim); font-size: 0.82em; flex-shrink: 0; font-variant-numeric: tabular-nums; }
  .badge { font-size: 0.82em; font-weight: 700; flex-shrink: 0; }
  .name  { color: var(--text); font-weight: 600; flex-shrink: 0; }
  .sep   { color: var(--text-dim); flex-shrink: 0; }
  .body  { color: var(--text); word-break: break-word; }
  .body.stamp { font-style: italic; font-size: 0.88em; }
  .body :global(.kw-hl) {
    background: var(--accent); color: #000;
    border-radius: 2px; padding: 0 2px;
    font-weight: 700;
  }

  .empty { display: flex; flex-direction: column; align-items: center; gap: 8px; margin-top: 60px; color: var(--text); text-align: center; }
  .empty-icon { font-size: 2em; }
  .empty-sub  { font-size: 0.85em; }

  .scroll-fab {
    position: absolute; bottom: 14px; right: 14px;
    background: var(--accent); color: #000; border: none; border-radius: 20px;
    padding: 5px 14px; cursor: pointer; font: inherit; font-size: 0.82em; font-weight: 700;
    box-shadow: 0 2px 12px var(--accent-glow);
    transition: transform 0.15s, box-shadow 0.15s;
  }
  .scroll-fab:hover { transform: translateY(-1px); box-shadow: 0 4px 16px var(--accent-glow); }

  /* Panel */
  .backdrop { position: absolute; inset: 0; background: rgba(0 0 0 / 0.22); backdrop-filter: blur(2px); z-index: 9; }
  .panel {
    position: absolute; top: 0; right: 0; bottom: 0; width: min(280px, 92vw);
    background: rgb(var(--surface) / 0.97); backdrop-filter: blur(20px);
    border-left: 1px solid var(--border); display: flex; flex-direction: column;
    z-index: 10; transform: translateX(100%);
    transition: transform 0.25s cubic-bezier(0.4, 0, 0.2, 1);
    box-shadow: -6px 0 32px rgba(0 0 0 / 0.45);
  }
  .panel.open { transform: translateX(0); }
  .panel-head {
    display: flex; align-items: center; justify-content: space-between;
    padding: 11px 14px 9px; border-bottom: 1px solid var(--border);
    font-weight: 700; font-size: 0.9em; color: var(--accent); flex-shrink: 0;
  }
  .close-btn {
    background: none; border: none; cursor: pointer; color: var(--text-dim);
    font-size: 1em; padding: 3px 6px; border-radius: 4px;
    transition: color 0.15s, background 0.15s;
  }
  .close-btn:hover { color: var(--text); background: var(--input-bg); }
  .panel-body { flex: 1; overflow-y: auto; padding: 4px 0 16px; }
  .panel-body::-webkit-scrollbar { width: 3px; }
  .panel-body::-webkit-scrollbar-thumb { background: var(--border); border-radius: 2px; }

  /* Sections */
  .sect { padding: 11px 14px; border-bottom: 1px solid var(--border); }
  .sect:last-child { border-bottom: none; }
  .sect-title { font-size: 0.78em; font-weight: 700; text-transform: uppercase; letter-spacing: 0.08em; color: var(--text-dim); margin-bottom: 9px; }
  .sect-title em { font-style: normal; color: var(--accent); margin-left: 6px; }

  /* Theme grid */
  .theme-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr)); /* minmax(0,1fr) prevents content from bloating cells */
    gap: 5px;
  }
  .theme-btn {
    /* explicit 100% width so all cells render identically */
    width: 100%;
    background: none; border: 2px solid transparent; cursor: pointer;
    border-radius: 8px; padding: 4px;
    display: flex; flex-direction: column; align-items: center; gap: 3px;
    transition: border-color 0.15s, transform 0.15s;
    /* prevent text from pushing button taller */
    overflow: hidden;
  }
  .theme-btn:hover { border-color: var(--border); transform: scale(1.04); }
  .theme-btn.sel   { border-color: var(--accent); }

  .th-preview { width: 100%; aspect-ratio: 1.4; border-radius: 4px; overflow: hidden; flex-shrink: 0; }
  .th-bg  { width: 100%; height: 100%; display: flex; align-items: flex-end; }
  .th-bar { width: 100%; padding: 3px 4px; display: flex; align-items: center; gap: 3px; }
  .th-dot { width: 5px; height: 5px; border-radius: 50%; flex-shrink: 0; }
  .th-name {
    font-size: 0.68em; color: var(--text-dim);
    /* fixed single-line to prevent text length from altering button height */
    width: 100%; text-align: center;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    line-height: 1.2;
  }

  /* Custom theme button */
  .custom-btn { }
  .custom-preview {
    background: linear-gradient(135deg, #1a0533 0%, #003322 50%, #001a2e 100%) !important;
    display: flex; align-items: center; justify-content: center;
  }
  .custom-preview { width: 100%; aspect-ratio: 1.4; border-radius: 4px; overflow: hidden; flex-shrink: 0; }
  .custom-icon { display: flex; align-items: center; justify-content: center; width: 100%; height: 100%; color: #888; }
  .theme-btn.sel.custom-btn .custom-icon { color: var(--accent); }

  /* Custom color pickers */
  .custom-pickers {
    margin-top: 10px; padding: 10px; border-radius: 8px;
    background: var(--input-bg); border: 1px solid var(--border);
    display: flex; flex-direction: column; gap: 8px;
  }
  .cp-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 5px; }
  .cp-row {
    display: flex; align-items: center; justify-content: space-between;
    font-size: 0.8em; color: var(--text-dim); cursor: pointer; gap: 6px;
    padding: 4px 6px; border-radius: 5px; background: rgb(var(--surface) / 0.5);
  }
  .cp-row:hover { background: rgb(var(--surface) / 0.8); }
  .cp-row span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cp-input {
    width: 28px; height: 22px; border: none; border-radius: 4px; cursor: pointer;
    padding: 1px; flex-shrink: 0;
  }
  .cp-divider {
    font-size: 0.74em; color: var(--text-dim); text-align: center;
    border-top: 1px solid var(--border); padding-top: 6px;
    letter-spacing: 0.05em;
  }
  .reset-btn {
    align-self: center; background: none; border: 1px solid var(--border);
    color: var(--text-dim); cursor: pointer; font: inherit; font-size: 0.78em;
    padding: 4px 12px; border-radius: 5px; transition: border-color 0.15s, color 0.15s;
  }
  .reset-btn:hover { border-color: var(--accent); color: var(--accent); }

  /* Slider */
  .slider {
    -webkit-appearance: none; appearance: none;
    width: 100%; height: 4px; background: var(--border);
    border-radius: 2px; outline: none; cursor: pointer; margin: 4px 0;
  }
  .slider::-webkit-slider-thumb {
    -webkit-appearance: none; appearance: none;
    width: 15px; height: 15px; border-radius: 50%; background: var(--thumb);
    cursor: pointer; box-shadow: 0 0 6px var(--accent-glow);
    transition: transform 0.1s, box-shadow 0.1s;
  }
  .slider::-webkit-slider-thumb:hover { transform: scale(1.25); box-shadow: 0 0 10px var(--accent-glow); }
  .range-labels { display: flex; justify-content: space-between; font-size: 0.74em; color: var(--text-dim); margin-top: 1px; }

  /* Toggle */
  .toggle-row { display: flex; align-items: center; justify-content: space-between; cursor: pointer; gap: 8px; }
  .toggle-label { font-size: 0.9em; color: var(--text); flex: 1; }
  .tog {
    width: 44px; height: 24px; border-radius: 12px; background: var(--border);
    border: none; cursor: pointer; position: relative; flex-shrink: 0; transition: background 0.2s;
  }
  .tog.on { background: var(--accent); }
  .tog-knob {
    position: absolute; width: 18px; height: 18px; border-radius: 50%; background: #fff;
    top: 3px; left: 3px;
    transition: transform 0.2s cubic-bezier(0.4, 0, 0.2, 1); pointer-events: none;
  }
  .tog.on .tog-knob { transform: translateX(20px); }

  /* Sub-sections */
  .tts-sub { margin-top: 10px; display: flex; flex-direction: column; gap: 11px; }
  .subsect { display: flex; flex-direction: column; gap: 5px; }
  .sublabel { font-size: 0.79em; color: var(--text-dim); }
  .sublabel em { font-style: normal; color: var(--accent); margin-left: 4px; }

  /* Channel pills */
  .ch-pills { display: flex; flex-wrap: wrap; gap: 5px; flex: 1; min-width: 0; }
  .pill {
    padding: 3px 10px; border-radius: 20px; border: 1.5px solid var(--border);
    background: none; cursor: pointer; font: inherit; font-size: 0.8em; color: var(--text-dim);
    transition: border-color 0.15s, color 0.15s;
  }
  .pill.on { border-color: var(--pc); color: var(--pc); font-weight: 700; }

  /* Keyword textarea */
  .kw-area {
    width: 100%; padding: 7px 8px; resize: vertical;
    background: var(--input-bg); border: 1px solid var(--border);
    color: var(--text); border-radius: 6px; font: inherit; font-size: 0.85em;
    outline: none; line-height: 1.5; min-height: 72px;
  }
  .kw-area:focus { border-color: var(--accent); }
  .kw-area::placeholder { color: var(--text-dim); }

  /* Sound file */
  .sound-row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .sound-name { font-size: 0.8em; color: var(--text); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sound-name.dim { color: var(--text-dim); }
  .mini-btn {
    padding: 4px 10px; border-radius: 5px; cursor: pointer; font: inherit; font-size: 0.78em;
    background: var(--input-bg); border: 1px solid var(--border); color: var(--text);
    transition: background 0.15s, border-color 0.15s; flex-shrink: 0;
  }
  .mini-btn:hover { border-color: var(--accent); }
  .mini-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
    border-color: var(--border);
  }
  .mini-btn:disabled:hover {
    border-color: var(--border);
    background: var(--input-bg);
  }
  .mini-btn.danger { color: #f87171; border-color: rgba(248,113,113,0.4); }
  .mini-btn.danger:hover { background: rgba(248,113,113,0.12); }
  .file-label { cursor: pointer; }
  .hidden-file { display: none; }

  /* Voice / buttons */
  .inline-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .text-input,
  .sel-voice {
    width: 100%; padding: 6px 8px;
    background: rgb(var(--surface) / 0.96); border: 1px solid var(--border);
    color: var(--text); border-radius: 6px; font: inherit; font-size: 0.85em; outline: none; cursor: pointer;
  }
  .sel-voice option {
    background: rgb(var(--surface));
    color: var(--text);
  }
  .text-input {
    cursor: text;
  }
  .text-input:focus,
  .sel-voice:focus { border-color: var(--accent); }
  .helper-text {
    margin: 2px 0 0;
    font-size: 0.76em;
    color: var(--text-dim);
    line-height: 1.45;
  }
  .helper-error {
    color: #fda4af;
  }
  .helper-inline {
    flex: 1;
    margin-top: 0;
  }
  .dict-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .dict-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr) auto;
    gap: 6px;
    align-items: center;
  }
  .dict-arrow {
    color: var(--text-dim);
    font-size: 0.8em;
    line-height: 1;
  }
  .btn-row { display: flex; gap: 8px; }
  .test-btn {
    background: var(--input-bg); border: 1px solid var(--border); color: var(--accent);
    cursor: pointer; font: inherit; font-size: 0.82em; padding: 5px 12px; border-radius: 6px;
    transition: background 0.15s, border-color 0.15s;
  }
  .test-btn:hover { background: rgb(var(--surface) / 0.8); border-color: var(--accent); }

  /* Clickable name */
  .clickable-name {
    cursor: pointer; border-radius: 3px;
    transition: color 0.12s, background 0.12s;
  }
  .clickable-name:hover { color: var(--accent); background: rgb(var(--surface) / 0.6); }

  /* Block popup */
  .block-popup {
    position: fixed; z-index: 100;
    background: rgb(var(--surface) / 0.97); border: 1px solid var(--border);
    border-radius: 8px; padding: 7px 10px;
    box-shadow: 0 4px 20px rgba(0 0 0 / 0.5);
    display: flex; align-items: center; gap: 8px;
    backdrop-filter: blur(12px);
  }
  .block-popup-name {
    font-size: 0.82em; color: var(--text-dim);
    max-width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .block-btn {
    background: rgba(248,113,113,0.12); border: 1px solid rgba(248,113,113,0.4);
    color: #f87171; cursor: pointer; font: inherit; font-size: 0.8em;
    padding: 4px 10px; border-radius: 5px; white-space: nowrap;
    transition: background 0.15s;
  }
  .block-btn:hover { background: rgba(248,113,113,0.22); }

  /* Blacklist tags */
  .bl-empty { font-size: 0.8em; color: var(--text-dim); margin: 0; }
  .bl-tags { display: flex; flex-wrap: wrap; gap: 5px; }
  .bl-tag {
    display: inline-flex; align-items: center; gap: 4px;
    background: rgba(248,113,113,0.1); border: 1px solid rgba(248,113,113,0.3);
    color: #f87171; border-radius: 20px;
    padding: 3px 8px 3px 10px; font-size: 0.8em;
  }
  .bl-remove {
    background: none; border: none; cursor: pointer;
    color: rgba(248,113,113,0.6); font-size: 1em; line-height: 1;
    padding: 0 1px; border-radius: 50%;
    transition: color 0.12s;
  }
  .bl-remove:hover { color: #f87171; }
  .header-title {
    padding: 0 10px;
    color: var(--accent);
    font-size: 0.84em;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .header-actions {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 4px 4px 0;
  }

  .split-toggle {
    flex-shrink: 0;
    min-width: 48px;
  }

  .chat-layout {
    flex: 1;
    min-height: 0;
    display: grid;
    overflow: hidden;
  }

  .chat-layout.split {
    gap: 0;
  }

  .message-pane {
    position: relative;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .splitter {
    position: relative;
    cursor: row-resize;
    background: rgb(var(--surface) / 0.92);
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }

  .splitter::after {
    content: '';
    position: absolute;
    left: 50%;
    top: 50%;
    width: 54px;
    height: 2px;
    transform: translate(-50%, -50%);
    border-radius: 999px;
    background: rgb(var(--bg) / 0.45);
  }

  .splitter:hover {
    background: rgb(var(--surface) / 1);
  }

  .filter-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    background: rgb(var(--surface) / 0.88);
  }

  :global(body.split-resizing) {
    user-select: none;
    cursor: row-resize;
  }

  @media (max-width: 560px) {
    .dict-row {
      grid-template-columns: 1fr;
    }
    .dict-arrow {
      display: none;
    }
  }
</style>


