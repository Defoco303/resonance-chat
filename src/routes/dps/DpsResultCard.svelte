<script lang="ts">
  import {
    classIconUrl,
    classSpecLabel,
    ENCOUNTER_END_LABELS,
    fmtCompact,
    fmtSeconds,
    type DpsEncounterRecord,
    type DpsPlayerRow,
  } from '$lib/dps';

  type HistoryNav = {
    index: number;
    total: number;
    onPrev: () => void;
    onNext: () => void;
  };

  let {
    record,
    colorOf,
    onClose,
    nav,
    showRanking = true,
    onSelectPlayer,
  }: {
    record: DpsEncounterRecord;
    colorOf: (className: string) => string;
    onClose?: () => void;
    // false なら順位の一覧を出さない（撃破直後に自動で出るカード。元の画面で見られるため）
    showRanking?: boolean;
    // 順位の行を押したとき（プレイヤー詳細を開く）
    onSelectPlayer?: (uid: number) => void;
    // 履歴タブで使うときの前後送り（onPrev = 新しい戦闘、onNext = 古い戦闘）
    nav?: HistoryNav;
  } = $props();

  const ranked = $derived(record.players.map((row, index) => ({ row, rank: index + 1 })));
  const localEntry = $derived(ranked.find((entry) => entry.row.uid === record.localPlayerUid));
  const topPct = $derived(ranked[0]?.row.damagePct ?? 0);

  function endedAtLabel(ms: number) {
    const date = new Date(ms);
    return `${date.getHours().toString().padStart(2, '0')}:${date.getMinutes().toString().padStart(2, '0')}`;
  }

  function barWidth(row: DpsPlayerRow) {
    return topPct > 0 ? Math.max(3, (row.damagePct / topPct) * 100) : 0;
  }
</script>

