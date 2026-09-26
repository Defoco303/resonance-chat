import { imagineName } from '$lib/imagines';

export interface DpsPlayerRow {
  uid: number;
  name: string;
  className: string;
  classSpecName: string;
  abilityScore: number;
  totalDamage: number;
  dps: number;
  tdps?: number;
  activeTimeMs?: number;
  damagePct: number;
  critRate: number;
  hits: number;
  tankScore?: number;
  avgTaken?: number;
}

export interface DpsMeterPayload {
  totalDps: number;
  totalDamage: number;
  elapsedMs: number;
  topDamage: number;
  localPlayerUid: number;
  bossEngaged?: boolean;
  bossName?: string;
  players: DpsPlayerRow[];
  enemies?: DpsMonsterInfo[];
  metrics?: DpsMetricsPayload;
}

export interface DpsMonsterInfo {
  uid: number;
  id: number;
  name: string;
  eliteStatus: number;
  isBoss: boolean;
  damageTaken: number;
}

export type DpsMetricType = 'dps' | 'tank' | 'heal';
export type DpsScope = 'all' | 'boss';

export interface DpsMetricPayload {
  totalValue: number;
  totalRate: number;
  topValue: number;
  players: DpsPlayerRow[];
}

export interface DpsMetricsPayload {
  dps: DpsMetricPayload;
  dpsBossOnly: DpsMetricPayload;
  tank: DpsMetricPayload;
  heal: DpsMetricPayload;
}

export const DPS_ID_CACHE_KEY = 'rchat-dps-id-cache';

// 名前だけを保存する（職業・型は切り替えられるので保存しない）
export interface DpsIdentity {
  name?: string;
}

export function loadIdCache(): Record<string, DpsIdentity> {
  if (typeof localStorage === 'undefined') return {};
  try {
    const saved = localStorage.getItem(DPS_ID_CACHE_KEY);
    if (!saved) return {};
    const parsed = JSON.parse(saved);
    return parsed && typeof parsed === 'object' ? (parsed as Record<string, DpsIdentity>) : {};
  } catch {
    return {};
  }
}

export function saveIdCache(cache: Record<string, DpsIdentity>) {
  if (typeof localStorage === 'undefined') return;
  try {
    localStorage.setItem(DPS_ID_CACHE_KEY, JSON.stringify(cache));
  } catch {}
}

export type DpsEncounterEndReason = 'bossDefeated' | 'idle' | 'manual' | 'sceneChange';

export interface DpsEncounterEndPayload {
  reason: DpsEncounterEndReason;
  endedAtMs: number;
  result: DpsMeterPayload;
  details: DpsPlayerDetail[];
  detailsBossOnly: boolean;
}

// プレイヤー詳細・比較用のダメージ内訳
export interface DpsSkillDetail {
  skillId: number;
  totalDamage: number;
  hits: number;
  critHits: number;
  critDamage: number;
  luckyHits: number;
  luckyDamage: number;
}

export interface DpsPlayerDetail {
  uid: number;
  name: string;
  className: string;
  classSpecName: string;
  abilityScore: number;
  elapsedMs: number;
  activeTimeMs: number;
  totalDamage: number;
  hits: number;
  critHits: number;
  critDamage: number;
  luckyHits: number;
  luckyDamage: number;
  skills: DpsSkillDetail[];
  // バトルイマジン（自分は装備中の2つ＋使ったもの、他人は使ったものだけ）。古い履歴には無い
  imagines?: DpsImagineDetail[];
}

export interface DpsImagineDetail {
  skillId: number;
  equipped: boolean;
  totalDamage: number;
  hits: number;
}

