<script lang="ts">
  import {
    classIconUrl,
    classSpecLabel,
    detailStats,
    dpsGapFactors,
    fmtCompact,
    fmtSeconds,
    loadSkillNames,
    skillLabel,
    type DpsDetailStats,
    type DpsImagineDetail,
    type DpsPlayerDetail,
    type SkillNameTable,
  } from '$lib/dps';
  import { imagineBaseId, imagineName } from '$lib/imagines';
  import { onMount } from 'svelte';

  let {
    detail,
    self,
    bossOnly,
    colorOf,
    onBack,
  }: {
    detail: DpsPlayerDetail;
    // 自分の内訳（比較用）。自分がダメージを出していなければ無い
    self?: DpsPlayerDetail;
    bossOnly: boolean;
    colorOf: (className: string) => string;
    onBack: () => void;
  } = $props();

  let comparing = $state(false);
  let skillNames = $state<SkillNameTable | null>(null);

  onMount(() => {
    loadSkillNames().then((table) => {
      skillNames = table;
    });
  });

  function label(skillId: number) {
    return skillLabel(skillId, skillNames);
  }

  function imagineLabel(imagine: DpsImagineDetail) {
    return imagineName(imagine.skillId, skillNames) ?? `イマジン ${imagine.skillId}`;
  }

  function imagineValue(imagine: DpsImagineDetail | undefined, total: number) {
    if (!imagine || imagine.totalDamage <= 0) return 'ダメージなし';
    return `${fmtCompact(imagine.totalDamage)}（${pct(total > 0 ? imagine.totalDamage / total : 0)}）`;
  }

  // 比較の表の1マス: 使っていない → —、装備中でダメージなし → 装備、ダメージあり → 割合
  function imagineShare(imagine: DpsImagineDetail | undefined, total: number) {
    if (!imagine) return '—';
    if (imagine.totalDamage <= 0) return '装備';
    return pct(total > 0 ? imagine.totalDamage / total : 0);
  }

  // 比較用: 相手と自分のイマジンを1つの一覧に
  const imagineCompareRows = $derived.by(() => {
    if (!self) return [];
    const targetImagines = detail.imagines ?? [];
    const mineImagines = self.imagines ?? [];
    const ids = [...new Set([...targetImagines, ...mineImagines].map((imagine) => imagine.skillId))];
    return ids.map((skillId) => ({
      skillId,
      target: targetImagines.find((imagine) => imagine.skillId === skillId),
      mine: mineImagines.find((imagine) => imagine.skillId === skillId),
    }));
  });

  const isSelf = $derived(Boolean(self) && self?.uid === detail.uid);
  const stats = $derived(detailStats(detail));
  const selfStats = $derived(self ? detailStats(self) : null);
  const factors = $derived(selfStats ? dpsGapFactors(stats, selfStats) : []);
  const sameClass = $derived(Boolean(self) && self?.className === detail.className);

  // 比較する項目: [ラベル, 値の取り出し, 表示]
  const COMPARE_ROWS: Array<[string, (s: DpsDetailStats) => number, (v: number) => string]> = [
    ['DPS', (s) => s.dps, fmtCompact],
    ['実働DPS', (s) => s.tdps, fmtCompact],
    ['稼働率', (s) => s.uptime, pct],
    ['手数（/秒）', (s) => s.hitsPerSec, (v) => v.toFixed(1)],
    ['1撃の平均', (s) => s.avgHit, fmtCompact],
    ['クリ率', (s) => s.critRate, pct],
    ['クリダメ割合', (s) => s.critShare, pct],
    ['幸運率', (s) => s.luckyRate, pct],
    ['幸運ダメ割合', (s) => s.luckyShare, pct],
  ];

  const skillCompareRows = $derived.by(() => {
    if (!self) return [];
    const ids = new Set([...detail.skills, ...self.skills].map((skill) => skill.skillId));
    return [...ids]
      .map((skillId) => {
        const target = detail.skills.find((skill) => skill.skillId === skillId);
        const mine = self.skills.find((skill) => skill.skillId === skillId);
        const targetShare = target ? target.totalDamage / detail.totalDamage : 0;
        const mineShare = mine ? mine.totalDamage / self.totalDamage : 0;
        return { skillId, targetShare, mineShare };
      })
      .sort((a, b) => b.targetShare - a.targetShare || b.mineShare - a.mineShare);
  });

  function pct(value: number) {
    return `${(value * 100).toFixed(value >= 0.1 ? 0 : 1)}%`;
  }

  function diffLabel(ratio: number) {
    if (!Number.isFinite(ratio) || ratio <= 0) return '—';
    const diff = (ratio - 1) * 100;
    if (Math.abs(diff) < 0.5) return '±0%';
    return `${diff > 0 ? '+' : ''}${diff.toFixed(0)}%`;
  }

  function diffClass(ratio: number) {
    if (!Number.isFinite(ratio) || Math.abs(ratio - 1) < 0.005) return '';
    return ratio > 1 ? 'up' : 'down';
  }

  function skillPct(skillDamage: number) {
    return pct(detail.totalDamage > 0 ? skillDamage / detail.totalDamage : 0);
  }