<section class="result-card" aria-label="戦闘リザルト">
  <header class="result-head">
    <div class="result-title">
      <strong>{record.bossName || '戦闘'}</strong>
      <span>
        {#if record.reason === 'bossDefeated'}
          撃破時刻 {endedAtLabel(record.endedAtMs)}
        {:else}
          {ENCOUNTER_END_LABELS[record.reason]} ・ 終了時刻 {endedAtLabel(record.endedAtMs)}
        {/if}
      </span>
    </div>
    {#if nav}
      <div class="result-nav">
        <button
          class="result-close"
          aria-label="新しい戦闘"
          title="新しい戦闘（↑ / ←）"
          disabled={nav.index <= 0}
          onclick={nav.onPrev}
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m15 18-6-6 6-6"></path>
          </svg>
        </button>
        <span>{nav.index + 1}/{nav.total}</span>
        <button
          class="result-close"
          aria-label="古い戦闘"
          title="古い戦闘（↓ / →）"
          disabled={nav.index >= nav.total - 1}
          onclick={nav.onNext}
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m9 18 6-6-6-6"></path>
          </svg>
        </button>
      </div>
    {/if}
    {#if onClose}
      <button class="result-close" aria-label="リザルトを閉じる" onclick={onClose}>
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M18 6 6 18"></path>
          <path d="m6 6 12 12"></path>
        </svg>
      </button>
    {/if}
  </header>

  <div class="result-stats">
    <div>
      <span>戦闘時間</span>
      <strong>{fmtSeconds(record.elapsedMs)}</strong>
    </div>
    <div>
      <span title={record.bossOnly ? 'ボス本体へのダメージだけで集計（呼び出された雑魚は含めない）' : undefined}>
        全体DPS
      </span>
      <strong>{fmtCompact(record.totalDps)}</strong>
    </div>
    <div>
      <span>あなた</span>
      <strong>
        {#if localEntry}
          {localEntry.rank}位 {fmtCompact(localEntry.row.dps)}
        {:else}
          —
        {/if}
      </strong>
    </div>
  </div>

  {#if showRanking}
  <ol class="result-rows">
    {#each ranked as entry (entry.row.uid)}
      <li class:local={entry.row.uid === record.localPlayerUid} style:--class-color={colorOf(entry.row.className)}>
        <button
          class="row-btn"
          disabled={!onSelectPlayer}
          title={onSelectPlayer ? '詳細・自分との比較を見る' : undefined}
          onclick={() => onSelectPlayer?.(entry.row.uid)}
        >
        <span class="result-rank">{entry.rank}</span>
        <span class="result-icon" aria-hidden="true">
          {#if classIconUrl(entry.row.className)}
            <img src={classIconUrl(entry.row.className)} alt="" />
          {/if}
        </span>
        <span class="result-name">
          {entry.row.name}
          {#if classSpecLabel(entry.row.classSpecName)}
            <em>{classSpecLabel(entry.row.classSpecName)}</em>
          {/if}
        </span>
        <span class="result-bar" aria-hidden="true">
          <span style:width={`${barWidth(entry.row)}%`}></span>
        </span>
        <span class="result-value">
          <strong>{fmtCompact(entry.row.dps)}</strong>
          <span>{entry.row.damagePct.toFixed(1)}%</span>
        </span>
        </button>
      </li>
    {:else}
      <li class="result-empty">ダメージの記録なし</li>
    {/each}
  </ol>
  {/if}

  {#if record.topHeal || record.topTank}
    <footer class="result-foot">
      {#if record.topHeal}
        <span>回復1位 <strong>{record.topHeal.name}</strong> {fmtCompact(record.topHeal.dps)} HPS</span>
      {/if}
      {#if record.topTank}
        <span>タンク1位 <strong>{record.topTank.name}</strong></span>
      {/if}
    </footer>
  {/if}
</section>

<style>
  .result-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    background: rgb(var(--surface));
    border: 1px solid var(--border);
    border-radius: 10px;
    color: var(--text);
    font-size: 0.86em;
  }
  .result-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 8px;
  }
  .result-title {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .result-title strong {
    font-size: 1.12em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .result-title span {
    color: var(--text-dim);
    font-size: 0.86em;
  }
  .result-close {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .result-close:hover {
    color: var(--text);
  }
  .result-close:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .result-nav {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--text-dim);
    font-size: 0.86em;
    font-variant-numeric: tabular-nums;
  }
  .result-stats {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 6px;
  }
  .result-stats div {
    display: flex;
    flex-direction: column;
    padding: 6px 8px;
    border-radius: 6px;
    background: rgb(var(--bg) / 0.6);
  }
  .result-stats span {
    color: var(--text-dim);
    font-size: 0.82em;
  }
  .result-stats strong {
    font-size: 1.05em;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .result-rows {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .result-rows li {
    border-top: 1px solid var(--border);
  }
  .row-btn {
    display: grid;
    grid-template-columns: 18px 20px minmax(0, 1fr) 56px auto;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 4px 2px;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .row-btn:disabled {
    cursor: default;
  }
  .row-btn:not(:disabled):hover {
    background: rgb(var(--bg) / 0.5);
  }
  .result-rows li:first-child {
    border-top: none;
  }
  .result-rows li.local {
    background: color-mix(in srgb, var(--class-color) 12%, transparent);
    border-radius: 4px;
  }
  .result-rank {
    color: var(--text-dim);
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .result-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--class-color);
  }
  .result-icon img {
    width: 14px;
    height: 14px;
    object-fit: contain;
  }
  .result-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .result-name em {
    margin-left: 4px;
    color: var(--text-dim);
    font-size: 0.86em;
    font-style: normal;
  }
  .result-bar {
    height: 4px;
    border-radius: 2px;
    background: rgb(var(--bg) / 0.6);
    overflow: hidden;
  }
  .result-bar span {
    display: block;
    height: 100%;
    background: var(--class-color);
  }
  .result-value {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    font-variant-numeric: tabular-nums;
  }
  .result-value span {
    color: var(--text-dim);
    font-size: 0.82em;
  }
  .result-empty {
    display: block !important;
    color: var(--text-dim);
    text-align: center;
  }
  .result-foot {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    color: var(--text-dim);
    font-size: 0.86em;
  }
  .result-foot strong {
    color: var(--text);
  }
</style>