// リザルトカードと履歴に使う1戦闘分の記録
export interface DpsEncounterRecord {
  id: string;
  reason: DpsEncounterEndReason;
  endedAtMs: number;
  bossName: string;
  elapsedMs: number;
  totalDps: number;
  totalDamage: number;
  // true ならボス本体へのダメージだけで集計した記録（古い記録には無い）
  bossOnly?: boolean;
  localPlayerUid: number;
  players: DpsPlayerRow[];
  topHeal?: DpsPlayerRow;
  topTank?: DpsPlayerRow;
  // 戦闘終了時点の全員の内訳（古い記録には無い）
  details?: DpsPlayerDetail[];
}

export const DPS_HISTORY_KEY = 'rchat-dps-history';
export const DPS_HISTORY_LIMIT = 30;

export const ENCOUNTER_END_LABELS: Record<DpsEncounterEndReason, string> = {
  bossDefeated: 'ボス撃破',
  idle: '30秒戦闘なし',
  manual: '手動リセット',
  sceneChange: '移動',
};

export function loadHistory(): DpsEncounterRecord[] {
  if (typeof localStorage === 'undefined') return [];
  try {
    const saved = localStorage.getItem(DPS_HISTORY_KEY);
    const parsed = saved ? JSON.parse(saved) : [];
    return Array.isArray(parsed) ? (parsed as DpsEncounterRecord[]) : [];
  } catch {
    return [];
  }
}

export function saveHistory(history: DpsEncounterRecord[]) {
  if (typeof localStorage === 'undefined') return;
  try {
    localStorage.setItem(DPS_HISTORY_KEY, JSON.stringify(history.slice(0, DPS_HISTORY_LIMIT)));
  } catch {}
}

export function isRealName(name: string | undefined): boolean {
  return !!name && name !== 'Unknown' && !name.startsWith('Player ') && !name.startsWith('Monster ');
}

export interface DpsClassColor {
  className: string;
  label: string;
  color: string;
}

export interface DpsSettings {
  classColors: DpsClassColor[];
  rankAnimation: 'flashy' | 'normal' | 'off';
  autoBossScope: boolean;
}

export const DPS_SETTINGS_KEY = 'rchat-dps-settings';

export const DEFAULT_DPS_CLASS_COLORS: DpsClassColor[] = [
  { className: 'Stormblade', label: 'ストームブレイド', color: '#f87171' },
  { className: 'Frost Mage', label: 'フロストメイジ', color: '#60a5fa' },
  { className: 'Wind Knight', label: 'ゲイルランサー', color: '#34d399' },
  { className: 'Verdant Oracle', label: 'ヴァーダントオラクル', color: '#a3e635' },
  { className: 'Heavy Guardian', label: 'ヘヴィガーディアン', color: '#f59e0b' },
  { className: 'Marksman', label: 'ディバインアーチャー', color: '#fb7185' },
  { className: 'Shield Knight', label: 'シールドファイター', color: '#38bdf8' },
  { className: 'Beat Performer', label: 'ビートパフォーマー', color: '#c084fc' },
  { className: 'Twin Striker', label: 'ツインストライカー', color: '#f97316' },
  { className: 'Unknown Class', label: '不明', color: '#94a3b8' },
  { className: 'Unimplemented Class', label: 'その他', color: '#94a3b8' },
]

export const DEFAULT_DPS_SETTINGS: DpsSettings = {
  classColors: DEFAULT_DPS_CLASS_COLORS.map((entry) => ({ ...entry })),
  rankAnimation: 'flashy',
  autoBossScope: true,
};

export function loadDpsSettings(): DpsSettings {
  if (typeof localStorage === 'undefined') return cloneDpsSettings(DEFAULT_DPS_SETTINGS);

  try {
    const saved = localStorage.getItem(DPS_SETTINGS_KEY);
    if (!saved) return cloneDpsSettings(DEFAULT_DPS_SETTINGS);
    const parsed = JSON.parse(saved) as Partial<DpsSettings>;
    const savedColors = Array.isArray(parsed.classColors) ? parsed.classColors : [];
    return {
      classColors: DEFAULT_DPS_CLASS_COLORS.map((defaults) => {
        const savedColor = savedColors.find((entry) => entry?.className === defaults.className);
        return {
          ...defaults,
          color: normalizeColor(savedColor?.color, defaults.color),
        };
      }),
      rankAnimation: normalizeRankAnimation(parsed.rankAnimation),
      autoBossScope: typeof parsed.autoBossScope === 'boolean' ? parsed.autoBossScope : true,
    };
  } catch {
    return cloneDpsSettings(DEFAULT_DPS_SETTINGS);
  }
}

