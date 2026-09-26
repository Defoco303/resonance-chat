<script lang="ts">
  import {
    classColor,
    classIconUrl,
    classSpecLabel,
    DEFAULT_DPS_SETTINGS,
    DPS_SETTINGS_KEY,
    fmtCompact,
    fmtSeconds,
    isRealName,
    loadDpsSettings,
    loadHistory,
    loadIdCache,
    ENCOUNTER_END_LABELS,
    DPS_HISTORY_LIMIT,
    saveDpsSettings,
    saveHistory,
    saveIdCache,
    type DpsEncounterEndPayload,
    type DpsEncounterRecord,
    type DpsPlayerDetail,
    type DpsIdentity,
    type DpsMonsterInfo,
    type DpsPlayerRow,
    type DpsMetricPayload,
    type DpsMetricType,
    type DpsMeterPayload,
    type DpsScope,
    type DpsSettings,
  } from '$lib/dps';
  import { emit, listen } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
  import { flip } from 'svelte/animate';
  import { onMount, tick } from 'svelte';
  import DpsResultCard from './DpsResultCard.svelte';
  import PlayerDetailPanel from './DpsPlayerDetail.svelte';

  type ActiveTab = DpsMetricType | 'enemy' | 'history';
  type EnemyGroup = { id: number; name: string; eliteStatus: number; damageTaken: number; count: number; isBoss: boolean };
  const ENEMY_COLOR = '#f59e0b';
  const HISTORY_COLOR = '#a78bfa';
  const RESULT_CARD_MS = 15000;
  // リザルトカードを出す終わり方（全滅して戻る「移動」も含める）
  const RESULT_CARD_REASONS = new Set<DpsEncounterEndPayload['reason']>(['bossDefeated', 'sceneChange']);
  const METRIC_TABS: Array<{ id: DpsMetricType; label: string; color: string; rateLabel: string; totalLabel: string; empty: string }> = [
    { id: 'dps', label: 'DPS', color: '#f87171', rateLabel: 'DPS', totalLabel: 'DMG', empty: '戦闘データ待機中' },
    { id: 'tank', label: 'TANK', color: '#38bdf8', rateLabel: 'SCORE', totalLabel: 'TAKEN', empty: '被ダメージデータ待機中' },
    { id: 'heal', label: 'HEAL', color: '#34d399', rateLabel: 'HPS', totalLabel: 'HEAL', empty: '回復データ待機中' },
  ];

  let dpsMeter = $state<DpsMeterPayload>({
    totalDps: 0,
    totalDamage: 0,
    elapsedMs: 0,
    topDamage: 0,
    localPlayerUid: -1,
    bossEngaged: false,
    bossName: '',
    players: [],
    metrics: {
      dps: emptyMetric(),
      dpsBossOnly: emptyMetric(),
      tank: emptyMetric(),
      heal: emptyMetric(),
    },
  });
  let activeMetric = $state<ActiveTab>('dps');
  let scope = $state<DpsScope>('all');
  let settings = $state<DpsSettings>(loadDpsSettings());
  let settingsOpen = $state(false);
  let rankBursts = $state<Record<string, number>>({});
  let prevBossEngaged = false;
  let idCache = $state<Record<string, DpsIdentity>>(loadIdCache());
  let history = $state<DpsEncounterRecord[]>(loadHistory());
  let resultCard = $state<DpsEncounterRecord | null>(null);
  // リザルトカードは RESULT_CARD_MS で自動で閉じる（✕ でも閉じられる）
  let resultCardTimer: ReturnType<typeof setTimeout> | null = null;
  // プレイヤー詳細・自分との比較（戦闘中の一覧から、または履歴から開く）
  let detailView = $state<{ source: 'live' | 'history'; uid: number } | null>(null);
  let liveDetails = $state<DpsPlayerDetail[]>([]);
  let liveDetailsBossOnly = $state(false);
  let liveDetailTimer: ReturnType<typeof setInterval> | null = null;
  // 履歴タブで表示中の戦闘（history[0] が最新）
  let selectedHistoryIndex = $state(0);
  const selectedHistory = $derived(history[Math.min(selectedHistoryIndex, history.length - 1)] ?? null);
  const detailList = $derived(
    detailView?.source === 'history' ? (selectedHistory?.details ?? []) : liveDetails
  );
  const detailSelfUid = $derived(
    detailView?.source === 'history' ? (selectedHistory?.localPlayerUid ?? -1) : dpsMeter.localPlayerUid
  );
  const currentDetail = $derived(detailView ? detailList.find((d) => d.uid === detailView?.uid) : undefined);
  const selfDetail = $derived(detailList.find((d) => d.uid === detailSelfUid));
  const detailBossOnly = $derived(
    detailView?.source === 'history' ? Boolean(selectedHistory?.bossOnly) : liveDetailsBossOnly
  );
  const effectiveScope = $derived<DpsScope>(activeMetric === 'dps' ? scope : 'all');
  const currentMetric = $derived(
    activeMetric === 'enemy' || activeMetric === 'history' ? emptyMetric() : resolveMetric(dpsMeter, activeMetric, effectiveScope)
  );
  const currentTab = $derived(metricTab(activeMetric));
  const displayPlayers = $derived(orderPlayers(currentMetric.players, activeMetric));
  const barMax = $derived(
    activeMetric === 'tank'
      ? displayPlayers.reduce((max, row) => Math.max(max, tankScoreOf(row)), 0)
      : currentMetric.topValue
  );
  const groupedEnemies = $derived(groupEnemies(dpsMeter.enemies ?? []));

  function updateClassColor(className: string, color: string) {
    settings = {
      ...settings,
      classColors: settings.classColors.map((entry) =>
        entry.className === className ? { ...entry, color } : entry
      ),
    };
    saveDpsSettings(settings);
  }

  function resetClassColors() {
    settings = {
      ...settings,
      classColors: DEFAULT_DPS_SETTINGS.classColors.map((entry) => ({ ...entry })),
    };
    saveDpsSettings(settings);
  }

  // 調査用ログ（通信とダメージの記録）。設定はアプリ側に保存される
  let diagLogging = $state(false);

  async function updateDiagLogging(value: boolean) {
    try {
      diagLogging = await invoke<boolean>('set_diag_logging', { enabled: value });
    } catch {
      diagLogging = false;
    }
  }

  function updateAutoBossScope(value: boolean) {
    settings = { ...settings, autoBossScope: value };
    saveDpsSettings(settings);
  }

  function updateRankAnimation(rankAnimation: DpsSettings['rankAnimation']) {
    settings = { ...settings, rankAnimation };
    if (rankAnimation !== 'flashy') {
      rankBursts = {};
    }
    saveDpsSettings(settings);
  }

  function emptyMetric(): DpsMetricPayload {
    return {
      totalValue: 0,
      totalRate: 0,
      topValue: 0,
      players: [],
    };
  }

  function metricTab(metric: ActiveTab) {
    return METRIC_TABS.find((tab) => tab.id === metric) ?? METRIC_TABS[0];
  }

  function metricWindow(payload: DpsMeterPayload, metric: DpsMetricType): DpsMetricPayload {
    const fromPayload = payload.metrics?.[metric];
    if (fromPayload) return fromPayload;
    if (metric === 'dps') {
      return {
        totalValue: payload.totalDamage,
        totalRate: payload.totalDps,
        topValue: payload.topDamage,
        players: payload.players,
      };
    }
    return emptyMetric();
  }

  // TANK is ranked by a dedicated score that favors single-target enemy hits
  // and discounts multi-target damage, instead of raw damage taken.
  function orderPlayers(players: DpsPlayerRow[], metric: ActiveTab): DpsPlayerRow[] {
    if (metric !== 'tank') return players;
    return [...players].sort((a, b) =>
      tankScoreOf(b) - tankScoreOf(a)
      || b.hits - a.hits
      || b.totalDamage - a.totalDamage
    );
  }

  function tankScoreOf(row: DpsPlayerRow) {
    return Number.isFinite(row.tankScore) ? row.tankScore ?? 0 : row.hits;
  }

  function avgTakenOf(row: DpsPlayerRow) {
    if (Number.isFinite(row.avgTaken)) return row.avgTaken ?? 0;
    return row.hits > 0 ? row.totalDamage / row.hits : 0;
  }

  function fmtTankScore(value: number) {
    if (!Number.isFinite(value) || value <= 0) return '0';
    if (value >= 1000) return fmtCompact(value);
    const digits = value >= 100 ? 0 : value >= 10 ? 1 : 2;
    return value.toFixed(digits).replace(/\.0+$/, '').replace(/(\.\d*[1-9])0+$/, '$1');
  }

  function playerBuildLabel(row: DpsPlayerRow) {
    if (Number.isFinite(row.abilityScore) && row.abilityScore > 0) {
      return `BP ${fmtCompact(row.abilityScore)}`;
    }
    return '';
  }

  function resolveMetric(payload: DpsMeterPayload, metric: DpsMetricType, sc: DpsScope): DpsMetricPayload {
    if (metric === 'dps' && sc === 'boss') {
      return payload.metrics?.dpsBossOnly ?? emptyMetric();
    }
    return metricWindow(payload, metric);
  }

  // Persist any real names/classes we learn, keyed by uid, so a player whose
  // identity packet was missed this session can be filled from a prior one.
  function learnIdentities(payload: DpsMeterPayload) {
    const rows = [
      ...(payload.players ?? []),
      ...(payload.metrics
        ? [
            ...payload.metrics.dps.players,
            ...payload.metrics.dpsBossOnly.players,
            ...payload.metrics.tank.players,
            ...payload.metrics.heal.players,
          ]
        : []),
    ];
    let changed = false;
    const next = { ...idCache };
    for (const row of rows) {
      const key = String(row.uid);
      const cur = { ...(next[key] ?? {}) };
      let updated = false;
      if (isRealName(row.name) && cur.name !== row.name) { cur.name = row.name; updated = true; }
      if (updated) { next[key] = { name: cur.name }; changed = true; }
    }
    if (changed) {
      idCache = next;
      saveIdCache(idCache);
    }
  }

  function effName(uid: number, name: string) {
    return isRealName(name) ? name : (idCache[String(uid)]?.name ?? name);
  }

  // 職業はゲーム内で頻繁に切り替えられるため、保存済みの職業・型は使わず今の値だけを表示する
  function effClass(_uid: number, className: string) {
    return className;
  }

  function effClassSpec(_uid: number, classSpecName: string) {
    return classSpecName;
  }

  function rowClassSpecLabel(row: DpsPlayerRow) {
    return classSpecLabel(effClassSpec(row.uid, row.classSpecName));
  }

  function setActiveMetric(metric: ActiveTab) {
    closeDetail();
    activeMetric = metric;
    if (metric === 'history') selectedHistoryIndex = 0;
    rankBursts = {};
    settingsOpen = false;
  }

  function setScope(next: DpsScope) {
    scope = next;
    rankBursts = {};
  }

  function groupEnemies(list: DpsMonsterInfo[]): EnemyGroup[] {
    const map = new Map<number, EnemyGroup>();
    for (const e of list) {
      const g = map.get(e.id);
      if (g) {
        g.count += 1;
        g.damageTaken += e.damageTaken;
        if (e.eliteStatus > g.eliteStatus) g.eliteStatus = e.eliteStatus;
        if (e.isBoss) g.isBoss = true;
        if (e.id !== 0 && e.name && e.name !== g.name) g.name = e.name;
      } else {
        map.set(e.id, {
          id: e.id,
          name: e.name,
          eliteStatus: e.eliteStatus,
          damageTaken: e.damageTaken,
          count: 1,
          isBoss: e.isBoss,
        });
      }
    }
    return [...map.values()].sort((a, b) => b.damageTaken - a.damageTaken);
  }

  function markRankUps(next: DpsMeterPayload) {
    if (settings.rankAnimation !== 'flashy') return;
    if (activeMetric === 'enemy' || activeMetric === 'history') return;

    const previousRanks = new Map(orderPlayers(resolveMetric(dpsMeter, activeMetric, effectiveScope).players, activeMetric).map((row, index) => [row.uid, index]));
    const nextBursts = { ...rankBursts };
    let changed = false;

    orderPlayers(resolveMetric(next, activeMetric, effectiveScope).players, activeMetric).forEach((row, index) => {
      const previous = previousRanks.get(row.uid);
      if (previous === undefined || index >= previous) return;

      const key = rankBurstKey(row.uid);
      const token = Date.now() + index;
      nextBursts[key] = token;
      changed = true;

      window.setTimeout(() => {
        if (rankBursts[key] !== token) return;
        const { [key]: _removed, ...rest } = rankBursts;
        rankBursts = rest;
      }, 520);
    });

    if (changed) {
      rankBursts = nextBursts;
    }
  }

  function applyChatTheme(override?: Record<string, unknown>) {
    try {
      let parsed: Record<string, unknown>;
      if (override) {
        parsed = override;
      } else {
        const saved = localStorage.getItem('rchat');
        if (!saved) return;
        parsed = JSON.parse(saved) as Record<string, unknown>;
      }
      const root = document.documentElement;
      root.setAttribute('data-theme', typeof parsed.theme === 'string' ? parsed.theme : 'dark');
      if (Number.isFinite(parsed.fontSize)) {
        root.style.setProperty('--font-size', `${parsed.fontSize}px`);
      }
      if (Number.isFinite(parsed.bgOpacity)) {
        root.style.setProperty('--bg-opacity', String(parsed.bgOpacity));
      }
      if (parsed.theme === 'custom' && parsed.customTheme) {
        applyCustomVars(parsed.customTheme as Record<string, string>);
      } else {
        // Remove inline vars left by a previous custom theme so the
        // stylesheet rules for the selected preset take effect again.
        for (const p of CUSTOM_PROPS) root.style.removeProperty(p);
      }
    } catch {}
  }

  const CUSTOM_PROPS = [
    '--bg', '--surface', '--border', '--text', '--text-dim',
    '--accent', '--accent-glow', '--input-bg', '--thumb',
    '--c-world', '--c-guild', '--c-party', '--c-channel',
  ];

  function applyCustomVars(ct: Record<string, string>) {
    const root = document.documentElement;
    const pairs = [
      ['--bg', hexToRgb(ct.bg)],
      ['--surface', hexToRgb(ct.surface)],
      ['--border', ct.border],
      ['--text', ct.text],
      ['--text-dim', ct.textDim],
      ['--accent', ct.accent],
      ['--thumb', ct.accent],
      ['--c-world', ct.cWorld],
      ['--c-guild', ct.cGuild],
      ['--c-party', ct.cParty],
      ['--c-channel', ct.cChannel],
    ] as const;
    for (const [key, value] of pairs) {
      if (value) root.style.setProperty(key, value);
    }
    const accent = parseRgb(ct.accent);
    if (accent) {
      root.style.setProperty('--accent-glow', `rgba(${accent[0]},${accent[1]},${accent[2]},0.35)`);
    }
    root.style.setProperty('--input-bg', 'rgba(255 255 255 / 0.05)');
  }

  function hexToRgb(hex?: string) {
    const rgb = parseRgb(hex);
    return rgb ? `${rgb[0]} ${rgb[1]} ${rgb[2]}` : '';
  }

  function parseRgb(hex?: string) {
    if (!hex || !/^#[0-9a-fA-F]{6}$/.test(hex)) return null;
    const n = parseInt(hex.slice(1), 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }

  function rowColor(className: string) {
    return classColor(settings, className);
  }

  function toEncounterRecord(event: DpsEncounterEndPayload): DpsEncounterRecord {
    const result = event.result;
    const named = (row: DpsPlayerRow) => ({ ...row, name: effName(row.uid, row.name) });
    // ボス戦はボス本体へのダメージだけで集計する（呼び出された雑魚へのダメージは含めない）
    const bossOnly = result.metrics?.dpsBossOnly;
    const isBossFight = Boolean(result.bossEngaged && bossOnly && bossOnly.players.length > 0);
    const dps = isBossFight && bossOnly ? bossOnly : metricWindow(result, 'dps');
    const topHeal = metricWindow(result, 'heal').players[0];
    const topTank = orderPlayers(metricWindow(result, 'tank').players, 'tank')[0];
    return {
      id: `${event.endedAtMs}-${Math.random().toString(36).slice(2, 8)}`,
      reason: event.reason,
      endedAtMs: event.endedAtMs,
      bossName: result.bossName ?? '',
      elapsedMs: result.elapsedMs,
      totalDps: dps.totalRate,
      totalDamage: dps.totalValue,
      bossOnly: isBossFight,
      localPlayerUid: result.localPlayerUid,
      players: dps.players.map(named),
      topHeal: topHeal ? named(topHeal) : undefined,
      topTank: topTank ? named(topTank) : undefined,
      details: event.details.map((detail) => ({ ...detail, name: effName(detail.uid, detail.name) })),
    };
  }

  function handleEncounterEnd(event: DpsEncounterEndPayload) {
    learnIdentities(event.result);
    const record = toEncounterRecord(event);
    // ダメージ0の戦闘（回復だけなど）は履歴に残さない
    if (record.totalDamage <= 0) return;
    // 履歴に残すのはボスと戦った戦闘だけ
    if (!record.bossOnly) return;
    // 履歴を見ている最中に新しい記録が増えても、表示中の戦闘がずれないようにする
    if (history.length > 0) {
      selectedHistoryIndex = Math.min(selectedHistoryIndex + 1, DPS_HISTORY_LIMIT - 1);
    }
    history = [record, ...history].slice(0, DPS_HISTORY_LIMIT);
    saveHistory(history);
    if (RESULT_CARD_REASONS.has(record.reason)) {
      showResultCard(record);
    }
  }

  function showResultCard(record: DpsEncounterRecord) {
    resultCard = record;
    if (resultCardTimer) clearTimeout(resultCardTimer);
    resultCardTimer = setTimeout(closeResultCard, RESULT_CARD_MS);
  }

  function selectHistory(index: number) {
    closeDetail();
    selectedHistoryIndex = Math.max(0, Math.min(index, history.length - 1));
    void tick().then(() => {
      document.querySelector('.history-row.selected')?.scrollIntoView({ block: 'nearest' });
    });
  }

  // -1 で新しい戦闘へ、+1 で古い戦闘へ
  function stepHistory(delta: number) {
    selectHistory(selectedHistoryIndex + delta);
  }

  function onHistoryKeydown(event: KeyboardEvent) {
    if (activeMetric !== 'history' || history.length === 0) return;
    if ((event.target as HTMLElement | null)?.closest('input, textarea')) return;
    if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') {
      event.preventDefault();
      stepHistory(-1);
    } else if (event.key === 'ArrowDown' || event.key === 'ArrowRight') {
      event.preventDefault();
      stepHistory(1);
    }
  }

  function openDetail(source: 'live' | 'history', uid: number) {
    closeDetail();
    detailView = { source, uid };
    if (source === 'live') {
      void refreshLiveDetails();
      // 戦闘中は1秒ごとに更新する
      liveDetailTimer = setInterval(() => { void refreshLiveDetails(); }, 1000);
    }
  }

  function closeDetail() {
    detailView = null;
    if (liveDetailTimer) {
      clearInterval(liveDetailTimer);
      liveDetailTimer = null;
    }
  }

  async function refreshLiveDetails() {
    const bossOnly = effectiveScope === 'boss';
    try {
      const details = await invoke<DpsPlayerDetail[]>('get_dps_details', { bossOnly });
      liveDetails = details;
      liveDetailsBossOnly = bossOnly;
    } catch {}
  }

  function closeResultCard() {
    resultCard = null;
    if (resultCardTimer) {
      clearTimeout(resultCardTimer);
      resultCardTimer = null;
    }
  }

  function historyTimeLabel(ms: number) {
    const date = new Date(ms);
    return `${date.getHours().toString().padStart(2, '0')}:${date.getMinutes().toString().padStart(2, '0')}`;
  }

  function rankBurstKey(uid: number) {
    return `${activeMetric}:${uid}`;
  }

  async function closeWindow() {
    await emit('dps-window-closed');
    await invoke('close_dps_window');
  }

  async function resetMeter() {
    await invoke('reset_dps_meter').catch(() => {});
  }

  function beginDrag(event: PointerEvent) {
    if ((event.target as HTMLElement).closest('button, input')) return;
    void getCurrentWebviewWindow().startDragging();
  }

  onMount(async () => {
    applyChatTheme();
    invoke<boolean>('get_diag_logging')
      .then((value) => { diagLogging = value; })
      .catch(() => {});
    await emit('dps-window-opened');
    const unlistenDps = await listen<DpsMeterPayload>('dps-meter', ({ payload }) => {
      const bossEngaged = Boolean(payload.bossEngaged);
      if (settings.autoBossScope) {
        if (bossEngaged && !prevBossEngaged) {
          scope = 'boss';
        } else if (!bossEngaged && prevBossEngaged) {
          scope = 'all';
        }
      }
      prevBossEngaged = bossEngaged;
      learnIdentities(payload);
      markRankUps(payload);
      dpsMeter = payload;
    });
    const unlistenEnd = await listen<DpsEncounterEndPayload>('dps-encounter-end', ({ payload }) => {
      handleEncounterEnd(payload);
    });
    const unlistenTheme = await listen<Record<string, unknown>>('rchat-theme', ({ payload }) => {
      applyChatTheme(payload);
    });
    const unlistenClose = await getCurrentWebviewWindow().onCloseRequested(() => {
      void emit('dps-window-closed');
    });
    const onStorage = (event: StorageEvent) => {
      if (event.key === DPS_SETTINGS_KEY) {
        settings = loadDpsSettings();
      } else if (event.key === 'rchat') {
        applyChatTheme();
      }
    };
    window.addEventListener('storage', onStorage);
    window.addEventListener('keydown', onHistoryKeydown);

    return () => {
      unlistenDps();
      unlistenEnd();
      unlistenTheme();
      unlistenClose();
      window.removeEventListener('storage', onStorage);
      window.removeEventListener('keydown', onHistoryKeydown);
      if (resultCardTimer) clearTimeout(resultCardTimer);
      closeDetail();
      void emit('dps-window-closed');
    };
  });
</script>

<div class="dps-app">
  <header class="dps-header" role="toolbar" tabindex="-1" onpointerdown={beginDrag}>
    <div class="title">
      <span>DPS Meter</span>
      <em>{fmtSeconds(dpsMeter.elapsedMs)}</em>
    </div>
    <div class="window-actions">
      <button class="icon-btn" title="リセット" aria-label="リセット" onclick={() => { void resetMeter(); }}>
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M3 12a9 9 0 1 0 3-6.7"></path>
          <path d="M3 3v6h6"></path>
        </svg>
      </button>
      <button
        class="icon-btn"
        class:active={settingsOpen}
        title="DPS設定"
        aria-label="DPS設定"
        onclick={() => { settingsOpen = !settingsOpen; }}
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="12" cy="12" r="3"/>
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>
        </svg>
      </button>
      <button class="icon-btn" title="閉じる" aria-label="閉じる" onclick={() => { void closeWindow(); }}>
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M18 6 6 18"></path>
          <path d="m6 6 12 12"></path>
        </svg>
      </button>
    </div>
  </header>

  <div class="metric-filter">
    <div class="metric-pills">
      {#each METRIC_TABS as tab}
        <button
          class="metric-pill"
          class:on={activeMetric === tab.id}
          style:--pc={tab.color}
          onclick={() => setActiveMetric(tab.id)}
        >{tab.label}</button>
      {/each}
      <button
        class="metric-pill"
        class:on={activeMetric === 'enemy'}
        style:--pc={ENEMY_COLOR}
        onclick={() => setActiveMetric('enemy')}
      >ENEMY</button>
      <button
        class="metric-pill"
        class:on={activeMetric === 'history'}
        style:--pc={HISTORY_COLOR}
        onclick={() => setActiveMetric('history')}
      >履歴</button>
    </div>
    {#if activeMetric === 'dps'}
      <div class="scope-toggle" role="group" aria-label="集計範囲">
        <button class:on={scope === 'all'} onclick={() => setScope('all')}>全体</button>
        <button class:on={scope === 'boss'} onclick={() => setScope('boss')}>ボス</button>
      </div>
    {/if}
  </div>
  {#if activeMetric === 'dps' && scope === 'boss'}
    <div class="boss-banner" title={dpsMeter.bossName || undefined}>
      <span class="boss-banner-label">BOSS</span>
      <span class="boss-banner-name">{dpsMeter.bossName || '対象なし'}</span>
    </div>
  {/if}

  {#if activeMetric === 'enemy'}
    <main class="dps-list">
      {#each groupedEnemies as enemy (enemy.id)}
        <div class="enemy-row" class:boss={enemy.isBoss}>
          <div class="enemy-info">
            <span class="enemy-name">{enemy.name}</span>
            <span class="enemy-sub">
              {enemy.id === 0 ? 'ID不明' : `ID ${enemy.id}`}{#if enemy.count > 1} ({enemy.count}){/if} ・ {fmtCompact(enemy.damageTaken)} DMG{#if enemy.eliteStatus > 0} ・ ELITE{/if}
            </span>
          </div>
          <span class="enemy-toggle" class:boss={enemy.isBoss}>{enemy.isBoss ? 'ボス' : '雑魚'}</span>
        </div>
      {:else}
        <div class="dps-empty">交戦中の敵なし</div>
      {/each}
    </main>
  {:else if activeMetric === 'history'}
    <main class="history-view">
      {#if history.length === 0}
        <div class="dps-empty">戦闘が終わると、ここに記録が残ります</div>
      {:else}
        {#if selectedHistory}
          <div class="history-detail">
            <DpsResultCard
              record={selectedHistory}
              colorOf={rowColor}
              onSelectPlayer={selectedHistory.details ? (uid) => openDetail('history', uid) : undefined}
              nav={{
                index: selectedHistoryIndex,
                total: history.length,
                onPrev: () => stepHistory(-1),
                onNext: () => stepHistory(1),
              }}
            />
          </div>
        {/if}
        <div class="history-list" role="listbox" aria-label="戦闘履歴">
          {#each history as record, index (record.id)}
            <button
              class="history-row"
              class:selected={index === selectedHistoryIndex}
              role="option"
              aria-selected={index === selectedHistoryIndex}
              onclick={() => selectHistory(index)}
            >
              <span class="history-time">{historyTimeLabel(record.endedAtMs)}</span>
              <span class="history-info">
                <strong>{record.bossName || '戦闘'}</strong>
                <span>{ENCOUNTER_END_LABELS[record.reason]} ・ {fmtSeconds(record.elapsedMs)}</span>
              </span>
              <span class="history-value">{fmtCompact(record.totalDps)} DPS</span>
            </button>
          {/each}
        </div>
      {/if}
    </main>
  {:else}
  <main class="dps-list">
    {#each displayPlayers as row (row.uid)}
      <div
        class="dps-row"
        class:local={row.uid === dpsMeter.localPlayerUid}
        class:rank-up={settings.rankAnimation === 'flashy' && Boolean(rankBursts[rankBurstKey(row.uid)])}
        animate:flip={{ duration: settings.rankAnimation === 'off' ? 0 : 220 }}
        style:--bar={`${barMax > 0 ? Math.max(3, (activeMetric === 'tank' ? tankScoreOf(row) : row.totalDamage) / barMax * 100) : 0}%`}
        style:--class-color={rowColor(effClass(row.uid, row.className))}
        class:clickable={activeMetric === 'dps'}
        role="button"
        tabindex={activeMetric === 'dps' ? 0 : -1}
        aria-disabled={activeMetric !== 'dps'}
        title={activeMetric === 'dps' ? '詳細・自分との比較を見る' : undefined}
        onclick={() => { if (activeMetric === 'dps') openDetail('live', row.uid); }}
        onkeydown={(event) => { if (activeMetric === 'dps' && event.key === 'Enter') openDetail('live', row.uid); }}
      >
        <div class="dps-class">
          <div class="dps-class-icon" aria-hidden="true">
            {#if classIconUrl(effClass(row.uid, row.className))}
              <img src={classIconUrl(effClass(row.uid, row.className))} alt="" />
            {:else}
              <span>?</span>
            {/if}
          </div>
          {#if rowClassSpecLabel(row)}
            <span class="dps-spec-name">{rowClassSpecLabel(row)}</span>
          {/if}
        </div>
        <div class="dps-player">
          <span class="dps-name">{effName(row.uid, row.name)}</span>
          {#if playerBuildLabel(row)}
            <span class="dps-build">{playerBuildLabel(row)}</span>
          {/if}
        </div>
        <div class="dps-values">
          {#if activeMetric === 'tank'}
            <strong>{fmtTankScore(tankScoreOf(row))} TANK</strong>
            <span title={`平均 ${fmtCompact(avgTakenOf(row))} / Hit`}>
              {fmtCompact(row.hits)}回 / {fmtCompact(row.totalDamage)}被ダメ
            </span>
          {:else}
            <strong>{fmtCompact(row.dps)}</strong>
            <span>
              {fmtCompact(row.totalDamage)} {currentTab.totalLabel} / {row.damagePct.toFixed(1)}%{#if activeMetric === 'dps' && (row.tdps ?? 0) > 0}<span
                  title="実際に攻撃していた時間で割ったDPS"
                > / 実働 {fmtCompact(row.tdps ?? 0)}</span>{/if}
            </span>
          {/if}
        </div>
      </div>
    {:else}
      <div class="dps-empty">{currentTab.empty}</div>
    {/each}
  </main>
  {/if}

  {#if resultCard}
    <div class="result-overlay" aria-live="polite">
      <DpsResultCard record={resultCard} colorOf={rowColor} onClose={closeResultCard} showRanking={false} />
    </div>
  {/if}

  {#if detailView}
    <div class="detail-overlay">
      {#if currentDetail}
        <PlayerDetailPanel
          detail={currentDetail}
          self={selfDetail}
          bossOnly={detailBossOnly}
          colorOf={rowColor}
          onBack={closeDetail}
        />
      {:else}
        <div class="detail-missing">
          <p>このプレイヤーの内訳はまだありません。</p>
          <button class="mini-btn" onclick={closeDetail}>戻る</button>
        </div>
      {/if}
    </div>
  {/if}

  {#if settingsOpen}
    <button
      class="settings-backdrop"
      aria-label="DPS設定を閉じる"
      onclick={() => { settingsOpen = false; }}
    ></button>
    <aside class="settings-panel">
      <div class="settings-head">
        <span>ボス表示</span>
      </div>
      <label class="toggle-row">
        <input
          type="checkbox"
          checked={settings.autoBossScope}
          onchange={(event) => updateAutoBossScope((event.currentTarget as HTMLInputElement).checked)}
        />
        <span>ボス戦になったら自動でボス表示に切り替える</span>
      </label>
      <div class="settings-head">
        <span>順位演出</span>
      </div>
      <div class="rank-mode" role="group" aria-label="順位演出">
        <button
          class:active={settings.rankAnimation === 'flashy'}
          onclick={() => updateRankAnimation('flashy')}
        >派手</button>
        <button
          class:active={settings.rankAnimation === 'normal'}
          onclick={() => updateRankAnimation('normal')}
        >通常</button>
        <button
          class:active={settings.rankAnimation === 'off'}
          onclick={() => updateRankAnimation('off')}
        >OFF</button>
      </div>
      <div class="settings-head">
        <span>調査用ログ</span>
      </div>
      <label class="toggle-row">
        <input
          type="checkbox"
          checked={diagLogging}
          onchange={(event) => updateDiagLogging((event.currentTarget as HTMLInputElement).checked)}
        />
        <span>通信とダメージの記録を保存する（不具合の調査用。最新5回分まで）</span>
      </label>
      <div class="settings-head">
        <span>職業カラー</span>
        <button class="mini-btn" onclick={resetClassColors}>リセット</button>
      </div>
      <div class="color-list">
        {#each settings.classColors as entry (entry.className)}
          <label class="color-row">
            <span class="swatch" style:background={entry.color}></span>
            <span>{entry.label}</span>
            <input
              type="color"
              value={entry.color}
              oninput={(event) => updateClassColor(entry.className, (event.currentTarget as HTMLInputElement).value)}
            />
          </label>
        {/each}
      </div>
    </aside>
  {/if}
</div>
<style>
  .dps-app {
    position: relative;
    height: 100vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: rgb(var(--bg) / var(--bg-opacity, 0.92));
    border: 1px solid var(--border);
    color: var(--text);
  }

  .dps-header {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 38px;
    padding: 5px 5px 5px 10px;
    background: rgb(var(--surface) / 0.97);
    border-bottom: 1px solid var(--border);
    cursor: move;
  }
  .title {
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 8px;
    color: var(--accent);
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    font-size: 0.84em;
  }
  .title em {
    color: var(--text-dim);
    font-style: normal;
    font-size: 0.82em;
    letter-spacing: 0;
  }
  .window-actions {
    display: flex;
    align-items: center;
    gap: 3px;
    flex-shrink: 0;
  }
  .icon-btn {
    width: 30px;
    height: 30px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .icon-btn:hover,
  .icon-btn.active {
    color: var(--accent);
    background: rgb(var(--bg) / 0.5);
  }

  .metric-filter {
    flex-shrink: 0;
    padding: 5px 6px 4px;
    background: rgb(var(--surface) / 0.82);
    border-bottom: 1px solid var(--border);
  }
  .metric-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    min-width: 0;
  }
  .metric-pill {
    padding: 3px 10px;
    border-radius: 20px;
    border: 1.5px solid var(--border);
    background: none;
    cursor: pointer;
    font: inherit;
    font-size: 0.8em;
    color: var(--text-dim);
    transition: border-color 0.15s, color 0.15s, background 0.15s;
  }
  .metric-pill:hover,
  .metric-pill.on {
    border-color: var(--pc);
    color: var(--pc);
    font-weight: 700;
    background: color-mix(in srgb, var(--pc) 9%, transparent);
  }

  .dps-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 3px 0;
  }
  .dps-list::-webkit-scrollbar { width: 3px; }
  .dps-list::-webkit-scrollbar-thumb { background: var(--border); border-radius: 2px; }
  .dps-row {
    position: relative;
    display: grid;
    grid-template-columns: minmax(64px, 72px) minmax(0, 1fr) minmax(88px, auto);
    align-items: center;
    gap: 6px;
    min-height: 32px;
    margin: 0 3px 2px;
    padding: 3px 6px 3px 3px;
    border: 1px solid color-mix(in srgb, var(--class-color) 38%, rgb(var(--surface)));
    border-radius: 5px;
    background:
      linear-gradient(180deg, rgb(var(--surface) / 0.42), rgb(var(--bg) / 0.34));
    overflow: hidden;
  }
  .dps-row::before {
    content: '';
    position: absolute;
    inset: 0 auto 0 0;
    width: var(--bar);
    background:
      linear-gradient(180deg,
        color-mix(in srgb, var(--class-color) 30%, transparent) 0%,
        color-mix(in srgb, var(--class-color) 58%, transparent) 100%);
    box-shadow:
      inset 0 1px 0 rgb(255 255 255 / 0.12),
      inset 0 -1px 0 rgb(0 0 0 / 0.24);
  }
  .dps-row::after {
    content: '';
    position: absolute;
    inset: 0;
    background:
      linear-gradient(180deg, rgb(255 255 255 / 0.075), transparent 42%, rgb(0 0 0 / 0.16));
    pointer-events: none;
  }
  .dps-row.local {
    border-color: color-mix(in srgb, var(--class-color) 62%, var(--border));
  }
  .dps-row.rank-up {
    z-index: 8;
    animation: rank-pop 520ms cubic-bezier(0.16, 1, 0.3, 1);
    border-color: color-mix(in srgb, var(--class-color) 82%, white);
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--class-color) 45%, transparent),
      0 8px 22px color-mix(in srgb, var(--class-color) 35%, transparent),
      0 12px 24px rgb(0 0 0 / 0.36);
  }
  .dps-row.local::before {
    background:
      linear-gradient(180deg,
        color-mix(in srgb, var(--class-color) 42%, transparent) 0%,
        color-mix(in srgb, var(--class-color) 70%, transparent) 100%);
  }
  @keyframes rank-pop {
    0% {
      translate: 0 5px;
      scale: 0.985;
      filter: brightness(1);
    }
    38% {
      translate: 0 -4px;
      scale: 1.045;
      filter: brightness(1.24);
    }
    70% {
      translate: 0 1px;
      scale: 1.015;
      filter: brightness(1.1);
    }
    100% {
      translate: 0 0;
      scale: 1;
      filter: brightness(1);
    }
  }
  .dps-class-icon,
  .dps-class,
  .dps-player,
  .dps-values {
    position: relative;
    z-index: 1;
  }
  .dps-class {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .dps-class-icon {
    flex: 0 0 auto;
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 5px;
    background: rgb(var(--bg) / 0.44);
    box-shadow:
      inset 0 0 0 1px color-mix(in srgb, var(--class-color) 52%, transparent),
      0 3px 10px rgb(0 0 0 / 0.18);
    overflow: hidden;
  }
  .dps-class-icon img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .dps-class-icon span {
    color: var(--text-dim);
    font-size: 0.72em;
    font-weight: 800;
  }
  .dps-spec-name {
    min-width: 0;
    color: color-mix(in srgb, var(--class-color) 72%, white);
    font-size: 0.62em;
    font-weight: 900;
    line-height: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    text-shadow: 0 0 8px color-mix(in srgb, var(--class-color) 28%, transparent);
    white-space: nowrap;
  }
  .dps-player {
    min-width: 0;
    line-height: 1.05;
  }
  .dps-name {
    display: block;
    color: var(--text);
    font-size: 0.82em;
    font-weight: 800;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dps-build {
    display: block;
    margin-top: 2px;
    color: color-mix(in srgb, var(--class-color) 46%, var(--text-dim));
    font-size: 0.58em;
    font-weight: 800;
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dps-values {
    min-width: 0;
    text-align: right;
    line-height: 1.08;
  }
  .dps-values strong {
    display: block;
    color: var(--class-color);
    font-size: 0.84em;
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-shadow: 0 0 12px color-mix(in srgb, var(--class-color) 34%, transparent);
  }
  .dps-values span {
    display: block;
    color: var(--text-dim);
    font-size: 0.62em;
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dps-empty {
    padding: 22px 12px;
    color: var(--text-dim);
    font-size: 0.85em;
    text-align: center;
  }

  .detail-overlay {
    position: absolute;
    inset: 69px 0 0;
    z-index: 16;
    overflow-y: auto;
    background: rgb(var(--bg) / 0.97);
    border-top: 1px solid var(--border);
  }
  .detail-overlay::-webkit-scrollbar { width: 3px; }
  .detail-overlay::-webkit-scrollbar-thumb { background: var(--border); border-radius: 2px; }
  .detail-missing {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 24px 12px;
    color: var(--text-dim);
    font-size: 0.85em;
  }
  .dps-row.clickable {
    cursor: pointer;
  }
  .result-overlay {
    position: absolute;
    inset: 69px 6px auto;
    z-index: 15;
    max-height: calc(100% - 75px);
    overflow-y: auto;
    box-shadow: 0 8px 24px rgba(0 0 0 / 0.35);
    border-radius: 10px;
  }
  .history-row {
    display: grid;
    grid-template-columns: 38px minmax(0, 1fr) auto;
    align-items: center;
    gap: 8px;
    margin: 0 3px 2px;
    width: calc(100% - 6px);
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: rgb(var(--surface) / 0.6);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .history-row:hover {
    border-color: var(--accent);
  }
  .history-row.selected {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, rgb(var(--surface)));
  }
  .history-view {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 3px 0;
  }
  /* カードを上に。小さいウィンドウでも一覧が2行ぶんは見えるよう、カードは高さの6割まで */
  .history-detail {
    flex: 0 1 auto;
    max-height: 60%;
    min-height: 0;
    overflow-y: auto;
    padding: 0 6px;
  }
  .history-list {
    flex: 1;
    min-height: 76px;
    overflow-y: auto;
  }
  .history-list::-webkit-scrollbar,
  .history-detail::-webkit-scrollbar { width: 3px; }
  .history-list::-webkit-scrollbar-thumb,
  .history-detail::-webkit-scrollbar-thumb { background: var(--border); border-radius: 2px; }
  .history-time {
    color: var(--text-dim);
    font-size: 0.82em;
    font-variant-numeric: tabular-nums;
  }
  .history-info {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .history-info strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 0.9em;
  }
  .history-info span {
    color: var(--text-dim);
    font-size: 0.76em;
  }
  .history-value {
    font-size: 0.86em;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }

  .settings-backdrop {
    position: absolute;
    inset: 69px 0 0;
    z-index: 20;
    border: none;
    background: rgb(var(--bg) / 0.76);
    cursor: default;
  }
  .settings-panel {
    position: absolute;
    inset: 69px 0 0 auto;
    z-index: 21;
    width: min(270px, 92vw);
    background: rgb(var(--surface));
    border-left: 1px solid var(--border);
    box-shadow: -6px 0 24px rgba(0 0 0 / 0.35);
    overflow-y: auto;
  }
  .settings-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
    color: var(--accent);
    font-weight: 800;
    font-size: 0.84em;
  }
  .rank-mode {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 6px;
    padding: 9px 12px 12px;
    border-bottom: 1px solid var(--border);
  }
  .rank-mode button {
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: rgb(var(--bg) / 0.34);
    color: var(--text-dim);
    cursor: pointer;
    font: inherit;
    font-size: 0.78em;
    font-weight: 800;
    padding: 5px 6px;
  }
  .rank-mode button:hover,
  .rank-mode button.active {
    border-color: var(--accent);
    color: var(--accent);
    background: rgb(var(--bg) / 0.58);
  }
  .mini-btn {
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--input-bg);
    color: var(--text);
    cursor: pointer;
    font: inherit;
    font-size: 0.78em;
    padding: 4px 9px;
  }
  .mini-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .color-list {
    display: flex;
    flex-direction: column;
  }
  .color-row {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) 36px;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid rgb(var(--bg) / 0.45);
    color: var(--text);
    font-size: 0.84em;
  }
  .color-row span:nth-child(2) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .swatch {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 1px solid rgb(var(--bg) / 0.55);
  }
  .color-row input {
    width: 34px;
    height: 24px;
    border: none;
    border-radius: 5px;
    background: transparent;
    cursor: pointer;
    padding: 0;
  }

  .scope-toggle {
    display: flex;
    gap: 4px;
    margin-top: 5px;
  }
  .scope-toggle button {
    flex: 1;
    min-width: 0;
    padding: 3px 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: rgb(var(--bg) / 0.34);
    color: var(--text-dim);
    cursor: pointer;
    font: inherit;
    font-size: 0.76em;
    font-weight: 800;
  }
  .scope-toggle button:hover,
  .scope-toggle button.on {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }

  .boss-banner {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 10px;
    background: color-mix(in srgb, var(--accent) 14%, rgb(var(--surface) / 0.82));
    border-bottom: 1px solid var(--border);
    font-size: 0.8em;
    min-width: 0;
  }
  .boss-banner-label {
    font-weight: 800;
    font-size: 0.72em;
    letter-spacing: 0.08em;
    color: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 4px;
    padding: 0 4px;
    flex-shrink: 0;
  }
  .boss-banner-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    font-weight: 700;
  }

  .toggle-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 12px 12px;
    border-bottom: 1px solid var(--border);
    color: var(--text);
    font-size: 0.82em;
    cursor: pointer;
  }
  .toggle-row input {
    flex-shrink: 0;
    width: 15px;
    height: 15px;
    accent-color: var(--accent);
    cursor: pointer;
  }

  .enemy-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid rgb(var(--bg) / 0.4);
  }
  .enemy-row.boss {
    background: color-mix(in srgb, #f59e0b 10%, transparent);
  }
  .enemy-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .enemy-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    font-size: 0.92em;
    font-weight: 700;
  }
  .enemy-sub {
    color: var(--text-dim);
    font-size: 0.74em;
    font-variant-numeric: tabular-nums;
  }
  .enemy-toggle {
    flex-shrink: 0;
    min-width: 48px;
    padding: 3px 10px;
    border-radius: 20px;
    border: 1.5px solid var(--border);
    background: none;
    color: var(--text-dim);
    text-align: center;
    font-size: 0.78em;
    font-weight: 800;
  }
  .enemy-toggle.boss {
    border-color: #f59e0b;
    color: #f59e0b;
    background: color-mix(in srgb, #f59e0b 14%, transparent);
  }
</style>
