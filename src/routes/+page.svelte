<script lang="ts">
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
  import { onMount } from 'svelte';

  interface ChatMessage {
    channel: number; channel_name: string; sender_id: number;
    sender_name: string; text: string; timestamp: number;
  }

  const TABS = [
    { id: 0, name: 'すべて' }, { id: 1, name: 'ワールド' },
    { id: 2, name: 'チャンネル' }, { id: 3, name: 'パーティ' }, { id: 4, name: 'ギルド' },
  ];

  // TTS pills: ワールド → ギルド → パーティ → チャンネル
  const CH_PILLS = [
    { id: 1, name: 'ワールド', css: '--c-world'   },
    { id: 4, name: 'ギルド',   css: '--c-guild'   },
    { id: 3, name: 'パーティ', css: '--c-party'   },
    { id: 2, name: 'チャンネル',css: '--c-channel' },
  ] as const;

  const THEMES = [
    { id: 'dark',     name: 'ダーク',   colors: ['#0d0d17', '#16163a', '#a78bfa'] },
    { id: 'light',    name: 'ライト',   colors: ['#f8fafc', '#e2e8f0', '#7c3aed'] },
    { id: 'oled',     name: 'OLED',    colors: ['#000000', '#080808', '#d8b4fe'] },
    { id: 'cyber',    name: 'サイバー', colors: ['#00050f', '#000c1c', '#00ff88'] },
    { id: 'sakura',   name: 'サクラ',   colors: ['#12050c', '#1e0814', '#f472b6'] },
    { id: 'midnight', name: 'ミッドナイト', colors: ['#050819', '#0a0f2d', '#c4b5fd'] },
    { id: 'nord',     name: 'ノルド',   colors: ['#242938', '#2e3448', '#b48ead'] },
  ] as const;

  // ─── Custom theme ──────────────────────────────────────
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

  // ─── Settings ──────────────────────────────────────────
  interface Settings {
    theme: string; fontSize: number; bgOpacity: number; alwaysOnTop: boolean;
    ttsEnabled: boolean; ttsRate: number; ttsVolume: number;
    ttsChannels: number[]; ttsVoice: string;
    notifyEnabled: boolean; notifyKeywords: string;
    notifyVolume: number; notifyHasCustomSound: boolean;
    customTheme: CustomTheme;
    blacklist: string[];
  }
  const DEFAULTS: Settings = {
    theme: 'dark', fontSize: 13, bgOpacity: 0.92, alwaysOnTop: true,
    ttsEnabled: false, ttsRate: 1.0, ttsVolume: 0.8,
    ttsChannels: [1, 2, 3, 4], ttsVoice: '',
    notifyEnabled: false, notifyKeywords: '', notifyVolume: 0.8, notifyHasCustomSound: false,
    customTheme: { ...DEFAULT_CUSTOM },
    blacklist: [],
  };

  // ─── State ─────────────────────────────────────────────
  let s          = $state<Settings>({ ...DEFAULTS, customTheme: { ...DEFAULT_CUSTOM } });
  let messages   = $state<ChatMessage[]>([]);
  let activeTab  = $state(0);
  let panelOpen  = $state(false);
  let listEl     = $state<HTMLElement | null>(null);
  let autoScroll = $state(true);
  let voices     = $state<SpeechSynthesisVoice[]>([]);
  let soundFileName = $state('');
  let blockPopup = $state<{ name: string; x: number; y: number } | null>(null);

  let notifyAudio: HTMLAudioElement | null = null;
  let audioCtx: AudioContext | null = null;

  const filtered = $derived(
    (activeTab === 0 ? messages : messages.filter(m => m.channel === activeTab))
      .filter(m => !s.blacklist.includes(m.sender_name))
  );

  // ─── Theme helpers ─────────────────────────────────────
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

  // ─── Audio ─────────────────────────────────────────────
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
    if (file.size > 3 * 1024 * 1024) { alert('ファイルサイズが大きすぎます（3MB以下推奨）'); return; }
    const reader = new FileReader();
    reader.onload = () => {
      try {
        const data = reader.result as string;
        localStorage.setItem('rchat-notify-sound', data);
        notifyAudio = new Audio(data);
        soundFileName = file.name;
        s.notifyHasCustomSound = true; save();
      } catch { alert('保存に失敗しました。'); }
    };
    reader.readAsDataURL(file);
  }

  function clearCustomSound() {
    localStorage.removeItem('rchat-notify-sound');
    notifyAudio = null; soundFileName = '';
    s.notifyHasCustomSound = false; save();
  }

  // ─── Text helpers ──────────────────────────────────────
  function isStamp(text: string) { return text.startsWith('emojiPic='); }
  function stripSprites(text: string) { return text.replace(/<sprite=\d+>/g, '').trim(); }
  function displayText(text: string) { return isStamp(text) ? null : stripSprites(text); }

  function checkKeywords(msg: ChatMessage) {
    if (!s.notifyEnabled || !s.notifyKeywords.trim()) return;
    if (isStamp(msg.text)) return;
    const kws = s.notifyKeywords.split(/[\n,、]/).map(k => k.trim()).filter(k => k);
    const clean = stripSprites(msg.text);
    if (kws.some(kw => clean.includes(kw) || msg.sender_name.includes(kw))) playNotification();
  }

  // ─── TTS ───────────────────────────────────────────────
  function speak(msg: ChatMessage) {
    if (!s.ttsEnabled || !s.ttsChannels.includes(msg.channel)) return;
    if (!('speechSynthesis' in window)) return;
    if (isStamp(msg.text)) return;
    const clean = stripSprites(msg.text);
    if (!clean) return;
    const utt = new SpeechSynthesisUtterance(`${msg.sender_name}、${clean}`);
    utt.lang = 'ja-JP'; utt.rate = s.ttsRate; utt.volume = s.ttsVolume;
    if (s.ttsVoice) { const v = voices.find(v => v.name === s.ttsVoice); if (v) utt.voice = v; }
    window.speechSynthesis.cancel(); window.speechSynthesis.speak(utt);
  }

  function testTts() {
    if (!('speechSynthesis' in window)) return;
    const utt = new SpeechSynthesisUtterance('テスト、ワールドチャット');
    utt.lang = 'ja-JP'; utt.rate = s.ttsRate; utt.volume = s.ttsVolume;
    if (s.ttsVoice) { const v = voices.find(v => v.name === s.ttsVoice); if (v) utt.voice = v; }
    window.speechSynthesis.cancel(); window.speechSynthesis.speak(utt);
  }

  function toggleTtsCh(id: number) {
    s.ttsChannels = s.ttsChannels.includes(id)
      ? s.ttsChannels.filter(c => c !== id) : [...s.ttsChannels, id];
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

  function onScroll() {
    if (!listEl) return;
    autoScroll = listEl.scrollHeight - listEl.scrollTop - listEl.clientHeight < 80;
  }
  function scrollBottom() { if (listEl) { listEl.scrollTop = listEl.scrollHeight; autoScroll = true; } }

  // ─── Mount ─────────────────────────────────────────────
  onMount(async () => {
    try {
      const saved = localStorage.getItem('rchat');
      if (saved) {
        const parsed = JSON.parse(saved);
        s = { ...DEFAULTS, customTheme: { ...DEFAULT_CUSTOM }, ...parsed,
              customTheme: { ...DEFAULT_CUSTOM, ...(parsed.customTheme ?? {}) } };
      }
    } catch {}
    applyCSS(s);
    try { await getCurrentWebviewWindow().setAlwaysOnTop(s.alwaysOnTop); } catch {}
    if (s.notifyHasCustomSound) {
      const data = localStorage.getItem('rchat-notify-sound');
      if (data) { notifyAudio = new Audio(data); soundFileName = '(保存済み)'; }
      else { s.notifyHasCustomSound = false; }
    }
    if ('speechSynthesis' in window) {
      const load = () => {
        const all = window.speechSynthesis.getVoices();
        voices = all.filter(v => v.lang.startsWith('ja') || v.name.toLowerCase().includes('japan'));
        if (!voices.length) voices = all;
      };
      load(); window.speechSynthesis.onvoiceschanged = load;
    }
    const unlisten = await listen<ChatMessage>('chat-message', ({ payload: msg }) => {
      messages = [...messages.slice(-999), msg];
      if (autoScroll && listEl) setTimeout(() => { listEl!.scrollTop = listEl!.scrollHeight; }, 0);
      speak(msg); checkKeywords(msg);
    });
    return unlisten;
  });
</script>

<!-- ────────── MARKUP ────────── -->
<div class="app" onclick={() => { blockPopup = null; }}>
  <header class="header">
    <nav class="tabs">
      {#each TABS as tab}
        <button class="tab" class:active={activeTab === tab.id}
          style:--tc={tab.id === 0 ? 'var(--accent)' : chColorById(tab.id)}
          onclick={() => { activeTab = tab.id; }}>{tab.name}</button>
      {/each}
    </nav>
    <button class="gear" class:active={panelOpen}
      onclick={() => { panelOpen = !panelOpen; }} title="設定">
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor"
        stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="3"/>
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
      </svg>
    </button>
  </header>

  <div class="messages" bind:this={listEl} onscroll={onScroll}>
    {#each filtered as msg (`${msg.timestamp}-${msg.sender_id}-${msg.text}`)}
      <div class="msg">
        <span class="time">{fmt(msg.timestamp)}</span>
        <span class="badge" style:color={chColorById(msg.channel)}>[{msg.channel_name}]</span>
        <span class="name clickable-name" onclick={(e) => showBlockPopup(e, msg.sender_name)}>{msg.sender_name}</span>
        <span class="sep">›</span>
        {#if isStamp(msg.text)}
          <span class="body stamp">🖼 スタンプ</span>
        {:else}
          <span class="body">{stripSprites(msg.text)}</span>
        {/if}
      </div>
    {:else}
      <div class="empty">
        <div class="empty-icon">💬</div>
        <div>チャットを待機中...</div>
        <div class="empty-sub">ゲームにログインするか、ch変更すると受信が始まります</div>
      </div>
    {/each}
  </div>

  {#if blockPopup}
    <div class="block-popup" style:left="{blockPopup.x}px" style:top="{blockPopup.y}px"
      onclick={(e) => e.stopPropagation()}>
      <span class="block-popup-name">{blockPopup.name}</span>
      <button class="block-btn" onclick={() => blockUser(blockPopup!.name)}>🚫 ブロック</button>
    </div>
  {/if}

  {#if !autoScroll}
    <button class="scroll-fab" onclick={scrollBottom}>↓ 最新へ</button>
  {/if}

  {#if panelOpen}
    <div class="backdrop" onclick={() => { panelOpen = false; }}></div>
  {/if}

  <aside class="panel" class:open={panelOpen}>
    <div class="panel-head">
      <span>設定</span>
      <button class="close-btn" onclick={() => { panelOpen = false; }}>✕</button>
    </div>
    <div class="panel-body">

      <!-- Theme -->
      <section class="sect">
        <h4 class="sect-title">テーマ</h4>
        <!-- 4×2 grid: 7 presets + 1 custom -->
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

      <!-- Font size -->
      <section class="sect">
        <h4 class="sect-title">フォントサイズ <em>{s.fontSize}px</em></h4>
        <input type="range" min="10" max="22" step="1"
          class="slider" bind:value={s.fontSize} oninput={save} />
        <div class="range-labels"><span>小</span><span>大</span></div>
      </section>

      <!-- Opacity -->
      <section class="sect">
        <h4 class="sect-title">背景の透明度 <em>{Math.round(s.bgOpacity * 100)}%</em></h4>
        <input type="range" min="0.05" max="1.0" step="0.05"
          class="slider" bind:value={s.bgOpacity} oninput={save} />
        <div class="range-labels"><span>透明</span><span>不透明</span></div>
      </section>

      <!-- Always on top -->
      <section class="sect">
        <label class="toggle-row">
          <span class="toggle-label">常に最前面に表示</span>
          <button class="tog" class:on={s.alwaysOnTop}
            onclick={() => setAlwaysOnTop(!s.alwaysOnTop)}>
            <span class="tog-knob"></span>
          </button>
        </label>
      </section>

      <!-- Keyword notify -->
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
              <span class="sublabel">監視キーワード（1行または , で区切り）</span>
              <textarea class="kw-area" placeholder={"例：ボス\nアリーナ\n救援"}
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
                  <span class="sound-name dim">デフォルト（ビープ音）</span>
                {/if}
                <label class="mini-btn file-label">ファイル選択
                  <input type="file" accept="audio/*" class="hidden-file" onchange={handleSoundFile} />
                </label>
              </div>
            </div>
            <div class="btn-row">
              <button class="test-btn" onclick={playNotification}>▶ テスト再生</button>
            </div>
          </div>
        {/if}
      </section>

      <!-- TTS -->
      <section class="sect">
        <label class="toggle-row">
          <span class="toggle-label">チャット読み上げ (TTS)</span>
          <button class="tog" class:on={s.ttsEnabled}
            onclick={() => { s.ttsEnabled = !s.ttsEnabled; save(); }}>
            <span class="tog-knob"></span>
          </button>
        </label>
        {#if s.ttsEnabled}
          <div class="tts-sub">
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
            {#if voices.length > 0}
              <div class="subsect">
                <span class="sublabel">音声</span>
                <select class="sel-voice" bind:value={s.ttsVoice} onchange={save}>
                  <option value="">デフォルト</option>
                  {#each voices as v}<option value={v.name}>{v.name}</option>{/each}
                </select>
              </div>
            {/if}
            <div class="btn-row">
              <button class="test-btn" onclick={testTts}>▶ テスト再生</button>
            </div>
          </div>
        {/if}
      </section>

      <!-- Blacklist -->
      <section class="sect">
        <h4 class="sect-title">ブラックリスト <em>{s.blacklist.length}</em></h4>
        {#if s.blacklist.length === 0}
          <p class="bl-empty">登録なし。名前をクリックして追加できます</p>
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

    </div>
  </aside>
</div>

<!-- ────────── STYLES ────────── -->
<style>
  .app {
    display: flex; flex-direction: column; height: 100vh;
    background: rgb(var(--bg) / var(--bg-opacity, 0.92));
    position: relative; overflow: hidden;
  }

  /* ── Header ── */
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

  .gear {
    width: 34px; height: 34px; display: flex; align-items: center; justify-content: center;
    border: none; background: transparent; color: var(--text-dim);
    cursor: pointer; border-radius: 6px; flex-shrink: 0;
    transition: color 0.15s, background 0.15s, transform 0.3s;
  }
  .gear:hover  { color: var(--accent); background: rgb(var(--bg) / 0.5); }
  .gear.active { color: var(--accent); transform: rotate(60deg); }

  /* ── Messages ── */
  .messages {
    flex: 1; overflow-y: auto; padding: 5px 8px;
    display: flex; flex-direction: column; gap: 2px;
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

  /* ── Panel ── */
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

  /* ── Sections ── */
  .sect { padding: 11px 14px; border-bottom: 1px solid var(--border); }
  .sect:last-child { border-bottom: none; }
  .sect-title { font-size: 0.78em; font-weight: 700; text-transform: uppercase; letter-spacing: 0.08em; color: var(--text-dim); margin-bottom: 9px; }
  .sect-title em { font-style: normal; color: var(--accent); margin-left: 6px; }

  /* ── Theme grid ── */
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
    /* fixed single-line — prevent text length from altering button height */
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

  /* ── Slider ── */
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

  /* ── Toggle ── */
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

  /* ── Sub-sections ── */
  .tts-sub { margin-top: 10px; display: flex; flex-direction: column; gap: 11px; }
  .subsect { display: flex; flex-direction: column; gap: 5px; }
  .sublabel { font-size: 0.79em; color: var(--text-dim); }
  .sublabel em { font-style: normal; color: var(--accent); margin-left: 4px; }

  /* ── Channel pills ── */
  .ch-pills { display: flex; flex-wrap: wrap; gap: 5px; }
  .pill {
    padding: 3px 10px; border-radius: 20px; border: 1.5px solid var(--border);
    background: none; cursor: pointer; font: inherit; font-size: 0.8em; color: var(--text-dim);
    transition: border-color 0.15s, color 0.15s;
  }
  .pill.on { border-color: var(--pc); color: var(--pc); font-weight: 700; }

  /* ── Keyword textarea ── */
  .kw-area {
    width: 100%; padding: 7px 8px; resize: vertical;
    background: var(--input-bg); border: 1px solid var(--border);
    color: var(--text); border-radius: 6px; font: inherit; font-size: 0.85em;
    outline: none; line-height: 1.5; min-height: 72px;
  }
  .kw-area:focus { border-color: var(--accent); }
  .kw-area::placeholder { color: var(--text-dim); }

  /* ── Sound file ── */
  .sound-row { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .sound-name { font-size: 0.8em; color: var(--text); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sound-name.dim { color: var(--text-dim); }
  .mini-btn {
    padding: 4px 10px; border-radius: 5px; cursor: pointer; font: inherit; font-size: 0.78em;
    background: var(--input-bg); border: 1px solid var(--border); color: var(--text);
    transition: background 0.15s, border-color 0.15s; flex-shrink: 0;
  }
  .mini-btn:hover { border-color: var(--accent); }
  .mini-btn.danger { color: #f87171; border-color: rgba(248,113,113,0.4); }
  .mini-btn.danger:hover { background: rgba(248,113,113,0.12); }
  .file-label { cursor: pointer; }
  .hidden-file { display: none; }

  /* ── Voice / buttons ── */
  .sel-voice {
    width: 100%; padding: 6px 8px;
    background: var(--input-bg); border: 1px solid var(--border);
    color: var(--text); border-radius: 6px; font: inherit; font-size: 0.85em; outline: none; cursor: pointer;
  }
  .sel-voice:focus { border-color: var(--accent); }
  .btn-row { display: flex; gap: 8px; }
  .test-btn {
    background: var(--input-bg); border: 1px solid var(--border); color: var(--accent);
    cursor: pointer; font: inherit; font-size: 0.82em; padding: 5px 12px; border-radius: 6px;
    transition: background 0.15s, border-color 0.15s;
  }
  .test-btn:hover { background: rgb(var(--surface) / 0.8); border-color: var(--accent); }

  /* ── Clickable name ── */
  .clickable-name {
    cursor: pointer; border-radius: 3px;
    transition: color 0.12s, background 0.12s;
  }
  .clickable-name:hover { color: var(--accent); background: rgb(var(--surface) / 0.6); }

  /* ── Block popup ── */
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

  /* ── Blacklist tags ── */
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
</style>