export function saveDpsSettings(settings: DpsSettings) {
  localStorage.setItem(DPS_SETTINGS_KEY, JSON.stringify(settings));
}

export function classColor(settings: DpsSettings, className: string) {
  return settings.classColors.find((entry) => entry.className === className)?.color
    ?? settings.classColors.find((entry) => entry.className === 'Unknown Class')?.color
    ?? '#94a3b8';
}

export function classIconUrl(className: string) {
  const fileName = CLASS_ICON_FILES[className];
  return fileName ? encodeURI(`/class_icon/${fileName}`) : '';
}

export function classSpecLabel(classSpecName: string | undefined) {
  if (!classSpecName) return '';
  return CLASS_SPEC_LABELS[classSpecName.trim()] ?? '';
}

export function fmtCompact(value: number) {
  if (!Number.isFinite(value)) return '0';

  const sign = value < 0 ? '-' : '';
  const abs = Math.abs(value);
  const units = [
    { value: 1_000_000_000_000, suffix: 'T' },
    { value: 1_000_000_000, suffix: 'B' },
    { value: 1_000_000, suffix: 'M' },
    { value: 1_000, suffix: 'K' },
  ];
  const unit = units.find((entry) => abs >= entry.value);

  if (!unit) {
    return `${sign}${Math.round(abs).toLocaleString('en-US')}`;
  }

  const scaled = abs / unit.value;
  const digits = scaled >= 100 ? 0 : scaled >= 10 ? 1 : 2;
  return `${sign}${trimTrailingZero(scaled.toFixed(digits))}${unit.suffix}`;
}

export function fmtSeconds(ms: number) {
  if (!Number.isFinite(ms) || ms <= 0) return '0:00';
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}

function cloneDpsSettings(settings: DpsSettings): DpsSettings {
  return {
    classColors: settings.classColors.map((entry) => ({ ...entry })),
    rankAnimation: settings.rankAnimation,
    autoBossScope: settings.autoBossScope,
  };
}

function trimTrailingZero(value: string) {
  return value.replace(/\.0+$/, '').replace(/(\.\d*[1-9])0+$/, '$1');
}

function normalizeColor(value: unknown, fallback: string) {
  if (typeof value === 'string' && /^#[0-9a-fA-F]{6}$/.test(value)) {
    return value;
  }
  return fallback;
}

function normalizeRankAnimation(value: unknown): DpsSettings['rankAnimation'] {
  return value === 'normal' || value === 'off' || value === 'flashy' ? value : 'flashy';
}

const CLASS_ICON_FILES: Record<string, string> = {
  Stormblade: 'ストームブレイド.webp',
  'Frost Mage': 'フロストメイジ.webp',
  'Wind Knight': 'ゲイルランサー.webp',
  'Verdant Oracle': 'ヴァーダントオラクル.webp',
  'Heavy Guardian': 'ヘヴィガーディアン.webp',
  Marksman: 'ディバインアーチャー.webp',
  'Shield Knight': 'シールドファイター.webp',
  'Beat Performer': 'ビートパフォーマー.webp',
  'Twin Striker': 'ツインストライカー.webp',
}

