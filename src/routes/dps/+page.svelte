<script lang="ts">
  import {
    classColor,
    classIconUrl,
    DEFAULT_DPS_SETTINGS,
    DPS_SETTINGS_KEY,
    fmtCompact,
    fmtSeconds,
    isRealClass,
    isRealName,
    loadBossList,
    loadDpsSettings,
    loadIdCache,
    normalizeBossList,
    saveBossList,
    saveDpsSettings,
    saveIdCache,
    type DpsBossEntry,
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
  import { onMount } from 'svelte';

  type PodiumEntry = DpsPlayerRow & {
    rank: number;
    color: string;
    iconUrl: string;
    placeholder?: boolean;
  };
  type PodiumGroup = {
    metric: DpsMetricType;
    label: string;
    rateLabel: string;
    entries: PodiumEntry[];
  };
  type ActiveTab = DpsMetricType | 'enemy';
  type EnemyGroup = { id: number; name: string; eliteStatus: number; damageTaken: number; count: number };
  const ENEMY_COLOR = '#f59e0b';
  const PODIUM_DURATION_MS = 15000;
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
  let bossPanelOpen = $state(false);
  let rankBursts = $state<Record<string, number>>({});
  let podiumGroups = $state<PodiumGroup[]>([]);
  let podiumToken = 0;
  let prevBossEngaged = false;
  let bossList = $state<DpsBossEntry[]>([]);
  let idCache = $state<Record<string, DpsIdentity>>(loadIdCache());
  let newBossId = $state('');
  let newBossName = $state('');
  const effectiveScope = $derived<DpsScope>(activeMetric === 'dps' ? scope : 'all');
  const currentMetric = $derived(
    activeMetric === 'enemy' ? emptyMetric() : resolveMetric(dpsMeter, activeMetric, effectiveScope)
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

  function updateAutoBossScope(value: boolean) {
    settings = { ...settings, autoBossScope: value };
    saveDpsSettings(settings);
  }

  function updateRankAnimation(rankAnimation: DpsSettings['rankAnimation']) {
    settings = { ...settings, rankAnimation };
    if (rankAnimation !== 'flashy') {
      rankBursts = {};
    }
    if (rankAnimation === 'off') {
      podiumGroups = [];
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

  function resolveMetric(payload: DpsMeterPayload, metric: DpsMetricType, sc: DpsScope): DpsMetricPayload {
    if (metric === 'dps' && sc === 'boss') {
      return payload.metrics?.dpsBossOnly ?? emptyMetric();
    }
    return metricWindow(payload, metric);
  }

  function hasAnyMetricData(payload: DpsMeterPayload) {
    return METRIC_TABS.some((tab) => metricWindow(payload, tab.id).players.length > 0);
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
      if (isRealClass(row.className) && cur.className !== row.className) { cur.className = row.className; updated = true; }
      if (updated) { next[key] = cur; changed = true; }
    }
    if (changed) {
      idCache = next;
      saveIdCache(idCache);
    }
  }

  function effName(uid: number, name: string) {
    return isRealName(name) ? name : (idCache[String(uid)]?.name ?? name);
  }

  function effClass(uid: number, className: string) {
    return isRealClass(className) ? className : (idCache[String(uid)]?.className ?? className);
  }

  function setActiveMetric(metric: ActiveTab) {
    activeMetric = metric;
    rankBursts = {};
    settingsOpen = false;
    bossPanelOpen = false;
  }

  function setScope(next: DpsScope) {
    scope = next;
    rankBursts = {};
  }

  function persistBossList() {
    bossList = normalizeBossList(bossList);
    saveBossList(bossList);
    void invoke('set_boss_list', { entries: bossList }).catch(() => {});
  }

  function addBoss() {
    const id = Number.parseInt(newBossId, 10);
    if (!Number.isInteger(id) || id < 0) return;
    const name = newBossName.trim() || `Boss ${id}`;
    bossList = normalizeBossList([...bossList.filter((b) => b.id !== id), { id, name }]);
    newBossId = '';
    newBossName = '';
    persistBossList();
  }

  function removeBoss(id: number) {
    bossList = bossList.filter((b) => b.id !== id);
    persistBossList();
  }

  function groupEnemies(list: DpsMonsterInfo[]): EnemyGroup[] {
    const map = new Map<number, EnemyGroup>();
    for (const e of list) {
      const g = map.get(e.id);
      if (g) {
        g.count += 1;
        g.damageTaken += e.damageTaken;
        if (e.eliteStatus > g.eliteStatus) g.eliteStatus = e.eliteStatus;
        if (e.id !== 0 && e.name && e.name !== g.name) g.name = e.name;
      } else {
        map.set(e.id, {
          id: e.id,
          name: e.name,
          eliteStatus: e.eliteStatus,
          damageTaken: e.damageTaken,
          count: 1,
        });
      }
    }
    return [...map.values()].sort((a, b) => b.damageTaken - a.damageTaken);
  }

  function isEnemyBoss(id: number) {
    return bossList.some((b) => b.id === id);
  }

  function bossEntryName(id: number) {
    return bossList.find((b) => b.id === id)?.name ?? '';
  }

  function renameBoss(id: number, name: string) {
    const trimmed = name.trim();
    bossList = bossList.map((b) => (b.id === id ? { ...b, name: trimmed || `Boss ${id}` } : b));
    persistBossList();
  }

  function toggleEnemyBoss(monster: { id: number; name: string }) {
    if (monster.id === 0) return;
    if (isEnemyBoss(monster.id)) {
      bossList = bossList.filter((b) => b.id !== monster.id);
    } else {
      const name = monster.name && !monster.name.startsWith('Monster ') ? monster.name : `Boss ${monster.id}`;
      bossList = normalizeBossList([...bossList.filter((b) => b.id !== monster.id), { id: monster.id, name }]);
    }
    persistBossList();
  }

  async function resetBosses() {
    try {
      const defaults = await invoke<DpsBossEntry[]>('reset_boss_list');
      bossList = normalizeBossList(defaults);
      saveBossList(bossList);
    } catch {}
  }

  async function initBossList() {
    const saved = loadBossList();
    if (saved.length > 0) {
      bossList = saved;
      await invoke('set_boss_list', { entries: bossList }).catch(() => {});
      return;
    }
    try {
      const defaults = await invoke<DpsBossEntry[]>('get_boss_list');
      bossList = normalizeBossList(defaults);
      saveBossList(bossList);
    } catch {
      bossList = [];
    }
  }

  function placeholderEntry(rank: number): PodiumEntry {
    return {
      uid: -rank,
      name: '該当者なし',
      className: 'Unknown Class',
      classSpecName: '',
      abilityScore: -1,
      totalDamage: 0,
      dps: 0,
      damagePct: 0,
      critRate: 0,
      hits: 0,
      tankScore: 0,
      avgTaken: 0,
      rank,
      color: '#64748b',
      iconUrl: '',
      placeholder: true,
    };
  }

  function markResetPodium(next: DpsMeterPayload) {
    if (settings.rankAnimation === 'off') return;
    if (!hasAnyMetricData(dpsMeter) || hasAnyMetricData(next)) return;

    const token = Date.now();
    podiumToken = token;
    podiumGroups = METRIC_TABS
      .map((tab) => {
        const entries: PodiumEntry[] = orderPlayers(metricWindow(dpsMeter, tab.id).players, tab.id).slice(0, 3).map((row, index) => ({
          ...row,
          name: effName(row.uid, row.name),
          rank: index + 1,
          color: rowColor(effClass(row.uid, row.className)),
          iconUrl: classIconUrl(effClass(row.uid, row.className)),
        }));
        const hadData = entries.length > 0;
        while (hadData && entries.length < 3) {
          entries.push(placeholderEntry(entries.length + 1));
        }
        return {
          metric: tab.id,
          label: tab.label,
          rateLabel: tab.rateLabel,
          entries,
        };
      })
      .filter((group) => group.entries.length > 0);

    window.setTimeout(() => {
      if (podiumToken !== token) return;
      podiumGroups = [];
    }, PODIUM_DURATION_MS);
  }

  function markRankUps(next: DpsMeterPayload) {
    if (settings.rankAnimation !== 'flashy') return;
    if (activeMetric === 'enemy') return;

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

  function rankBurstKey(uid: number) {
    return `${activeMetric}:${uid}`;
  }

  function podiumMedal(rank: number) {
    if (rank === 1) return '\u{1F3C5}';
    if (rank === 2) return '\u{1F948}';
    return '\u{1F949}';
  }

  async function closeWindow() {
    await emit('dps-window-closed');
    await invoke('close_dps_window');
  }

  function beginDrag(event: PointerEvent) {
    if ((event.target as HTMLElement).closest('button, input')) return;
    void getCurrentWebviewWindow().startDragging();
  }

  onMount(async () => {
    applyChatTheme();
    await emit('dps-window-opened');
    await initBossList();
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
      markResetPodium(payload);
      markRankUps(payload);
      dpsMeter = payload;
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

    return () => {
      unlistenDps();
      unlistenTheme();
      unlistenClose();
      window.removeEventListener('storage', onStorage);
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
      <button
        class="icon-btn"
        class:active={bossPanelOpen}
        title="ボス設定"
        aria-label="ボス設定"
        onclick={() => { bossPanelOpen = !bossPanelOpen; if (bossPanelOpen) settingsOpen = false; }}
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"></path>
          <path d="M6.5 2H20v20H6.5a2.5 2.5 0 0 1 0-5H20"></path>
          <path d="M10 7h5"></path>
          <path d="M10 11h6"></path>
        </svg>
      </button>
      <button
        class="icon-btn"
        class:active={settingsOpen}
        title="DPS設定"
        aria-label="DPS設定"
        onclick={() => { settingsOpen = !settingsOpen; if (settingsOpen) bossPanelOpen = false; }}
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
        <div class="enemy-row" class:boss={isEnemyBoss(enemy.id)}>
          <div class="enemy-info">
            {#if isEnemyBoss(enemy.id)}
              <input
                class="enemy-name-input"
                value={bossEntryName(enemy.id)}
                placeholder="ボス名"
                onchange={(event) => renameBoss(enemy.id, (event.currentTarget as HTMLInputElement).value)}
              />
            {:else}
              <span class="enemy-name">{enemy.name}</span>
            {/if}
            <span class="enemy-sub">
              {enemy.id === 0 ? 'ID不明' : `ID ${enemy.id}`}{#if enemy.count > 1} ({enemy.count}){/if} ・ {fmtCompact(enemy.damageTaken)} DMG{#if enemy.eliteStatus > 0} ・ ELITE{/if}
            </span>
          </div>
          <button
            class="enemy-toggle"
            class:boss={isEnemyBoss(enemy.id)}
            disabled={enemy.id === 0}
            onclick={() => toggleEnemyBoss(enemy)}
          >{enemy.id === 0 ? 'ID不明' : isEnemyBoss(enemy.id) ? 'ボス' : '雑魚'}</button>
        </div>
      {:else}
        <div class="dps-empty">交戦中の敵なし</div>
      {/each}
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
      >
        <div class="dps-class-icon" aria-hidden="true">
          {#if classIconUrl(effClass(row.uid, row.className))}
            <img src={classIconUrl(effClass(row.uid, row.className))} alt="" />
          {:else}
            <span>?</span>
          {/if}
        </div>
        <div class="dps-player">
          <span class="dps-name">{effName(row.uid, row.name)}</span>
        </div>
        <div class="dps-values">
          {#if activeMetric === 'tank'}
            <strong>{fmtTankScore(tankScoreOf(row))} TANK</strong>
            <span title={`平均 ${fmtCompact(avgTakenOf(row))} / Hit`}>
              {fmtCompact(row.hits)}回 / {fmtCompact(row.totalDamage)}被ダメ
            </span>
          {:else}
            <strong>{fmtCompact(row.dps)}</strong>
            <span>{fmtCompact(row.totalDamage)} {currentTab.totalLabel} / {row.damagePct.toFixed(1)}%</span>
          {/if}
        </div>
      </div>
    {:else}
      <div class="dps-empty">{currentTab.empty}</div>
    {/each}
  </main>
  {/if}

  {#if podiumGroups.length > 0}
    <div
      class="podium-showcase"
      class:single-group={podiumGroups.length === 1}
      aria-live="polite"
    >
      <div class="podium-burst" aria-hidden="true"></div>
      {#each podiumGroups as group, groupIndex (group.metric)}
        <div class="podium-section" style:--podium-group={groupIndex}>
          <div class="podium-title">{group.label}</div>
          <div class="podium-cards" class:solo={group.entries.length === 1} class:duo={group.entries.length === 2}>
            {#each group.entries as entry (entry.uid)}
              <div
                class="podium-card rank-{entry.rank}"
                class:placeholder={entry.placeholder}
                style:--class-color={entry.color}
                style:--podium-delay={`${(groupIndex * 3 + (3 - entry.rank)) * 600}ms`}
              >
                <div class="podium-medal" aria-hidden="true">{podiumMedal(entry.rank)}</div>
                <div class="podium-icon" aria-hidden="true">
                  {#if entry.placeholder}
                    <span>–</span>
                  {:else if entry.iconUrl}
                    <img src={entry.iconUrl} alt="" />
                  {:else}
                    <span>?</span>
                  {/if}
                </div>
                <div class="podium-info">
                  <span>{entry.rank}位</span>
                  <strong>{entry.name}</strong>
                  <em>{entry.placeholder ? '—' : group.metric === 'tank' ? `${fmtTankScore(tankScoreOf(entry))} TANK` : `${fmtCompact(entry.dps)} ${group.rateLabel}`}</em>
                </div>
              </div>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  {/if}
  {#if bossPanelOpen}
    <button
      class="settings-backdrop"
      aria-label="ボス設定を閉じる"
      onclick={() => { bossPanelOpen = false; }}
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
        <span>ボスリスト ({bossList.length})</span>
        <button class="mini-btn" onclick={() => { void resetBosses(); }}>デフォルトに戻す</button>
      </div>
      <div class="boss-add">
        <input
          class="boss-input id"
          type="number"
          min="0"
          placeholder="ID"
          bind:value={newBossId}
        />
        <input
          class="boss-input name"
          type="text"
          placeholder="名前 (任意)"
          bind:value={newBossName}
        />
        <button class="mini-btn" onclick={addBoss}>追加</button>
      </div>
      <div class="boss-list">
        {#each bossList as boss (boss.id)}
          <div class="boss-row">
            <span class="boss-id">{boss.id}</span>
            <span class="boss-name">{boss.name}</span>
            <button
              class="boss-remove"
              title="削除"
              aria-label="削除"
              onclick={() => removeBoss(boss.id)}
            >×</button>
          </div>
        {:else}
          <div class="boss-empty">登録なし</div>
        {/each}
      </div>
    </aside>
  {/if}
  {#if settingsOpen}
    <button
      class="settings-backdrop"
      aria-label="DPS設定を閉じる"
      onclick={() => { settingsOpen = false; }}
    ></button>
    <aside class="settings-panel">
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
    grid-template-columns: 24px minmax(0, 1fr) minmax(88px, auto);
    align-items: center;
    gap: 6px;
    min-height: 27px;
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
  .dps-player,
  .dps-values {
    position: relative;
    z-index: 1;
  }
  .dps-class-icon {
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

  .podium-showcase {
    position: absolute;
    inset: 98px 6px auto;
    z-index: 18;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 7px;
    padding: 6px;
    pointer-events: none;
    animation: podium-layer 15000ms ease both;
  }
  .podium-showcase.single-group {
    inset-inline: 24px;
  }
  .podium-section {
    position: relative;
    min-width: 0;
    display: grid;
    gap: 3px;
  }
  .podium-title {
    justify-self: start;
    padding: 2px 8px;
    border: 1px solid color-mix(in srgb, var(--accent) 42%, var(--border));
    border-radius: 999px;
    background: rgb(var(--surface) / 0.9);
    color: var(--accent);
    font-size: 0.68em;
    font-weight: 900;
    letter-spacing: 0.06em;
  }
  .podium-cards {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 4px;
  }
  .podium-cards.duo .rank-2,
  .podium-cards.solo .rank-1 {
    grid-column: auto;
  }
  .podium-burst {
    position: absolute;
    inset: 2px 0 auto;
    height: 52px;
    border-radius: 8px;
    background:
      linear-gradient(90deg, transparent, color-mix(in srgb, var(--accent) 34%, transparent), transparent);
    filter: blur(5px);
    opacity: 0;
    animation: podium-burst 850ms ease-out both;
  }
  .podium-card {
    position: relative;
    min-width: 0;
    display: grid;
    grid-template-columns: 26px minmax(0, 1fr);
    align-items: center;
    gap: 6px;
    min-height: 32px;
    padding: 4px 7px 4px 4px;
    border: 1px solid color-mix(in srgb, var(--class-color) 74%, white);
    border-radius: 7px;
    background:
      linear-gradient(180deg,
        color-mix(in srgb, var(--class-color) 30%, rgb(var(--surface))) 0%,
        color-mix(in srgb, var(--class-color) 16%, rgb(var(--bg))) 100%);
    box-shadow:
      0 0 0 1px color-mix(in srgb, var(--class-color) 26%, transparent),
      0 10px 24px rgb(0 0 0 / 0.46),
      inset 0 1px 0 rgb(255 255 255 / 0.18);
    overflow: hidden;
    animation: podium-card 330ms linear both;
    animation-delay: var(--podium-delay);
  }
  .podium-card.placeholder {
    border-color: color-mix(in srgb, var(--class-color) 40%, var(--border));
    opacity: 0.78;
  }
  .podium-card::before {
    content: '';
    position: absolute;
    inset: 0;
    background:
      linear-gradient(115deg, transparent 0 28%, rgb(255 255 255 / 0.22) 42%, transparent 58% 100%);
    translate: -110% 0;
    animation: podium-sheen 1100ms ease-out both;
    animation-delay: calc(var(--podium-delay) + 300ms);
    pointer-events: none;
  }
  .podium-card.rank-1 {
    min-height: 38px;
    grid-template-columns: 32px minmax(0, 1fr);
    transform-origin: center right;
    border-color: color-mix(in srgb, var(--class-color) 86%, white);
  }
  .podium-card.rank-1::after {
    content: '';
    position: absolute;
    inset: -35% -18%;
    background:
      radial-gradient(circle at 18% 30%, rgb(255 255 255 / 0.9) 0 2px, transparent 3px),
      radial-gradient(circle at 76% 24%, rgb(255 255 255 / 0.85) 0 1px, transparent 3px),
      linear-gradient(115deg, transparent 0 38%, rgb(255 255 255 / 0.72) 48%, transparent 58% 100%);
    opacity: 0;
    rotate: -7deg;
    translate: -70% 0;
    filter: blur(0.2px) drop-shadow(0 0 9px color-mix(in srgb, var(--class-color) 70%, white));
    animation: podium-kira 2600ms ease-out infinite;
    animation-delay: calc(var(--podium-delay) + 330ms);
    pointer-events: none;
  }
  .podium-medal {
    position: absolute;
    right: 5px;
    top: 2px;
    z-index: 4;
    font-size: 1.08em;
    filter: drop-shadow(0 2px 5px rgb(0 0 0 / 0.45));
  }
  .podium-icon {
    position: relative;
    z-index: 3;
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 7px;
    background: rgb(var(--bg) / 0.46);
    box-shadow:
      inset 0 0 0 1px color-mix(in srgb, var(--class-color) 66%, transparent),
      0 4px 13px rgb(0 0 0 / 0.32);
    overflow: hidden;
  }
  .rank-1 .podium-icon {
    width: 30px;
    height: 30px;
  }
  .podium-icon img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .podium-icon span {
    color: var(--text-dim);
    font-size: 0.76em;
    font-weight: 900;
  }
  .podium-info {
    position: relative;
    z-index: 3;
    min-width: 0;
    display: grid;
    gap: 1px;
  }
  .podium-info span {
    color: color-mix(in srgb, var(--class-color) 76%, white);
    font-size: 0.62em;
    font-weight: 900;
  }
  .podium-info strong {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    font-size: 0.86em;
    font-weight: 900;
  }
  .rank-1 .podium-info strong {
    font-size: 1em;
  }
  .podium-info em {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-dim);
    font-style: normal;
    font-size: 0.65em;
    font-weight: 800;
    font-variant-numeric: tabular-nums;
  }
  @keyframes podium-layer {
    0% { opacity: 0; translate: 0 -12px; }
    10% { opacity: 1; translate: 0 0; }
    82% { opacity: 1; translate: 0 0; }
    100% { opacity: 0; translate: 0 -8px; }
  }
  @keyframes podium-card {
    0% {
      opacity: 0;
      translate: 95px 0;
      scale: 0.98;
      filter: brightness(1.3);
    }
    14% {
      opacity: 1;
      translate: 84px 0;
      scale: 0.98;
    }
    52% {
      translate: 30px 0;
      filter: brightness(1.18);
    }
    74% {
      translate: -18px 0;
      scale: 1.03;
      filter: brightness(1.05);
    }
    88% {
      translate: 7px 0;
      scale: 1.01;
    }
    100% {
      opacity: 1;
      translate: 0 0;
      scale: 1;
      filter: brightness(1);
    }
  }
  @keyframes podium-burst {
    0% { opacity: 0; scale: 0.7 0.6; }
    18% { opacity: 0.9; scale: 1 1; }
    100% { opacity: 0; scale: 1.08 1; }
  }
  @keyframes podium-sheen {
    0% { translate: -110% 0; }
    64% { translate: 110% 0; }
    100% { translate: 110% 0; }
  }
  @keyframes podium-kira {
    0% {
      opacity: 0;
      translate: -70% 0;
      scale: 0.92;
    }
    10% {
      opacity: 0.98;
    }
    26% {
      opacity: 0.68;
      translate: 72% 0;
      scale: 1.04;
    }
    38% {
      opacity: 0.22;
      translate: 86% 0;
    }
    50% {
      opacity: 0;
      translate: 95% 0;
    }
    100% {
      opacity: 0;
      translate: -70% 0;
    }
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

  .boss-add {
    display: grid;
    grid-template-columns: 72px minmax(0, 1fr) auto;
    gap: 6px;
    padding: 9px 12px;
    border-bottom: 1px solid rgb(var(--bg) / 0.45);
  }
  .boss-input {
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--input-bg);
    color: var(--text);
    font: inherit;
    font-size: 0.8em;
    padding: 4px 7px;
  }
  .boss-input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .boss-list {
    display: flex;
    flex-direction: column;
  }
  .boss-row {
    display: grid;
    grid-template-columns: 64px minmax(0, 1fr) 22px;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid rgb(var(--bg) / 0.45);
    font-size: 0.82em;
    color: var(--text);
  }
  .boss-id {
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .boss-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .boss-remove {
    width: 20px;
    height: 20px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: none;
    color: var(--text-dim);
    cursor: pointer;
    font: inherit;
    line-height: 1;
    padding: 0;
  }
  .boss-remove:hover {
    border-color: #f87171;
    color: #f87171;
  }
  .boss-empty {
    padding: 10px 12px;
    color: var(--text-dim);
    font-size: 0.8em;
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
  .enemy-name-input {
    width: 100%;
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--input-bg);
    color: var(--text);
    font: inherit;
    font-size: 0.9em;
    font-weight: 700;
    padding: 2px 6px;
  }
  .enemy-name-input:focus {
    outline: none;
    border-color: #f59e0b;
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
    cursor: pointer;
    font: inherit;
    font-size: 0.78em;
    font-weight: 800;
  }
  .enemy-toggle:hover {
    border-color: var(--text-dim);
  }
  .enemy-toggle.boss {
    border-color: #f59e0b;
    color: #f59e0b;
    background: color-mix(in srgb, #f59e0b 14%, transparent);
  }
</style>