</script>

<section class="detail" style:--class-color={colorOf(detail.className)} aria-label="プレイヤー詳細">
  <header class="detail-head">
    <button class="back-btn" aria-label="戻る" onclick={onBack}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor"
        stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="m15 18-6-6 6-6"></path>
      </svg>
    </button>
    <span class="detail-icon" aria-hidden="true">
      {#if classIconUrl(detail.className)}
        <img src={classIconUrl(detail.className)} alt="" />
      {/if}
    </span>
    <div class="detail-title">
      <strong>{detail.name}</strong>
      <span>
        {classSpecLabel(detail.classSpecName) || '型不明'}
        {#if detail.abilityScore > 0} ・ BP {fmtCompact(detail.abilityScore)}{/if}
        ・ {bossOnly ? 'ボス本体' : '全体'} ・ {fmtSeconds(detail.elapsedMs)}
      </span>
    </div>
    {#if self && !isSelf}
      <button class="compare-btn" class:on={comparing} onclick={() => { comparing = !comparing; }}>
        {comparing ? '詳細' : '自分と比較'}
      </button>
    {/if}
  </header>

  {#if !comparing || !selfStats || !self}
    <div class="stat-grid">
      <div><span>DPS</span><strong>{fmtCompact(stats.dps)}</strong></div>
      <div><span>実働DPS</span><strong>{fmtCompact(stats.tdps)}</strong></div>
      <div><span>稼働率</span><strong>{pct(stats.uptime)}</strong></div>
      <div><span>手数（/秒）</span><strong>{stats.hitsPerSec.toFixed(1)}</strong></div>
      <div><span>1撃の平均</span><strong>{fmtCompact(stats.avgHit)}</strong></div>
      <div><span>合計</span><strong>{fmtCompact(detail.totalDamage)}</strong></div>
      <div><span>クリ率</span><strong>{pct(stats.critRate)}</strong></div>
      <div><span>クリダメ割合</span><strong>{pct(stats.critShare)}</strong></div>
      <div><span>幸運率 / 割合</span><strong>{pct(stats.luckyRate)} / {pct(stats.luckyShare)}</strong></div>
    </div>

    {#if detail.imagines?.length}
      <div class="section-title">バトルイマジン</div>
      <div class="imagines">
        {#each detail.imagines as imagine (imagine.skillId)}
          <div class:unused={imagine.totalDamage <= 0} title={`ID ${imagine.skillId}`}>
            <strong>{imagineLabel(imagine)}</strong>
            <span>
              {imagineValue(imagine, detail.totalDamage)}
              {#if imagine.hits > 0} ・ {fmtCompact(imagine.hits)}回{/if}
            </span>
          </div>
        {/each}
      </div>
      {#if !isSelf}
        <p class="note">他の人の装備は分からないため、ダメージを出したイマジンだけ表示します。</p>
      {/if}
    {/if}

    <div class="section-title">スキル内訳</div>
    <div class="table">
      <div class="trow head skills">
        <span>スキル</span><span>割合</span><span>平均</span><span>回数</span><span>クリ率</span>
      </div>
      {#each detail.skills as skill (skill.skillId)}
        <div class="trow skills">
          <span class="name" title={`ID ${skill.skillId}`}>{label(skill.skillId)}{#if imagineBaseId(skill.skillId) !== null}<em class="tag">イマジン</em>{/if}</span>
          <span>{skillPct(skill.totalDamage)}</span>
          <span>{fmtCompact(skill.hits > 0 ? skill.totalDamage / skill.hits : 0)}</span>
          <span>{fmtCompact(skill.hits)}</span>
          <span>{pct(skill.hits > 0 ? skill.critHits / skill.hits : 0)}</span>
        </div>
      {/each}
    </div>
  {:else}
    <div class="versus">
      <div><span>{detail.name}</span><strong>{fmtCompact(stats.dps)}</strong></div>
      <em class={diffClass(stats.dps / selfStats.dps)}>{diffLabel(stats.dps / selfStats.dps)}</em>
      <div class="mine"><span>あなた</span><strong>{fmtCompact(selfStats.dps)}</strong></div>
    </div>

    <div class="section-title">DPSの差の内訳（DPS ＝ 稼働率 × 手数 × 1撃の重さ）</div>
    {#each factors as factor (factor.label)}
      <div class="factor">
        <span>{factor.label}</span>
        <span class="factor-bar" aria-hidden="true">
          <span class={diffClass(factor.ratio)} style:width={`${Math.max(2, factor.weight * 100)}%`}></span>
        </span>
        <em class={diffClass(factor.ratio)}>{diffLabel(factor.ratio)}</em>
      </div>
    {/each}

    <div class="section-title">項目ごと（相手 / あなた）</div>
    <div class="table">
      {#each COMPARE_ROWS as [label, pick, format] (label)}
        <div class="trow compare">
          <span class="name">{label}</span>
          <span>{format(pick(stats))}</span>
          <span>{format(pick(selfStats))}</span>
          <em class={diffClass(pick(stats) / pick(selfStats))}>{diffLabel(pick(stats) / pick(selfStats))}</em>
        </div>
      {/each}
    </div>

    {#if imagineCompareRows.length}
      <div class="section-title">バトルイマジン（相手 / あなた、ダメージの割合）</div>
      <div class="table">
        {#each imagineCompareRows as row (row.skillId)}
          <div class="trow compare">
            <span class="name" title={`ID ${row.skillId}`}>{imagineName(row.skillId, skillNames)}</span>
            <span>{imagineShare(row.target, detail.totalDamage)}</span>
            <span>{imagineShare(row.mine, self.totalDamage)}</span>
            <span></span>
          </div>
        {/each}
      </div>
    {/if}

    <div class="section-title">スキルの割合（相手 / あなた）</div>
    {#if sameClass}
      <div class="table">
        {#each skillCompareRows as row (row.skillId)}
          <div class="trow compare">
            <span class="name" title={`ID ${row.skillId}`}>{label(row.skillId)}</span>
            <span>{pct(row.targetShare)}</span>
            <span>{pct(row.mineShare)}</span>
            <em class={row.targetShare > row.mineShare ? 'up' : row.targetShare < row.mineShare ? 'down' : ''}>
              {((row.targetShare - row.mineShare) * 100).toFixed(0)}pt
            </em>
          </div>
        {/each}
      </div>
    {:else}
      <p class="note">職業が違うため、スキルの比較はありません。</p>
    {/if}
  {/if}
</section>

<style>
  .detail {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    color: var(--text);
    font-size: 0.86em;
  }
  .detail-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .back-btn,
  .compare-btn {
    flex-shrink: 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    cursor: pointer;
  }
  .back-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
  }
  .compare-btn {
    padding: 4px 8px;
    font-size: 0.9em;
    font-weight: 700;
  }
  .back-btn:hover,
  .compare-btn:hover,
  .compare-btn.on {
    color: var(--text);
    border-color: var(--accent);
  }
  .detail-icon {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    background: var(--class-color);
  }
  .detail-icon img {
    width: 18px;
    height: 18px;
    object-fit: contain;
  }
  .detail-title {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .detail-title strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 1.05em;
  }
  .detail-title span {
    color: var(--text-dim);
    font-size: 0.84em;
  }
  .stat-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 5px;
  }
  .stat-grid div,
  .versus div {
    display: flex;
    flex-direction: column;
    padding: 5px 7px;
    border-radius: 6px;
    background: rgb(var(--surface) / 0.8);
    min-width: 0;
  }
  .stat-grid span,
  .versus span {
    color: var(--text-dim);
    font-size: 0.8em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .stat-grid strong,
  .versus strong {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .section-title {
    margin-top: 2px;
    color: var(--text-dim);
    font-size: 0.82em;
    font-weight: 700;
  }
  .table {
    display: flex;
    flex-direction: column;
  }
  .trow {
    display: grid;
    align-items: center;
    gap: 4px;
    padding: 3px 2px;
    border-top: 1px solid var(--border);
    font-variant-numeric: tabular-nums;
  }
  .trow.skills {
    grid-template-columns: minmax(0, 1fr) 40px 48px 40px 40px;
  }
  .trow.compare {
    grid-template-columns: minmax(0, 1fr) 58px 58px 46px;
  }
  .trow > :not(.name) {
    text-align: right;
  }
  .trow.head {
    border-top: none;
    color: var(--text-dim);
    font-size: 0.82em;
  }
  .trow .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .trow em,
  .versus em,
  .factor em {
    font-style: normal;
    font-weight: 700;
  }
  .versus {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: center;
    gap: 6px;
  }
  .versus .mine {
    text-align: right;
  }
  .factor {
    display: grid;
    grid-template-columns: 70px minmax(0, 1fr) 46px;
    align-items: center;
    gap: 6px;
  }
  .factor em {
    text-align: right;
  }
  .factor-bar {
    height: 6px;
    border-radius: 3px;
    background: rgb(var(--surface) / 0.8);
    overflow: hidden;
  }
  .factor-bar span {
    display: block;
    height: 100%;
    background: var(--text-dim);
  }
  .up {
    color: #f87171;
  }
  .down {
    color: #60a5fa;
  }
  .factor-bar span.up {
    background: #f87171;
  }
  .factor-bar span.down {
    background: #60a5fa;
  }
  .imagines {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 5px;
  }
  .imagines div {
    display: flex;
    flex-direction: column;
    padding: 5px 7px;
    border-radius: 6px;
    background: rgb(var(--surface) / 0.8);
    min-width: 0;
  }
  .imagines strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .imagines span {
    color: var(--text-dim);
    font-size: 0.8em;
    font-variant-numeric: tabular-nums;
  }
  .imagines div.unused strong {
    color: var(--text-dim);
  }
  .tag {
    margin-left: 4px;
    padding: 0 4px;
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text-dim);
    font-size: 0.78em;
    font-weight: 400 !important;
  }
  .note {
    margin: 0;
    color: var(--text-dim);
    font-size: 0.86em;
  }
</style>