const CLASS_SPEC_LABELS: Record<string, string> = {
  'Twin Flame': '双炎',
  'Flame Dance': '炎舞',
  Iaido: '雷刃',
  'Iaido Slash': '雷刃',
  'Iaido Style': '雷刃',
  Moonstrike: '月影',
  Moonblade: '月影',
  Vanguard: '烈風',
  'Overdrive Style': '烈風',
  Aerial: '乱風',
  Skyward: '乱風',
  'Skyward Style': '乱風',
  Icicle: '氷牙',
  'Frost Lance Style': '氷牙',
  Frostbeam: '霜天',
  'Ray Style': '霜天',
  Wildpack: '狼弓',
  'Beast Master': '狼弓',
  Falconry: '鷹弓',
  Earthfort: '剛身',
  Stonewall: '剛身',
  Block: '剛守',
  Recovery: '光砕',
  'Bulwark Style': '光砕',
  Shield: '光盾',
  'Radiant Guard Style': '光盾',
  Smite: '威咲',
  'Thorn Lash': '威咲',
  Lifebind: '森癒',
  'Healing Style': '森癒',
  Dissonance: '狂音',
  Concerto: '響奏',
};

// スキル名の表（参考ツール StarResonanceDpsAnalysis の skills.ja-JP.json）。
// 大きいので詳細画面を開いたときに読み込む
export type SkillNameTable = Record<string, string>;
let skillNameTable: Promise<SkillNameTable> | null = null;

export function loadSkillNames(): Promise<SkillNameTable> {
  skillNameTable ??= import('$lib/data/skill-names.ja.json')
    .then((module) => module.default as SkillNameTable)
    .catch(() => {
      skillNameTable = null;
      return {};
    });
  return skillNameTable;
}

// ツインストライカーのスキル名。ゲーム画面とログで確かめたもの（表では中国語のままのため優先する）
const SKILL_NAMES: Record<number, string> = {
  1601: 'ブレイズスイング',
  1602: 'ブレイズスイング',
  1603: 'ブレイズスイング',
  1604: 'ブレイズスイング',
  1605: 'スパイラルブロウ',
  1606: 'ブレイズアクス',
  1607: 'ツインラッシュ',
  // ラースアクスは4段まで。段ごとに ID が分かれている
  1608: 'ラースアクス（1段）',
  1609: 'ラースアクス（2段）',
  1610: 'ラースアクス（3段）',
  1611: 'ラースアクス（4段）',
  1612: 'アクススパイラル',
  1613: 'バーニングラッシュ',
  1614: 'ラーヴァインパクト',
  1615: 'バーニングギア',
  1616: 'バーニングクロス',
  1617: 'イグニッション',
  1618: 'ヴォルテクスインパクト',
  // クリムゾンブロウ: バーニングギア中にスパイラルブロウが変化した技（表では「飞腾炎华」一段・二段）
  1619: 'クリムゾンブロウ（1段）',
  1620: 'クリムゾンブロウ（2段）',
  // イラプション: ダメージを与えるたびに確率で出る追加攻撃（1段 20%・2段 10%・3段 5%）
  35107: 'イラプション（1段）',
  35108: 'イラプション（2段）',
  35109: 'イラプション（3段）',
};

// 表で中国語のままの名前 → ゲーム内の日本語名。表の名前がこれで始まれば置き換える
// （例: 迷幻梦境-不灭战姿 → 夢幻迷界）
const CHINESE_NAME_TO_JA: Array<[string, string]> = [
  ['梦幻之力', '夢幻の力'],
  ['虚妄裁定', '虚妄断罪'],
  ['无尽思维', '無限思念'],
  ['寂灭之梦', '寂滅の夢'],
  ['幻想冲击', 'イマジンインパクト'],
  ['幻梦处决', '幻夢の断罪'],
  ['梦幻之箭', '夢幻の矢'],
  ['迷幻梦境', '夢幻迷界'],
  // バトルイマジンの技名（39xx 以外の ID で出る効果。どのイマジン由来かは未確認のため技名で出す）
  ['奥义！神灵依凭', '奥義！神霊憑依'],
  ['奥义！佑世边界', '奥義！加護結界'],
  ['奥义！万象升华', '奥義！万象昇華'],
  ['奥义！炎灵之幸', '奥義！ラッキーフレイム'],
  ['炎角-主动触发', '炎角・アクティブ効果'],
  ['绝技！落雷惩戒', '絶技！サンダーパニッシュ'],
  ['绝技！治疗爆弹', '絶技！ヒールボム'],
  ['绝技！双重牙', '絶技！ダブルファング'],
  ['绝技！灵动出击', '絶技！ラッシュアタック'],
  ['奥义！魅影奇袭', '奥義！ファントムレイド'],
  ['奥义！卷心菜重锤', '奥義！甘藍ヘビーハンマー'],
];

