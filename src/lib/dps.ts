export interface DpsPlayerRow {
  uid: number;
  name: string;
  className: string;
  classSpecName: string;
  abilityScore: number;
  totalDamage: number;
  dps: number;
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

export interface DpsBossEntry {
  id: number;
  name: string;
}

export const DPS_BOSS_LIST_KEY = 'rchat-dps-boss-list';
export const DPS_ID_CACHE_KEY = 'rchat-dps-id-cache';

export interface DpsIdentity {
  name?: string;
  className?: string;
  classSpecName?: string;
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

export function isRealName(name: string | undefined): boolean {
  return !!name && name !== 'Unknown' && !name.startsWith('Player ') && !name.startsWith('Monster ');
}

export function isRealClass(className: string | undefined): boolean {
  return !!className && className !== 'Unknown Class' && className !== 'Unimplemented Class';
}

export function isRealClassSpec(classSpecName: string | undefined): boolean {
  return !!classSpecName && !!classSpecLabel(classSpecName);
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

export function loadBossList(): DpsBossEntry[] {
  if (typeof localStorage === 'undefined') return [];
  try {
    const saved = localStorage.getItem(DPS_BOSS_LIST_KEY);
    if (!saved) return [];
    const parsed = JSON.parse(saved);
    if (!Array.isArray(parsed)) return [];
    return normalizeBossList(parsed);
  } catch {
    return [];
  }
}

export function saveBossList(list: DpsBossEntry[]) {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(DPS_BOSS_LIST_KEY, JSON.stringify(normalizeBossList(list)));
}

export function normalizeBossList(list: unknown[]): DpsBossEntry[] {
  const byId = new Map<number, string>();
  for (const raw of list) {
    if (!raw || typeof raw !== 'object') continue;
    const entry = raw as Record<string, unknown>;
    const id = Number(entry.id);
    if (!Number.isInteger(id) || id < 0) continue;
    const name = typeof entry.name === 'string' && entry.name.trim() ? entry.name : `Boss ${id}`;
    byId.set(id, name);
  }
  return Array.from(byId, ([id, name]) => ({ id, name })).sort((a, b) => a.id - b.id);
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
}

const CLASS_SPEC_LABELS: Record<string, string> = {
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