function translateTableName(name: string | undefined): string | undefined {
  if (name === undefined) return undefined;
  return CHINESE_NAME_TO_JA.find(([chinese]) => name.startsWith(chinese))?.[1] ?? name;
}

// 名前の優先順: ツインストライカーの確認済み → イマジン名 → スキル名の表 → ID
export function skillLabel(skillId: number, table?: SkillNameTable | null): string {
  // 2031101〜2031199 は幸運の一撃（職業ごとに番号が違う）
  if (skillId >= 2031101 && skillId <= 2031199) return '幸運の一撃';
  return (
    SKILL_NAMES[skillId] ??
    imagineName(skillId, table) ??
    translateTableName(table?.[String(skillId)]) ??
    `スキル ${skillId}`
  );
}

// 詳細・比較で使う指標
export interface DpsDetailStats {
  dps: number;
  tdps: number;
  // 稼働率：実際に攻撃していた時間 / 戦闘時間
  uptime: number;
  // 手数：実働1秒あたりのヒット数
  hitsPerSec: number;
  avgHit: number;
  critRate: number;
  critShare: number;
  luckyRate: number;
  luckyShare: number;
}

function safeDiv(a: number, b: number) {
  return b > 0 && Number.isFinite(a / b) ? a / b : 0;
}

export function detailStats(detail: DpsPlayerDetail): DpsDetailStats {
  const elapsedSec = detail.elapsedMs / 1000;
  const activeSec = detail.activeTimeMs / 1000;
  return {
    dps: safeDiv(detail.totalDamage, elapsedSec),
    tdps: safeDiv(detail.totalDamage, activeSec),
    uptime: Math.min(1, safeDiv(detail.activeTimeMs, detail.elapsedMs)),
    hitsPerSec: safeDiv(detail.hits, activeSec),
    avgHit: safeDiv(detail.totalDamage, detail.hits),
    critRate: safeDiv(detail.critHits, detail.hits),
    critShare: safeDiv(detail.critDamage, detail.totalDamage),
    luckyRate: safeDiv(detail.luckyHits, detail.hits),
    luckyShare: safeDiv(detail.luckyDamage, detail.totalDamage),
  };
}

export interface DpsGapFactor {
  label: string;
  // 相手 / 自分 の比（1.15 なら相手が15%多い）
  ratio: number;
  // DPSの差のうち、この要素が占める割合（0〜1）
  weight: number;
}

// DPS = 稼働率 × 手数 × 1撃の重さ なので、相手と自分の差をこの3つに分解する
export function dpsGapFactors(target: DpsDetailStats, self: DpsDetailStats): DpsGapFactor[] {
  const pairs: Array<[string, number, number]> = [
    ['稼働率', target.uptime, self.uptime],
    ['手数', target.hitsPerSec, self.hitsPerSec],
    ['1撃の重さ', target.avgHit, self.avgHit],
  ];
  const factors = pairs.map(([label, a, b]) => ({ label, ratio: a > 0 && b > 0 ? a / b : 1 }));
  const logs = factors.map((factor) => Math.abs(Math.log(factor.ratio)));
  const sum = logs.reduce((acc, value) => acc + value, 0);
  return factors.map((factor, index) => ({ ...factor, weight: sum > 0 ? logs[index] / sum : 0 }));
}
