use crate::packets::diag;
use crate::protocol::constants::{attr_type, damage, entity};
use crate::protocol::pb;
use bytes::{Buf, Bytes};
use prost::Message;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DPS_SERVICE_UUID: u64 = 0x63335342;

/// monster_id -> 表示名。ボスの判定には使わず、名前の表示にだけ使う（載っていない敵は ID で表示）
static MONSTER_NAMES: LazyLock<HashMap<u32, String>> = LazyLock::new(|| {
    let data = include_str!("../../../src/lib/data/json/MonsterNameBoss.json");
    let raw: HashMap<String, String> =
        serde_json::from_str(data).expect("invalid MonsterNameBoss.json");
    raw.into_iter()
        .filter_map(|(key, value)| key.parse::<u32>().ok().map(|id| (id, value)))
        .collect()
});

fn monster_name(monster_id: u32) -> Option<String> {
    MONSTER_NAMES.get(&monster_id).cloned()
}

const SYNC_NEAR_ENTITIES: u32 = 0x00000006;
const SYNC_SCENE_ATTRS: u32 = 0x00000007;
const SYNC_CONTAINER_DATA: u32 = 0x00000015;
const SYNC_NEAR_DELTA_INFO: u32 = 0x0000002d;
const SYNC_TO_ME_DELTA_INFO: u32 = 0x0000002e;
const ENTER_SCENE: u32 = 0x00000003;
const EMIT_INTERVAL: Duration = Duration::from_millis(250);
// 最後の戦闘からこれだけ空いたら、次の戦闘で集計を区切る
const IDLE_RESET_MS: u128 = 30_000;
// 実働時間: 攻撃間隔がこれ以内なら加算、超えたら（または初撃は）HIT_GRACE_MS だけ加算
const ACTIVE_INACTIVITY_CUTOFF_MS: u128 = 3_000;
const ACTIVE_HIT_GRACE_MS: u128 = 500;
// ボス撃破からリザルトを出すまでの待ち時間
const BOSS_DEFEAT_SETTLE_MS: u128 = 2_000;

/// 受信スレッドと画面からのコマンド（手動リセット）で共有する集計
#[derive(Default)]
pub struct DpsState(pub Mutex<DpsMeter>);

impl DpsState {
    pub fn lock(&self) -> MutexGuard<'_, DpsMeter> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
const TANK_SCORE_SCALE: i64 = 100;
const TANK_SINGLE_TARGET_SCORE: i64 = 100;
const TANK_SPLASH_TARGET_SCORE: i64 = 65;
const TANK_AOE_TARGET_SCORE: i64 = 20;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DpsMeterPayload {
    pub total_dps: f64,
    pub total_damage: f64,
    pub elapsed_ms: f64,
    pub top_damage: f64,
    pub local_player_uid: f64,
    pub boss_engaged: bool,
    pub boss_name: String,
    pub players: Vec<DpsPlayerRow>,
    pub enemies: Vec<MonsterInfo>,
    pub metrics: DpsMetricsPayload,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonsterInfo {
    pub uid: f64,
    pub id: f64,
    pub name: String,
    pub elite_status: f64,
    pub is_boss: bool,
    pub damage_taken: f64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DpsMetricsPayload {
    pub dps: DpsMetricPayload,
    pub dps_boss_only: DpsMetricPayload,
    pub tank: DpsMetricPayload,
    pub heal: DpsMetricPayload,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DpsMetricPayload {
    pub total_value: f64,
    pub total_rate: f64,
    pub top_value: f64,
    pub players: Vec<DpsPlayerRow>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DpsPlayerRow {
    pub uid: f64,
    pub name: String,
    pub class_name: String,
    pub class_spec_name: String,
    pub ability_score: f64,
    pub total_damage: f64,
    pub dps: f64,
    // 実働DPS（実際に攻撃していた時間で割ったDPS）。DPS以外の指標では0
    pub tdps: f64,
    pub active_time_ms: f64,
    pub damage_pct: f64,
    pub crit_rate: f64,
    pub hits: f64,
    pub tank_score: f64,
    pub avg_taken: f64,
}

#[derive(Debug, Default, Clone)]
struct CombatStats {
    value: i64,
    score: i64,
    hits: i64,
    crit_value: i64,
    crit_hits: i64,
    lucky_value: i64,
    lucky_hits: i64,
}

#[derive(Debug, Clone)]
struct Entity {
    entity_type: pb::EEntityType,
    dmg_stats: CombatStats,
    // 相手ごとのダメージ。ボス限定は表示時に今のボスリストで判定して集計する
    dmg_to_target: HashMap<i64, CombatStats>,
    // 実働DPS用: 実際に攻撃していた時間
    active_dmg_time_ms: u128,
    last_dmg_timestamp_ms: Option<u128>,
    heal_stats: CombatStats,
    tank_stats: CombatStats,
    skill_uid_to_dps_stats: HashMap<i32, CombatStats>,
    // （スキル, 相手）ごとのダメージ。ボス限定のスキル内訳に使う
    skill_target_stats: HashMap<(i32, i64), CombatStats>,
    name: Option<String>,
    class: Option<Class>,
    class_spec: Option<ClassSpec>,
    profession_id: Option<i32>,
    ability_score: Option<i32>,
    monster_id: Option<u32>,
    elite_status: Option<i64>,
    // ボスバー用の属性（0x1d7）が届いた。ボスにだけ送られる
    boss_bar: bool,
    damage_taken_total: i64,
}

type TankHitKey = (i64, i32);

impl Default for Entity {
    fn default() -> Self {
        Self {
            entity_type: pb::EEntityType::EntErrType,
            dmg_stats: CombatStats::default(),
            dmg_to_target: HashMap::new(),
            active_dmg_time_ms: 0,
            last_dmg_timestamp_ms: None,
            heal_stats: CombatStats::default(),
            tank_stats: CombatStats::default(),
            skill_uid_to_dps_stats: HashMap::new(),
            skill_target_stats: HashMap::new(),
            name: None,
            class: None,
            class_spec: None,
            profession_id: None,
            ability_score: None,
            monster_id: None,
            elite_status: None,
            boss_bar: false,
            damage_taken_total: 0,
        }
    }
}

impl Entity {
    /// ボスバー用の属性（0x1d7）が届いた敵をボスとする。
    /// 実測ではボス5体すべてに届き、精鋭・雑魚には届かなかった。一覧の手入れは不要。
    fn is_boss(&self) -> bool {
        self.entity_type == pb::EEntityType::EntMonster && self.boss_bar
    }

    fn display_name(&self) -> String {
        let id = self.monster_id.unwrap_or(0);
        self.monster_id
            .and_then(monster_name)
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| {
                if id == 0 {
                    "Monster (ID不明)".to_string()
                } else if self.boss_bar {
                    format!("ボス（ID {id}）")
                } else {
                    format!("Monster {id}")
                }
            })
    }
}

#[derive(Default)]
pub struct DpsMeter {
    entity_uid_to_entity: HashMap<i64, Entity>,
    total_dmg_stats: CombatStats,
    total_heal_stats: CombatStats,
    total_tank_stats: CombatStats,
    local_player_uid: Option<i64>,
    current_scene_id: Option<i32>,
    current_scene_guid: Option<String>,
    time_fight_start_ms: u128,
    time_last_combat_packet_ms: u128,
    last_emit: Option<Instant>,
    // 今の戦闘は終了済み（リザルトを送った）。次に数える戦闘は新しい集計から始める
    finished: bool,
    // ボス撃破後、少し待ってから終了にする（最後の多段ヒット・継続ダメージを含めるため）
    pending_end: Option<(EncounterEndReason, u128)>,
    // この戦闘でプレイヤーが攻撃したボスと、そのうち倒れたボス。全員倒れたら終了にする
    engaged_bosses: HashSet<i64>,
    defeated_bosses: HashSet<i64>,
    // ダメージ（プレイヤーの攻撃・敵からの被ダメージ）で戦闘が始まった。回復だけでは始めない
    combat_started: bool,
    end_events: Vec<EncounterEndPayload>,
    // 自分が装備中のバトルイマジン（スキル枠7・8のスキルID）。他人の装備は届かない
    local_imagines: Vec<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EncounterEndReason {
    BossDefeated,
    Idle,
    Manual,
    SceneChange,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncounterEndPayload {
    pub reason: EncounterEndReason,
    pub ended_at_ms: f64,
    pub result: DpsMeterPayload,
    // 戦闘後に詳細・比較を見るための全員の内訳（ボス戦ならボス本体へのダメージだけ）
    pub details: Vec<PlayerDetail>,
    pub details_boss_only: bool,
}

/// プレイヤー詳細・自分との比較に使う内訳（ダメージのみ）
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerDetail {
    pub uid: f64,
    pub name: String,
    pub class_name: String,
    pub class_spec_name: String,
    pub ability_score: f64,
    pub elapsed_ms: f64,
    pub active_time_ms: f64,
    pub total_damage: f64,
    pub hits: f64,
    pub crit_hits: f64,
    pub crit_damage: f64,
    pub lucky_hits: f64,
    pub lucky_damage: f64,
    pub skills: Vec<SkillDetail>,
    // バトルイマジン（装備中の自分の分と、ダメージを出したもの）
    pub imagines: Vec<ImagineDetail>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagineDetail {
    // 親のスキルID（例: 391001 → 3910）
    pub skill_id: f64,
    pub equipped: bool,
    pub total_damage: f64,
    pub hits: f64,
}

// スキル枠のうちバトルイマジンの枠
const IMAGINE_SLOTS: [i32; 2] = [7, 8];

/// バトルイマジンのスキルなら親のID（39xx）を返す。派生（39xxyy）もまとめる
fn imagine_base_id(skill_id: i32) -> Option<i32> {
    match skill_id {
        3900..=3999 => Some(skill_id),
        390000..=399999 => Some(skill_id / 100),
        _ => None,
    }
}

fn imagine_details(skills: &[SkillDetail], equipped: &[i32]) -> Vec<ImagineDetail> {
    let mut imagines: Vec<ImagineDetail> = equipped
        .iter()
        .map(|&id| ImagineDetail {
            skill_id: id as f64,
            equipped: true,
            total_damage: 0.0,
            hits: 0.0,
        })
        .collect();
    for skill in skills {
        let Some(base) = imagine_base_id(skill.skill_id as i32) else {
            continue;
        };
        let index = match imagines.iter().position(|i| i.skill_id == base as f64) {
            Some(index) => index,
            None => {
                imagines.push(ImagineDetail {
                    skill_id: base as f64,
                    equipped: false,
                    total_damage: 0.0,
                    hits: 0.0,
                });
                imagines.len() - 1
            }
        };
        imagines[index].total_damage += skill.total_damage;
        imagines[index].hits += skill.hits;
    }
    imagines
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    pub skill_id: f64,
    pub total_damage: f64,
    pub hits: f64,
    pub crit_hits: f64,
    pub crit_damage: f64,
    pub lucky_hits: f64,
    pub lucky_damage: f64,
}

fn skill_detail(skill_id: i32, stats: &CombatStats) -> SkillDetail {
    SkillDetail {
        skill_id: skill_id as f64,
        total_damage: stats.value as f64,
        hits: stats.hits as f64,
        crit_hits: stats.crit_hits as f64,
        crit_damage: stats.crit_value as f64,
        lucky_hits: stats.lucky_hits as f64,
        lucky_damage: stats.lucky_value as f64,
    }
}

impl DpsMeter {
    pub fn process_packet(&mut self, method_id: u32, payload: &[u8]) -> bool {
        match method_id {
            ENTER_SCENE => {
                let Some(packet) = decode_packet::<pb::EnterScene>(method_id, payload) else {
                    return false;
                };
                self.process_enter_scene(packet)
            }
            SYNC_SCENE_ATTRS => {
                let Some(packet) = decode_packet::<pb::SyncSceneAttrs>(method_id, payload) else {
                    return false;
                };
                self.process_sync_scene_attrs(packet)
            }
            SYNC_NEAR_ENTITIES => {
                let Some(packet) = decode_packet::<pb::SyncNearEntities>(method_id, payload) else {
                    return false;
                };
                self.process_sync_near_entities(packet);
                false
            }
            SYNC_CONTAINER_DATA => {
                let Some(packet) = decode_packet::<pb::SyncContainerData>(method_id, payload) else {
                    return false;
                };
                self.process_sync_container_data(packet);
                false
            }
            SYNC_TO_ME_DELTA_INFO => {
                let Some(packet) = decode_packet::<pb::SyncToMeDeltaInfo>(method_id, payload) else {
                    return false;
                };
                self.process_sync_to_me_delta_info(packet)
            }
            SYNC_NEAR_DELTA_INFO => {
                let Some(packet) = decode_packet::<pb::SyncNearDeltaInfo>(method_id, payload) else {
                    return false;
                };
                self.process_sync_near_delta_info(packet)
            }
            _ => false,
        }
    }

    pub fn should_emit(&mut self) -> bool {
        if self.is_empty() {
            return false;
        }
        let now = Instant::now();
        if self
            .last_emit
            .is_some_and(|last| now.duration_since(last) < EMIT_INTERVAL)
        {
            return false;
        }
        self.last_emit = Some(now);
        true
    }

    pub fn is_empty(&self) -> bool {
        self.total_dmg_stats.value <= 0
            && self.total_heal_stats.value <= 0
            && self.total_tank_stats.value <= 0
    }

    pub fn reset_combat_state(&mut self) {
        self.total_dmg_stats = CombatStats::default();
        self.total_heal_stats = CombatStats::default();
        self.total_tank_stats = CombatStats::default();
        self.time_fight_start_ms = 0;
        self.time_last_combat_packet_ms = 0;
        self.last_emit = None;
        self.finished = false;
        self.pending_end = None;
        self.engaged_bosses.clear();
        self.defeated_bosses.clear();
        self.combat_started = false;
        for entity in self.entity_uid_to_entity.values_mut() {
            entity.dmg_stats = CombatStats::default();
            entity.dmg_to_target.clear();
            entity.active_dmg_time_ms = 0;
            entity.last_dmg_timestamp_ms = None;
            entity.heal_stats = CombatStats::default();
            entity.tank_stats = CombatStats::default();
            entity.damage_taken_total = 0;
            entity.skill_uid_to_dps_stats.clear();
            entity.skill_target_stats.clear();
        }
    }

    pub fn reset_for_server_change(&mut self) {
        self.finish_encounter(EncounterEndReason::SceneChange, now_ms());
        self.current_scene_id = None;
        self.current_scene_guid = None;
        self.reset_combat_state();
    }

    /// 今の戦闘を終了し、リザルトを送る準備をする（空・終了済みなら何もしない）
    fn finish_encounter(&mut self, reason: EncounterEndReason, now: u128) {
        if self.finished || self.is_empty() {
            return;
        }
        diag::net(format!("encounter_end reason={reason:?}"));
        let details_boss_only = self.has_boss_damage();
        self.end_events.push(EncounterEndPayload {
            reason,
            ended_at_ms: now as f64,
            result: self.snapshot(),
            details: self.player_details(details_boss_only),
            details_boss_only,
        });
        self.finished = true;
        self.pending_end = None;
    }

    /// 受信のたびに呼ぶ。ボス撃破の待ち時間切れと、30秒間戦闘なしを終了にする
    pub fn poll(&mut self, now: u128) {
        if let Some((reason, at)) = self.pending_end {
            if now >= at {
                self.finish_encounter(reason, now);
            }
            return;
        }
        if !self.finished
            && self.time_last_combat_packet_ms != 0
            && now.saturating_sub(self.time_last_combat_packet_ms) >= IDLE_RESET_MS
        {
            self.finish_encounter(EncounterEndReason::Idle, now);
        }
    }

    pub fn take_end_events(&mut self) -> Vec<EncounterEndPayload> {
        std::mem::take(&mut self.end_events)
    }

    pub fn snapshot(&self) -> DpsMeterPayload {
        let elapsed_ms = self
            .time_last_combat_packet_ms
            .saturating_sub(self.time_fight_start_ms);
        let elapsed_secs = elapsed_ms as f64 / 1000.0;
        // ボス限定は、今のボス判定で相手ごとのダメージを集計し直す
        let boss_uids = self.boss_uids();
        let mut boss_stats_by_player: HashMap<i64, CombatStats> = HashMap::new();
        let mut total_boss_stats = CombatStats::default();
        for (&uid, entity) in &self.entity_uid_to_entity {
            if entity.entity_type != pb::EEntityType::EntChar {
                continue;
            }
            let mut stats = CombatStats::default();
            for (target_uid, target_stats) in &entity.dmg_to_target {
                if boss_uids.contains(target_uid) {
                    add_stats(&mut stats, target_stats);
                }
            }
            if stats.value > 0 {
                add_stats(&mut total_boss_stats, &stats);
                boss_stats_by_player.insert(uid, stats);
            }
        }

        let dps = self.metric_snapshot(&self.total_dmg_stats, elapsed_secs, true, |_, entity| {
            Some(entity.dmg_stats.clone())
        });
        let dps_boss_only =
            self.metric_snapshot(&total_boss_stats, elapsed_secs, true, |uid, _| {
                boss_stats_by_player.get(&uid).cloned()
            });
        let tank = self.tank_metric_snapshot(elapsed_secs);
        let heal = self.metric_snapshot(&self.total_heal_stats, elapsed_secs, false, |_, entity| {
            Some(entity.heal_stats.clone())
        });

        DpsMeterPayload {
            total_dps: dps.total_rate,
            total_damage: dps.total_value,
            elapsed_ms: elapsed_ms as f64,
            top_damage: dps.top_value,
            local_player_uid: self.local_player_uid.unwrap_or(-1) as f64,
            boss_engaged: total_boss_stats.value > 0,
            boss_name: self.current_boss_name(&boss_uids).unwrap_or_default(),
            players: dps.players.clone(),
            enemies: self.enemies(),
            metrics: DpsMetricsPayload {
                dps,
                dps_boss_only,
                tank,
                heal,
            },
        }
    }

    fn enemies(&self) -> Vec<MonsterInfo> {
        let mut monsters: Vec<MonsterInfo> = self
            .entity_uid_to_entity
            .iter()
            .filter(|(_, entity)| {
                entity.entity_type == pb::EEntityType::EntMonster
                    && entity.damage_taken_total > 0
            })
            .map(|(&uid, entity)| {
                let id = entity.monster_id.unwrap_or(0);
                MonsterInfo {
                    uid: uid as f64,
                    id: id as f64,
                    name: entity.display_name(),
                    elite_status: entity.elite_status.unwrap_or(0) as f64,
                    is_boss: entity.is_boss(),
                    damage_taken: entity.damage_taken_total as f64,
                }
            })
            .collect();
        monsters.sort_by(|a, b| {
            b.damage_taken
                .partial_cmp(&a.damage_taken)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        monsters.truncate(15);
        monsters
    }

    fn boss_uids(&self) -> HashSet<i64> {
        self.entity_uid_to_entity
            .iter()
            .filter(|(_, entity)| entity.is_boss())
            .map(|(&uid, _)| uid)
            .collect()
    }

    /// 全プレイヤーのダメージ内訳（合計の多い順）。boss_only ならボス本体へのダメージだけ
    pub fn player_details(&self, boss_only: bool) -> Vec<PlayerDetail> {
        let boss_uids = if boss_only { self.boss_uids() } else { HashSet::new() };
        let elapsed_ms = self
            .time_last_combat_packet_ms
            .saturating_sub(self.time_fight_start_ms);
        let mut details: Vec<PlayerDetail> = Vec::new();

        for (&uid, entity) in &self.entity_uid_to_entity {
            if entity.entity_type != pb::EEntityType::EntChar {
                continue;
            }
            let (stats, skills) = if boss_only {
                let mut stats = CombatStats::default();
                let mut by_skill: HashMap<i32, CombatStats> = HashMap::new();
                for (&(skill_id, target_uid), skill_stats) in &entity.skill_target_stats {
                    if boss_uids.contains(&target_uid) {
                        add_stats(&mut stats, skill_stats);
                        add_stats(by_skill.entry(skill_id).or_default(), skill_stats);
                    }
                }
                (stats, by_skill)
            } else {
                (entity.dmg_stats.clone(), entity.skill_uid_to_dps_stats.clone())
            };
            if stats.value <= 0 {
                continue;
            }

            let mut skills: Vec<SkillDetail> = skills
                .iter()
                .filter(|(_, skill_stats)| skill_stats.value > 0)
                .map(|(&skill_id, skill_stats)| skill_detail(skill_id, skill_stats))
                .collect();
            skills.sort_by(|a, b| b.total_damage.total_cmp(&a.total_damage));
            let equipped: &[i32] = if Some(uid) == self.local_player_uid {
                &self.local_imagines
            } else {
                &[]
            };
            let imagines = imagine_details(&skills, equipped);

            details.push(PlayerDetail {
                uid: uid as f64,
                name: entity.name.clone().unwrap_or_else(|| "Unknown".to_string()),
                class_name: class_name(entity.class.unwrap_or_default()),
                class_spec_name: class_spec_name(entity.class_spec.unwrap_or_default()),
                ability_score: entity.ability_score.unwrap_or(-1) as f64,
                elapsed_ms: elapsed_ms as f64,
                active_time_ms: entity.active_dmg_time_ms as f64,
                total_damage: stats.value as f64,
                hits: stats.hits as f64,
                crit_hits: stats.crit_hits as f64,
                crit_damage: stats.crit_value as f64,
                lucky_hits: stats.lucky_hits as f64,
                lucky_damage: stats.lucky_value as f64,
                skills,
                imagines,
            });
        }

        details.sort_by(|a, b| b.total_damage.total_cmp(&a.total_damage));
        details
    }

    fn has_boss_damage(&self) -> bool {
        let boss_uids = self.boss_uids();
        self.entity_uid_to_entity.values().any(|entity| {
            entity.entity_type == pb::EEntityType::EntChar
                && entity
                    .dmg_to_target
                    .iter()
                    .any(|(target_uid, stats)| boss_uids.contains(target_uid) && stats.value > 0)
        })
    }

    // ダメージを受けたボスの名前（複数ならダメージの多い順に「・」でつなぐ）
    fn current_boss_name(&self, boss_uids: &HashSet<i64>) -> Option<String> {
        let mut bosses: Vec<&Entity> = self
            .entity_uid_to_entity
            .iter()
            .filter(|(uid, entity)| boss_uids.contains(uid) && entity.damage_taken_total > 0)
            .map(|(_, entity)| entity)
            .collect();
        if bosses.is_empty() {
            return None;
        }
        bosses.sort_by_key(|entity| std::cmp::Reverse(entity.damage_taken_total));
        let mut names: Vec<String> = Vec::new();
        for boss in bosses {
            let name = boss.display_name();
            if !names.contains(&name) {
                names.push(name);
            }
        }
        Some(names.join("・"))
    }

    fn metric_snapshot<F>(
        &self,
        total_stats: &CombatStats,
        elapsed_secs: f64,
        with_active_time: bool,
        stats_for_entity: F,
    ) -> DpsMetricPayload
    where
        F: Fn(i64, &Entity) -> Option<CombatStats>,
    {
        let total_value = total_stats.value as f64;
        let mut players = Vec::new();
        let mut top_value: f64 = 0.0;

        for (&uid, entity) in &self.entity_uid_to_entity {
            if entity.entity_type != pb::EEntityType::EntChar {
                continue;
            }
            let Some(stats) = stats_for_entity(uid, entity) else {
                continue;
            };
            if stats.value <= 0 {
                continue;
            }

            let total = stats.value as f64;
            top_value = top_value.max(total);
            let active_time_ms = if with_active_time {
                entity.active_dmg_time_ms
            } else {
                0
            };
            players.push(DpsPlayerRow {
                uid: uid as f64,
                name: entity.name.clone().unwrap_or_else(|| "Unknown".to_string()),
                class_name: class_name(entity.class.unwrap_or_default()),
                class_spec_name: class_spec_name(entity.class_spec.unwrap_or_default()),
                ability_score: entity.ability_score.unwrap_or(-1) as f64,
                total_damage: total,
                dps: finite_or_zero(total / elapsed_secs),
                tdps: finite_or_zero(total / (active_time_ms as f64 / 1000.0)),
                active_time_ms: active_time_ms as f64,
                damage_pct: finite_or_zero(total / total_value * 100.0),
                crit_rate: finite_or_zero(stats.crit_hits as f64 / stats.hits as f64 * 100.0),
                hits: stats.hits as f64,
                tank_score: 0.0,
                avg_taken: 0.0,
            });
        }

        players.sort_by(|a, b| {
            b.total_damage
                .partial_cmp(&a.total_damage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        DpsMetricPayload {
            total_value,
            total_rate: finite_or_zero(total_value / elapsed_secs),
            top_value,
            players,
        }
    }

    fn tank_metric_snapshot(&self, elapsed_secs: f64) -> DpsMetricPayload {
        let total_score = scaled_tank_score(self.total_tank_stats.score);
        let mut players = Vec::new();
        let mut top_score: f64 = 0.0;

        for (&uid, entity) in &self.entity_uid_to_entity {
            let stats = &entity.tank_stats;
            if entity.entity_type != pb::EEntityType::EntChar || stats.hits <= 0 || stats.score <= 0 {
                continue;
            }

            let damage_taken = stats.value as f64;
            let tank_score = scaled_tank_score(stats.score);
            top_score = top_score.max(tank_score);
            players.push(DpsPlayerRow {
                uid: uid as f64,
                name: entity.name.clone().unwrap_or_else(|| "Unknown".to_string()),
                class_name: class_name(entity.class.unwrap_or_default()),
                class_spec_name: class_spec_name(entity.class_spec.unwrap_or_default()),
                ability_score: entity.ability_score.unwrap_or(-1) as f64,
                total_damage: damage_taken,
                dps: finite_or_zero(tank_score / elapsed_secs),
                tdps: 0.0,
                active_time_ms: 0.0,
                damage_pct: finite_or_zero(tank_score / total_score * 100.0),
                crit_rate: finite_or_zero(stats.crit_hits as f64 / stats.hits as f64 * 100.0),
                hits: stats.hits as f64,
                tank_score,
                avg_taken: finite_or_zero(damage_taken / stats.hits as f64),
            });
        }

        players.sort_by(|a, b| {
            b.tank_score
                .partial_cmp(&a.tank_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    b.hits
                        .partial_cmp(&a.hits)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    b.total_damage
                        .partial_cmp(&a.total_damage)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        DpsMetricPayload {
            total_value: total_score,
            total_rate: finite_or_zero(total_score / elapsed_secs),
            top_value: top_score,
            players,
        }
    }

    fn process_sync_near_entities(&mut self, packet: pb::SyncNearEntities) {
        for pkt_entity in packet.appear {
            let target_uuid = pkt_entity.uuid;
            if target_uuid == 0 {
                continue;
            }
            let target_uid = entity::get_player_uid(target_uuid);
            let target_entity_type = pb::EEntityType::from(target_uuid);
            let target_entity = self.entity_uid_to_entity.entry(target_uid).or_default();
            target_entity.entity_type = target_entity_type;

            if let Some(attrs) = pkt_entity.attrs {
                self.process_attrs(target_uid, attrs.attrs);
            }
        }
    }

    fn process_enter_scene(&mut self, packet: pb::EnterScene) -> bool {
        let Some(info) = packet.enter_scene_info else {
            return false;
        };

        let scene_id = extract_scene_id_from_enter_scene(&info);
        let scene_guid = info
            .scene_guid
            .filter(|value| !value.is_empty())
            .or_else(|| info.connect_guid.filter(|value| !value.is_empty()));

        self.apply_scene_identity(scene_id, scene_guid)
    }

    fn process_sync_scene_attrs(&mut self, packet: pb::SyncSceneAttrs) -> bool {
        let scene_id = packet
            .attrs
            .as_ref()
            .and_then(extract_scene_id_from_attr_collection);

        if scene_id.is_none() {
            return false;
        }

        self.apply_scene_identity(scene_id, None)
    }

    fn apply_scene_identity(
        &mut self,
        scene_id: Option<i32>,
        scene_guid: Option<String>,
    ) -> bool {
        let scene_changed = scene_id
            .zip(self.current_scene_id)
            .is_some_and(|(new_scene, old_scene)| new_scene != old_scene);
        let guid_changed = scene_guid
            .as_ref()
            .zip(self.current_scene_guid.as_ref())
            .is_some_and(|(new_guid, old_guid)| new_guid != old_guid);
        let changed = scene_changed || guid_changed;
        let had_combat = !self.is_empty();

        if let Some(scene_id) = scene_id {
            self.current_scene_id = Some(scene_id);
        }
        if let Some(scene_guid) = scene_guid {
            self.current_scene_guid = Some(scene_guid);
        }

        if changed && had_combat {
            self.finish_encounter(EncounterEndReason::SceneChange, now_ms());
            diag::net(format!(
                "meter_reset reason=scene_change scene_id={:?} scene_changed={scene_changed} guid_changed={guid_changed}",
                self.current_scene_id
            ));
            self.reset_combat_state();
            return true;
        }

        false
    }

    fn process_sync_container_data(&mut self, packet: pb::SyncContainerData) {
        let Some(v_data) = packet.v_data else {
            return;
        };
        let player_uid = v_data.char_id;
        if player_uid == 0 {
            return;
        }
        self.local_player_uid = Some(player_uid);
        if let Some(slots) = &v_data.slots {
            self.local_imagines = IMAGINE_SLOTS
                .iter()
                .filter_map(|slot| slots.slots.get(slot))
                .map(|info| info.skill_id)
                .filter(|&skill_id| imagine_base_id(skill_id).is_some())
                .collect();
        }
        let target_entity = self.entity_uid_to_entity.entry(player_uid).or_default();
        target_entity.entity_type = pb::EEntityType::EntChar;

        if let Some(char_base) = v_data.char_base {
            if !char_base.name.is_empty() {
                target_entity.name = Some(char_base.name);
            }
            if char_base.fight_point != 0 {
                target_entity.ability_score = Some(char_base.fight_point);
            }
        }

        if let Some(profession_list) = v_data.profession_list {
            if profession_list.cur_profession_id != 0 {
                apply_profession_id(player_uid, target_entity, profession_list.cur_profession_id);
            }
        }
    }

    fn process_sync_to_me_delta_info(&mut self, packet: pb::SyncToMeDeltaInfo) -> bool {
        let Some(delta_info) = packet.delta_info else {
            return false;
        };
        if delta_info.uuid != 0 {
            self.local_player_uid = Some(entity::get_player_uid(delta_info.uuid));
        }
        let Some(base_delta) = delta_info.base_delta else {
            return false;
        };
        self.process_aoi_sync_delta(base_delta, None, "tome")
    }

    fn process_sync_near_delta_info(&mut self, packet: pb::SyncNearDeltaInfo) -> bool {
        let tank_hit_target_counts = collect_tank_hit_target_counts(&packet.delta_infos);
        let mut changed = false;
        for delta in packet.delta_infos {
            changed |= self.process_aoi_sync_delta(delta, Some(&tank_hit_target_counts), "near");
        }
        changed
    }

    fn process_aoi_sync_delta(
        &mut self,
        delta: pb::AoiSyncDelta,
        tank_hit_target_counts: Option<&HashMap<TankHitKey, usize>>,
        source: &'static str,
    ) -> bool {
        let target_uuid = delta.uuid;
        if target_uuid == 0 {
            return false;
        }
        let target_uid = entity::get_player_uid(target_uuid);
        let target_entity_type = pb::EEntityType::from(target_uuid);

        {
            let target_entity = self.entity_uid_to_entity.entry(target_uid).or_default();
            target_entity.entity_type = target_entity_type;
        }

        if let Some(attrs) = delta.attrs {
            self.process_attrs(target_uid, attrs.attrs);
        }

        let Some(skill_effect) = delta.skill_effects else {
            return false;
        };

        let target_is_boss = diag::enabled() && self.is_boss_target(target_uid);
        let now = now_ms();

        let mut changed = false;
        // ダメージがあったか（最後に戦闘があった時刻は、回復ではなくダメージのときだけ進める）
        let mut damage_seen = false;
        for damage_info in skill_effect.damages {
            if diag::enabled() {
                log_damage_row(source, target_uuid, target_is_boss, &damage_info);
            }

            if damage_info.is_miss {
                continue;
            }

            let is_heal = damage_info.r#type == pb::EDamageType::Heal as i32;
            let attacker_uuid = damage_attacker_uuid(&damage_info);
            let skill_uid = damage_info.owner_id;

            if attacker_uuid != 0 && skill_uid != 0 {
                // 戦闘を始めるのはダメージだけ。回復は戦闘中のものだけ数える
                let counts = if is_heal {
                    self.heal_counts(now)
                } else {
                    self.reset_if_idle(now);
                    self.combat_started = true;
                    damage_seen = true;
                    true
                };
                let attacker_uid = entity::get_player_uid(attacker_uuid);
                let attacker_entity_type = pb::EEntityType::from(attacker_uuid);
                // 合計・割合はプレイヤーの攻撃だけで出す（モンスターの攻撃は含めない）
                let attacker_is_player = attacker_entity_type == pb::EEntityType::EntChar;
                let attacker_entity = self.entity_uid_to_entity.entry(attacker_uid).or_default();
                attacker_entity.entity_type = attacker_entity_type;
                infer_class_from_skill(attacker_uid, attacker_entity, skill_uid);

                if !counts {
                    // 戦闘外（撃破後・町など）の回復は数えない
                } else if is_heal {
                    process_stats(&damage_info, &mut attacker_entity.heal_stats);
                    if attacker_is_player {
                        process_stats(&damage_info, &mut self.total_heal_stats);
                    }
                } else {
                    let skill_stats = attacker_entity
                        .skill_uid_to_dps_stats
                        .entry(skill_uid)
                        .or_default();
                    process_stats(&damage_info, skill_stats);
                    process_stats(&damage_info, &mut attacker_entity.dmg_stats);
                    process_stats(
                        &damage_info,
                        attacker_entity.dmg_to_target.entry(target_uid).or_default(),
                    );
                    process_stats(
                        &damage_info,
                        attacker_entity
                            .skill_target_stats
                            .entry((skill_uid, target_uid))
                            .or_default(),
                    );
                    update_active_damage_time(attacker_entity, now);
                    if attacker_is_player {
                        process_stats(&damage_info, &mut self.total_dmg_stats);
                    }
                }
                if counts {
                    changed = true;
                }
            }

            if !is_heal && target_entity_type == pb::EEntityType::EntChar {
                let hp_loss = damage_info.hp_lessen_value.max(0);
                let shield_loss = damage_info.shield_lessen_value.max(0);
                let taken_value = hp_loss.saturating_add(shield_loss);
                if taken_value > 0 {
                    if let Some(tank_key) = tank_hit_key(&damage_info) {
                        self.reset_if_idle(now);
                        self.combat_started = true;
                        damage_seen = true;
                        let target_count = tank_hit_target_counts
                            .and_then(|counts| counts.get(&tank_key).copied())
                            .unwrap_or(1);
                        let tank_score = tank_score_for_hit(target_count, taken_value);
                        let target_entity =
                            self.entity_uid_to_entity.entry(target_uid).or_default();
                        target_entity.entity_type = target_entity_type;
                        process_tank_stats(
                            &damage_info,
                            taken_value,
                            tank_score,
                            &mut target_entity.tank_stats,
                        );
                        process_tank_stats(
                            &damage_info,
                            taken_value,
                            tank_score,
                            &mut self.total_tank_stats,
                        );
                        changed = true;
                    }
                }
            }

            // 敵ごとの被ダメージ（ENEMY タブ・ボス名の表示用）とボス撃破の判定
            if !is_heal && target_entity_type == pb::EEntityType::EntMonster {
                let value = damage_info
                    .hp_lessen_value
                    .saturating_add(damage_info.shield_lessen_value)
                    .max(0);
                let value = if value > 0 {
                    value
                } else if damage_info.lucky_value != 0 {
                    damage_info.lucky_value
                } else {
                    damage_info.value
                };
                let target_entity = self.entity_uid_to_entity.entry(target_uid).or_default();
                target_entity.entity_type = target_entity_type;
                if value > 0 {
                    target_entity.damage_taken_total =
                        target_entity.damage_taken_total.saturating_add(value);
                }
                if target_entity.is_boss() {
                    let by_player = attacker_uuid != 0
                        && pb::EEntityType::from(attacker_uuid) == pb::EEntityType::EntChar;
                    if by_player && self.engaged_bosses.insert(target_uid) {
                        diag::net(format!("boss_engaged target_uid={target_uid}"));
                    }
                    // 撃破は is_dead だけで判定する（HP属性は実際のHPと合わないことがあるため使わない）
                    if damage_info.is_dead && self.engaged_bosses.contains(&target_uid) {
                        self.defeated_bosses.insert(target_uid);
                        self.schedule_end_if_all_bosses_defeated(target_uid, now);
                    }
                }
            }
        }

        if damage_seen {
            if self.time_fight_start_ms == 0 {
                self.time_fight_start_ms = now;
            }
            self.time_last_combat_packet_ms = now;
        }

        changed
    }

    // 最後の戦闘から IDLE_RESET_MS 以上空いていたら、次の戦闘を数える前に集計を空にする。
    // 戦闘後も次の戦闘までは結果が画面に残る。
    // 終了済みの戦闘（ボス撃破・30秒戦闘なし）があれば、次の戦闘を数える前に集計を空にする。
    // 受信が途切れて poll で終了できていなかった場合も、ここで終了してから空にする。
    fn reset_if_idle(&mut self, now: u128) {
        if self.pending_end.is_some() {
            return;
        }
        let idle = self.time_last_combat_packet_ms != 0
            && now.saturating_sub(self.time_last_combat_packet_ms) >= IDLE_RESET_MS;
        if !self.finished && !idle {
            return;
        }
        if idle {
            self.finish_encounter(EncounterEndReason::Idle, now);
        }
        diag::net(format!(
            "meter_reset reason=next_encounter idle_ms={}",
            now.saturating_sub(self.time_last_combat_packet_ms)
        ));
        self.reset_combat_state();
    }

    // 回復を数えるのは、ダメージで始まった戦闘の最中だけ（終了後・30秒以上戦闘なし・戦闘前は数えない）
    fn heal_counts(&self, now: u128) -> bool {
        self.combat_started
            && !self.finished
            && now.saturating_sub(self.time_last_combat_packet_ms) < IDLE_RESET_MS
    }

    // この戦闘で攻撃したボスが全員倒れたら、少し待ってから終了にする（ボス2体の戦闘は2体目で終わる）
    fn schedule_end_if_all_bosses_defeated(&mut self, target_uid: i64, now: u128) {
        diag::net(format!("boss_defeated target_uid={target_uid}"));
        if self.finished || self.pending_end.is_some() || self.is_empty() {
            return;
        }
        if !self.engaged_bosses.is_subset(&self.defeated_bosses) {
            return;
        }
        self.pending_end = Some((
            EncounterEndReason::BossDefeated,
            now.saturating_add(BOSS_DEFEAT_SETTLE_MS),
        ));
    }

    pub fn reset_manually(&mut self) {
        self.finish_encounter(EncounterEndReason::Manual, now_ms());
        diag::net("meter_reset reason=manual".to_string());
        self.reset_combat_state();
    }

    fn is_boss_target(&self, target_uid: i64) -> bool {
        self.entity_uid_to_entity
            .get(&target_uid)
            .is_some_and(|entity| entity.is_boss())
    }

    fn process_attrs(&mut self, entity_uid: i64, attrs: Vec<pb::Attr>) {
        let target_entity = self.entity_uid_to_entity.entry(entity_uid).or_default();
        for attr in attrs {
            if attr.id == 0 || attr.raw_data.is_empty() {
                continue;
            }

            match attr.id {
                attr_type::ATTR_NAME => {
                    if let Some(name) = decode_protobuf_string(&attr.raw_data) {
                        target_entity.name = Some(name);
                    }
                }
                attr_type::ATTR_PROFESSION_ID => {
                    if let Ok(class_id) = decode_protobuf_int32(&attr.raw_data) {
                        apply_profession_id(entity_uid, target_entity, class_id);
                    }
                }
                attr_type::ATTR_FIGHT_POINT => {
                    if let Ok(ability_score) = decode_protobuf_int32(&attr.raw_data) {
                        target_entity.ability_score = Some(ability_score);
                    }
                }
                attr_type::ATTR_ID => {
                    if let Ok(id) = decode_protobuf_int32(&attr.raw_data) {
                        if id >= 0 {
                            target_entity.monster_id = Some(id as u32);
                        }
                    }
                }
                attr_type::ATTR_ELITE_STATUS => {
                    if let Ok(status) = decode_protobuf_int32(&attr.raw_data) {
                        target_entity.elite_status = Some(status as i64);
                    }
                }
                attr_type::ATTR_BOSS_BAR_TARGET => {
                    if !target_entity.boss_bar {
                        target_entity.boss_bar = true;
                        diag::net(format!(
                            "boss_detected uid={entity_uid} monster_id={:?} attr_0x1d7={:?}",
                            target_entity.monster_id,
                            decode_protobuf_int64(&attr.raw_data).ok()
                        ));
                    }
                }
                _ => {}
            }
        }
    }

}

fn extract_scene_id_from_enter_scene(info: &pb::EnterSceneInfo) -> Option<i32> {
    for attrs in [info.subscene_attrs.as_ref(), info.scene_attrs.as_ref()]
        .into_iter()
        .flatten()
    {
        if let Some(scene_id) = extract_scene_id_from_attr_collection(attrs) {
            return Some(scene_id);
        }
    }

    info.player_ent
        .as_ref()
        .and_then(|entity| entity.attrs.as_ref())
        .and_then(extract_scene_id_from_attr_collection)
}

fn extract_scene_id_from_attr_collection(attrs: &pb::AttrCollection) -> Option<i32> {
    for attr in &attrs.attrs {
        if attr.id != attr_type::ATTR_ID || attr.raw_data.is_empty() {
            continue;
        }

        if let Ok(id) = decode_protobuf_int32(&attr.raw_data) {
            if id > 0 {
                return Some(id);
            }
        }
    }

    None
}

fn decode_packet<T: Message + Default>(method_id: u32, payload: &[u8]) -> Option<T> {
    match T::decode(Bytes::copy_from_slice(payload)) {
        Ok(packet) => Some(packet),
        Err(_) => {
            diag::decode_failure(method_id, payload.len());
            None
        }
    }
}

fn decode_protobuf_int32(data: &[u8]) -> Result<i32, prost::DecodeError> {
    let mut cursor = Cursor::new(data);
    prost::encoding::decode_varint(&mut cursor).map(|v| v as i32)
}

fn decode_protobuf_int64(data: &[u8]) -> Result<i64, prost::DecodeError> {
    let mut cursor = Cursor::new(data);
    prost::encoding::decode_varint(&mut cursor).map(|v| v as i64)
}

fn decode_protobuf_string(data: &[u8]) -> Option<String> {
    let mut buf = data;
    if let Ok(len) = prost::encoding::decode_varint(&mut buf) {
        let len = len as usize;
        if buf.remaining() >= len {
            if let Ok(value) = String::from_utf8(buf.copy_to_bytes(len).to_vec()) {
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
    }

    let fallback = String::from_utf8_lossy(data.get(1..).unwrap_or(data))
        .trim_end_matches('\0')
        .to_string();
    if fallback.is_empty() {
        None
    } else {
        Some(fallback)
    }
}

fn damage_attacker_uuid(damage_info: &pb::SyncDamageInfo) -> i64 {
    if damage_info.top_summoner_id != 0 {
        damage_info.top_summoner_id
    } else if damage_info.attacker_uuid != 0 {
        damage_info.attacker_uuid
    } else {
        0
    }
}

fn stat_value(damage_info: &pb::SyncDamageInfo) -> i64 {
    if damage_info.lucky_value != 0 {
        damage_info.lucky_value
    } else {
        damage_info.value
    }
}

fn process_stats(damage_info: &pb::SyncDamageInfo, stats: &mut CombatStats) {
    process_stats_value(damage_info, stat_value(damage_info), stats);
}

fn add_stats(stats: &mut CombatStats, other: &CombatStats) {
    stats.value += other.value;
    stats.score += other.score;
    stats.hits += other.hits;
    stats.crit_value += other.crit_value;
    stats.crit_hits += other.crit_hits;
    stats.lucky_value += other.lucky_value;
    stats.lucky_hits += other.lucky_hits;
}

// 参考ツール（resonance-logs）と同じ方式の実働時間
fn update_active_damage_time(entity: &mut Entity, timestamp_ms: u128) {
    let additional = match entity.last_dmg_timestamp_ms {
        Some(last) => {
            let delta = timestamp_ms.saturating_sub(last);
            if delta <= ACTIVE_INACTIVITY_CUTOFF_MS {
                delta
            } else {
                ACTIVE_HIT_GRACE_MS
            }
        }
        None => ACTIVE_HIT_GRACE_MS,
    };
    entity.active_dmg_time_ms = entity.active_dmg_time_ms.saturating_add(additional);
    entity.last_dmg_timestamp_ms = Some(timestamp_ms);
}

fn entity_type_label(uuid: i64) -> &'static str {
    if uuid == 0 {
        return "none";
    }
    match pb::EEntityType::from(uuid) {
        pb::EEntityType::EntChar => "char",
        pb::EEntityType::EntMonster => "monster",
        _ => "other",
    }
}

// process_aoi_sync_delta と同じ条件で、各ヒットをメーターがどう扱ったかを記録する
fn log_damage_row(
    source: &str,
    target_uuid: i64,
    target_is_boss: bool,
    damage_info: &pb::SyncDamageInfo,
) {
    let attacker_uuid = damage_attacker_uuid(damage_info);
    let is_heal = damage_info.r#type == pb::EDamageType::Heal as i32;
    let (decision, counted_value) = if damage_info.is_miss {
        ("skip_miss", 0)
    } else if attacker_uuid == 0 {
        ("skip_no_attacker", 0)
    } else if damage_info.owner_id == 0 {
        ("skip_no_skill", 0)
    } else if is_heal {
        ("heal", stat_value(damage_info))
    } else {
        ("damage", stat_value(damage_info))
    };

    diag::damage_row(format!(
        "{},{source},{target_uuid},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{decision},{counted_value},{}",
        diag::now_ms(),
        entity_type_label(target_uuid),
        damage_info.attacker_uuid,
        damage_info.top_summoner_id,
        entity_type_label(attacker_uuid),
        damage_info.owner_id,
        damage_info.r#type,
        damage_info.type_flag,
        damage_info.damage_source,
        damage_info.is_crit as u8,
        damage_info.is_miss as u8,
        damage_info.is_dead as u8,
        damage_info.value,
        damage_info.lucky_value,
        damage_info.actual_value,
        damage_info.hp_lessen_value,
        damage_info.shield_lessen_value,
        damage_info.hit_event_id,
        damage_info.property,
        target_is_boss as u8,
    ));
}

fn process_stats_value(damage_info: &pb::SyncDamageInfo, actual_value: i64, stats: &mut CombatStats) {
    let is_lucky = damage_info.lucky_value != 0;
    let is_crit = (damage_info.type_flag & damage::CRIT_BIT) != 0;

    if is_crit {
        stats.crit_hits += 1;
        stats.crit_value += actual_value;
    }
    if is_lucky {
        stats.lucky_hits += 1;
        stats.lucky_value += actual_value;
    }
    stats.hits += 1;
    stats.value += actual_value;
}

fn process_tank_stats(
    damage_info: &pb::SyncDamageInfo,
    damage_taken: i64,
    tank_score: i64,
    stats: &mut CombatStats,
) {
    process_stats_value(damage_info, damage_taken, stats);
    stats.score += tank_score;
}

fn collect_tank_hit_target_counts(deltas: &[pb::AoiSyncDelta]) -> HashMap<TankHitKey, usize> {
    let mut targets_by_hit: HashMap<TankHitKey, HashSet<i64>> = HashMap::new();

    for delta in deltas {
        if delta.uuid == 0 || pb::EEntityType::from(delta.uuid) != pb::EEntityType::EntChar {
            continue;
        }

        let target_uid = entity::get_player_uid(delta.uuid);
        let Some(skill_effect) = &delta.skill_effects else {
            continue;
        };

        for damage_info in &skill_effect.damages {
            let damage_taken = damage_info
                .hp_lessen_value
                .max(0)
                .saturating_add(damage_info.shield_lessen_value.max(0));
            if damage_taken <= 0 {
                continue;
            }
            if let Some(key) = tank_hit_key(damage_info) {
                targets_by_hit.entry(key).or_default().insert(target_uid);
            }
        }
    }

    targets_by_hit
        .into_iter()
        .map(|(key, targets)| (key, targets.len()))
        .collect()
}

fn tank_hit_key(damage_info: &pb::SyncDamageInfo) -> Option<TankHitKey> {
    if damage_info.is_miss || damage_info.r#type == pb::EDamageType::Heal as i32 {
        return None;
    }
    if damage_info.owner_id == 0 {
        return None;
    }

    let source_uuid = if damage_info.top_summoner_id != 0 {
        damage_info.top_summoner_id
    } else {
        damage_info.attacker_uuid
    };
    if !is_enemy_damage_source(source_uuid) {
        return None;
    }

    Some((source_uuid, damage_info.owner_id))
}

fn is_enemy_damage_source(source_uuid: i64) -> bool {
    source_uuid != 0 && pb::EEntityType::from(source_uuid) == pb::EEntityType::EntMonster
}

fn tank_score_for_hit(target_count: usize, damage_taken: i64) -> i64 {
    let base = match target_count {
        0 | 1 => TANK_SINGLE_TARGET_SCORE,
        2 => TANK_SPLASH_TARGET_SCORE,
        _ => TANK_AOE_TARGET_SCORE,
    };
    let bonus = if target_count <= 2 {
        tank_severity_bonus(damage_taken)
    } else {
        0
    };
    base + bonus
}

fn tank_severity_bonus(damage_taken: i64) -> i64 {
    match damage_taken {
        1_000_000.. => 35,
        500_000.. => 25,
        200_000.. => 15,
        100_000.. => 8,
        _ => 0,
    }
}

fn scaled_tank_score(score: i64) -> f64 {
    score as f64 / TANK_SCORE_SCALE as f64
}

// 職業はころころ切り替えられるので、最後に分かった情報（スキル・職業ID）を優先する
fn infer_class_from_skill(uid: i64, entity: &mut Entity, skill_uid: i32) {
    let spec = ClassSpec::from_skill_id(skill_uid);
    if spec != ClassSpec::Unknown {
        let class = class_from_spec(spec);
        if entity.class_spec != Some(spec) || entity.class != Some(class) {
            set_class(uid, entity, class, spec, "skill", skill_uid);
        }
        return;
    }

    // 型は決められないが職業は決まるスキル。職業が変わったら型は不明に戻す
    if let Some(class) = class_from_shared_skill(skill_uid) {
        if entity.class != Some(class) {
            set_class(uid, entity, class, ClassSpec::Unknown, "skill", skill_uid);
        }
    }
}

// 職業IDは同じ値が繰り返し届くことがあるので、前回と違うときだけ反映する。
// 対応表にない職業ID（Unimplemented）でも、スキルで決めた職業を毎回上書きしないため。
fn apply_profession_id(uid: i64, entity: &mut Entity, profession_id: i32) {
    if entity.profession_id == Some(profession_id) {
        return;
    }
    entity.profession_id = Some(profession_id);
    set_class(
        uid,
        entity,
        Class::from(profession_id),
        ClassSpec::Unknown,
        "profession",
        profession_id,
    );
}

fn set_class(
    uid: i64,
    entity: &mut Entity,
    class: Class,
    spec: ClassSpec,
    reason: &str,
    source_id: i32,
) {
    if entity.entity_type == pb::EEntityType::EntChar
        && (entity.class != Some(class) || entity.class_spec != Some(spec))
    {
        diag::net(format!(
            "class_change uid={uid} from={}/{} to={}/{} by={reason}:{source_id}",
            class_name(entity.class.unwrap_or_default()),
            class_spec_name(entity.class_spec.unwrap_or_default()),
            class_name(class),
            class_spec_name(spec),
        ));
    }
    entity.class = Some(class);
    entity.class_spec = Some(spec);
}

fn class_from_shared_skill(skill_id: i32) -> Option<Class> {
    match skill_id {
        1601..=1604 | 1607..=1621 | 35104..=35109 | 160101 | 160102 => Some(Class::TwinStriker),
        _ => None,
    }
}

fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Stormblade,
    FrostMage,
    WindKnight,
    VerdantOracle,
    HeavyGuardian,
    Marksman,
    ShieldKnight,
    BeatPerformer,
    TwinStriker,
    Unimplemented,
    Unknown,
}

impl Default for Class {
    fn default() -> Self {
        Self::Unknown
    }
}

impl From<i32> for Class {
    fn from(class_id: i32) -> Self {
        match class_id {
            1 => Class::Stormblade,
            2 => Class::FrostMage,
            3 => Class::TwinStriker,
            4 => Class::WindKnight,
            5 => Class::VerdantOracle,
            9 => Class::HeavyGuardian,
            11 => Class::Marksman,
            12 => Class::ShieldKnight,
            13 => Class::BeatPerformer,
            _ => Class::Unimplemented,
        }
    }
}

fn class_name(class: Class) -> String {
    match class {
        Class::Stormblade => "Stormblade",
        Class::FrostMage => "Frost Mage",
        Class::WindKnight => "Wind Knight",
        Class::VerdantOracle => "Verdant Oracle",
        Class::HeavyGuardian => "Heavy Guardian",
        Class::Marksman => "Marksman",
        Class::ShieldKnight => "Shield Knight",
        Class::BeatPerformer => "Beat Performer",
        Class::TwinStriker => "Twin Striker",
        Class::Unknown => "Unknown Class",
        Class::Unimplemented => "Unimplemented Class",
    }
    .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClassSpec {
    Iaido,
    Moonstrike,
    Icicle,
    Frostbeam,
    Vanguard,
    Skyward,
    Smite,
    Lifebind,
    Earthfort,
    Block,
    Wildpack,
    Falconry,
    Recovery,
    Shield,
    Dissonance,
    Concerto,
    TwinFlame,
    FlameDance,
    Unknown,
}

impl Default for ClassSpec {
    fn default() -> Self {
        Self::Unknown
    }
}

impl ClassSpec {
    // 型で変わるのは特殊攻撃だけなので、各型の特殊攻撃（派生スキルは含まない）のIDだけを目印にする。
    // マスタリーや究極スキルは型をまたいで共通のため目印にしない。
    fn from_skill_id(skill_id: i32) -> Self {
        match skill_id {
            1714 | 171401 | 173401 => ClassSpec::Iaido, // 雷刃抜刀
            1715 | 173801 => ClassSpec::Moonstrike,       // 月影殺
            1242 | 120902 | 124201 | 12090201 => ClassSpec::Icicle, // フロストファング
            1241 | 124101 => ClassSpec::Frostbeam,                   // フロストレイ
            1405 | 1418 | 141801 => ClassSpec::Vanguard,             // 烈風撃
            1419 => ClassSpec::Skyward,                              // 飛燕乱
            1518 | 1530 | 1541 | 21402 | 151801 | 154101 => ClassSpec::Smite, // ワイルドブルーム
            1507 | 20301 | 21101 | 150701 => ClassSpec::Lifebind,   // バイタルブルーム
            1922 | 192201 => ClassSpec::Earthfort,                   // 砕刃の猛撃
            1930 | 1931 | 1934 | 50050 | 193001 | 193401 => ClassSpec::Block, // 護刃の衝撃
            2222 | 220106 => ClassSpec::Falconry,                    // ストライクボルト
            2220 | 2221 | 220104 | 222001 | 222101 => ClassSpec::Wildpack, // ハウリングボルト
            2405 | 240501 | 245001 => ClassSpec::Recovery,           // ブレイヴバッシュ
            240601 => ClassSpec::Shield,                             // グローイングガード
            2306 | 230601 => ClassSpec::Dissonance,                  // ハードディストーション
            2307 | 55302 | 230701 => ClassSpec::Concerto,            // コーラスヒーリング
            1605 => ClassSpec::TwinFlame,                            // スパイラルブロウ
            1606 => ClassSpec::FlameDance,                           // ブレイズアクス
            _ => ClassSpec::Unknown,
        }
    }
}

fn class_from_spec(class_spec: ClassSpec) -> Class {
    match class_spec {
        ClassSpec::Iaido | ClassSpec::Moonstrike => Class::Stormblade,
        ClassSpec::Icicle | ClassSpec::Frostbeam => Class::FrostMage,
        ClassSpec::Vanguard | ClassSpec::Skyward => Class::WindKnight,
        ClassSpec::Smite | ClassSpec::Lifebind => Class::VerdantOracle,
        ClassSpec::Earthfort | ClassSpec::Block => Class::HeavyGuardian,
        ClassSpec::Wildpack | ClassSpec::Falconry => Class::Marksman,
        ClassSpec::Recovery | ClassSpec::Shield => Class::ShieldKnight,
        ClassSpec::Dissonance | ClassSpec::Concerto => Class::BeatPerformer,
        ClassSpec::TwinFlame | ClassSpec::FlameDance => Class::TwinStriker,
        ClassSpec::Unknown => Class::Unknown,
    }
}

fn class_spec_name(class_spec: ClassSpec) -> String {
    match class_spec {
        ClassSpec::Iaido => "Iaido",
        ClassSpec::Moonstrike => "Moonstrike",
        ClassSpec::Icicle => "Icicle",
        ClassSpec::Frostbeam => "Frostbeam",
        ClassSpec::Vanguard => "Vanguard",
        ClassSpec::Skyward => "Skyward",
        ClassSpec::Smite => "Smite",
        ClassSpec::Lifebind => "Lifebind",
        ClassSpec::Earthfort => "Earthfort",
        ClassSpec::Block => "Block",
        ClassSpec::Wildpack => "Wildpack",
        ClassSpec::Falconry => "Falconry",
        ClassSpec::Recovery => "Recovery",
        ClassSpec::Shield => "Shield",
        ClassSpec::Dissonance => "Dissonance",
        ClassSpec::Concerto => "Concerto",
        ClassSpec::TwinFlame => "Twin Flame",
        ClassSpec::FlameDance => "Flame Dance",
        ClassSpec::Unknown => "Unknown Spec",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::{apply_profession_id, infer_class_from_skill, Class, ClassSpec, Entity};
    use crate::protocol::pb;

    fn player() -> Entity {
        Entity {
            entity_type: pb::EEntityType::EntChar,
            ..Entity::default()
        }
    }

    #[test]
    fn twin_striker_shared_skill_sets_class_then_special_attack_sets_spec() {
        let mut entity = player();
        infer_class_from_skill(1, &mut entity, 1601);
        assert_eq!(entity.class, Some(Class::TwinStriker));
        assert_eq!(entity.class_spec, Some(ClassSpec::Unknown));

        infer_class_from_skill(1, &mut entity, 1605);
        assert_eq!(entity.class, Some(Class::TwinStriker));
        assert_eq!(entity.class_spec, Some(ClassSpec::TwinFlame));

        // 共通スキルは同じ職業なら型を消さない
        infer_class_from_skill(1, &mut entity, 1602);
        assert_eq!(entity.class_spec, Some(ClassSpec::TwinFlame));
    }

    #[test]
    fn twin_striker_spec_follows_special_attack_only() {
        let mut entity = player();
        infer_class_from_skill(1, &mut entity, 1606);
        assert_eq!(entity.class, Some(Class::TwinStriker));
        assert_eq!(entity.class_spec, Some(ClassSpec::FlameDance));

        // マスタリー・究極スキル・追加効果では型は変わらない
        for skill in [1607, 1608, 1612, 1613, 1618, 35107, 35108, 35109, 160101, 160102] {
            infer_class_from_skill(1, &mut entity, skill);
            assert_eq!(entity.class_spec, Some(ClassSpec::FlameDance), "skill {skill}");
            assert_eq!(entity.class, Some(Class::TwinStriker), "skill {skill}");
        }

        infer_class_from_skill(1, &mut entity, 1605);
        assert_eq!(entity.class_spec, Some(ClassSpec::TwinFlame));
    }

    #[test]
    fn heavy_guardian_spec_follows_special_attack_only() {
        let mut entity = player();
        infer_class_from_skill(1, &mut entity, 1930);
        assert_eq!(entity.class, Some(Class::HeavyGuardian));
        assert_eq!(entity.class_spec, Some(ClassSpec::Block));

        // 地崩の山摧・怒撃は型の目印ではない
        infer_class_from_skill(1, &mut entity, 199902);
        infer_class_from_skill(1, &mut entity, 1935);
        assert_eq!(entity.class_spec, Some(ClassSpec::Block));

        infer_class_from_skill(1, &mut entity, 1922);
        assert_eq!(entity.class_spec, Some(ClassSpec::Earthfort));
    }

    #[test]
    fn existing_class_detection_is_unchanged() {
        let mut entity = player();
        infer_class_from_skill(1, &mut entity, 1241);
        assert_eq!(entity.class_spec, Some(ClassSpec::Frostbeam));
        assert_eq!(entity.class, Some(Class::FrostMage));
    }

    #[test]
    fn unmapped_skill_keeps_current_class() {
        let mut entity = player();
        infer_class_from_skill(1, &mut entity, 1241);
        infer_class_from_skill(1, &mut entity, 2031102);
        assert_eq!(entity.class, Some(Class::FrostMage));
        assert_eq!(entity.class_spec, Some(ClassSpec::Frostbeam));
    }

    #[test]
    fn special_attack_switches_class_and_spec() {
        let mut entity = player();
        infer_class_from_skill(1, &mut entity, 1241);
        infer_class_from_skill(1, &mut entity, 1606);
        assert_eq!(entity.class, Some(Class::TwinStriker));
        assert_eq!(entity.class_spec, Some(ClassSpec::FlameDance));
    }

    #[test]
    fn shared_skill_of_other_class_resets_spec() {
        let mut entity = player();
        infer_class_from_skill(1, &mut entity, 1241);
        infer_class_from_skill(1, &mut entity, 1601);
        assert_eq!(entity.class, Some(Class::TwinStriker));
        assert_eq!(entity.class_spec, Some(ClassSpec::Unknown));
    }

    #[test]
    fn changed_profession_id_updates_class_and_resets_spec() {
        let mut entity = player();
        apply_profession_id(1, &mut entity, 2);
        infer_class_from_skill(1, &mut entity, 1241);
        assert_eq!(entity.class_spec, Some(ClassSpec::Frostbeam));

        apply_profession_id(1, &mut entity, 3);
        assert_eq!(entity.class, Some(Class::TwinStriker));
        assert_eq!(entity.class_spec, Some(ClassSpec::Unknown));
    }

    #[test]
    fn repeated_profession_id_does_not_override_skill_class() {
        let mut entity = player();
        apply_profession_id(1, &mut entity, 999);
        assert_eq!(entity.class, Some(Class::Unimplemented));
        infer_class_from_skill(1, &mut entity, 1605);
        assert_eq!(entity.class, Some(Class::TwinStriker));

        apply_profession_id(1, &mut entity, 999);
        assert_eq!(entity.class, Some(Class::TwinStriker));
        assert_eq!(entity.class_spec, Some(ClassSpec::TwinFlame));
    }

    // ---- 集計（合計・ボス限定・自動リセット・実働時間） ----

    use super::{update_active_damage_time, DpsMeter};

    const PLAYER_UID: i64 = 100;
    const MONSTER_UID: i64 = 200;

    fn player_uuid(uid: i64) -> i64 {
        (uid << 16) | 640
    }

    fn monster_uuid(uid: i64) -> i64 {
        (uid << 16) | 64
    }

    fn hit(attacker_uuid: i64, value: i64) -> pb::SyncDamageInfo {
        pb::SyncDamageInfo {
            attacker_uuid,
            owner_id: 1,
            value,
            hp_lessen_value: value,
            ..Default::default()
        }
    }

    fn feed(meter: &mut DpsMeter, target_uuid: i64, damages: Vec<pb::SyncDamageInfo>) {
        meter.process_sync_near_delta_info(pb::SyncNearDeltaInfo {
            delta_infos: vec![pb::AoiSyncDelta {
                uuid: target_uuid,
                attrs: None,
                skill_effects: Some(pb::SkillEffect { damages }),
            }],
        });
    }

    #[test]
    fn monster_attacks_are_excluded_from_totals() {
        let mut meter = DpsMeter::default();
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 100)]);
        feed(&mut meter, player_uuid(PLAYER_UID), vec![hit(monster_uuid(MONSTER_UID), 50)]);

        let snapshot = meter.snapshot();
        assert_eq!(snapshot.total_damage, 100.0);
        assert_eq!(snapshot.players.len(), 1);
        assert_eq!(snapshot.players[0].damage_pct, 100.0);
    }

    #[test]
    fn boss_only_is_evaluated_when_displayed() {
        let mut meter = DpsMeter::default();
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 100)]);
        let before = meter.snapshot();
        assert_eq!(before.metrics.dps_boss_only.total_value, 0.0);
        assert!(!before.boss_engaged);

        // 後からボスだと分かっても、それまでのダメージがボス限定に入る
        meter
            .entity_uid_to_entity
            .get_mut(&MONSTER_UID)
            .expect("monster entity")
            .boss_bar = true;
        let after = meter.snapshot();
        assert_eq!(after.metrics.dps_boss_only.total_value, 100.0);
        assert_eq!(after.metrics.dps_boss_only.players.len(), 1);
        assert!(after.boss_engaged);
        assert!(!after.boss_name.is_empty());
    }

    #[test]
    fn idle_gap_resets_before_next_combat() {
        let mut meter = DpsMeter::default();
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 100)]);

        // 30秒未満の間隔なら続けて数える
        meter.time_last_combat_packet_ms -= 10_000;
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 20)]);
        assert_eq!(meter.snapshot().total_damage, 120.0);

        // 30秒以上空いたら、次の戦闘の前に集計を空にする
        meter.time_last_combat_packet_ms -= 31_000;
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 30)]);
        assert_eq!(meter.snapshot().total_damage, 30.0);
    }

    #[test]
    fn manual_reset_clears_meter() {
        let mut meter = DpsMeter::default();
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 100)]);
        meter.reset_manually();
        assert!(meter.is_empty());
        assert!(meter.snapshot().players.is_empty());
    }

    #[test]
    fn active_time_uses_cutoff_and_grace() {
        let mut entity = player();
        update_active_damage_time(&mut entity, 10_000);
        assert_eq!(entity.active_dmg_time_ms, 500);
        update_active_damage_time(&mut entity, 12_000);
        assert_eq!(entity.active_dmg_time_ms, 2_500);
        update_active_damage_time(&mut entity, 17_000);
        assert_eq!(entity.active_dmg_time_ms, 3_000);
    }

    // ---- 戦闘の終了（ボス撃破・30秒戦闘なし・手動リセット・移動） ----

    use super::{now_ms, EncounterEndReason};

    fn make_boss(meter: &mut DpsMeter, uid: i64) {
        let entity = meter.entity_uid_to_entity.entry(uid).or_default();
        entity.entity_type = pb::EEntityType::EntMonster;
        entity.boss_bar = true;
    }

    fn killing_hit(value: i64) -> pb::SyncDamageInfo {
        pb::SyncDamageInfo {
            is_dead: true,
            ..hit(player_uuid(PLAYER_UID), value)
        }
    }

    #[test]
    fn boss_defeat_ends_encounter_after_settle_time_once() {
        let mut meter = DpsMeter::default();
        make_boss(&mut meter, MONSTER_UID);
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 100)]);
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![killing_hit(50), killing_hit(50)]);
        // 撃破直後の追加ヒットも同じ戦闘に入る
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 10)]);

        meter.poll(now_ms());
        assert!(meter.take_end_events().is_empty());

        meter.poll(now_ms() + 2_000);
        let events = meter.take_end_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reason, EncounterEndReason::BossDefeated);
        assert_eq!(events[0].result.total_damage, 210.0);

        // 30秒たっても同じ戦闘のリザルトは二度出さない
        meter.poll(now_ms() + 40_000);
        assert!(meter.take_end_events().is_empty());

        // 撃破後の戦闘は新しい集計から始まる
        feed(&mut meter, monster_uuid(300), vec![hit(player_uuid(PLAYER_UID), 7)]);
        assert_eq!(meter.snapshot().total_damage, 7.0);
    }

    #[test]
    fn boss_is_detected_from_boss_bar_attribute() {
        let mut meter = DpsMeter::default();
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 100)]);
        assert!(!meter.snapshot().boss_engaged);

        // 0x1d7（ボスバー用・ターゲットを取っている人）が届いた敵はボスになる
        let raw = {
            let mut buf = Vec::new();
            prost::encoding::encode_varint(2_669_029, &mut buf);
            buf
        };
        meter.process_attrs(
            MONSTER_UID,
            vec![pb::Attr {
                id: 0x1d7,
                raw_data: raw,
            }],
        );
        let snapshot = meter.snapshot();
        assert!(snapshot.boss_engaged);
        assert_eq!(snapshot.metrics.dps_boss_only.total_value, 100.0);
    }

    #[test]
    fn damage_beyond_hp_without_is_dead_does_not_defeat_boss() {
        let mut meter = DpsMeter::default();
        make_boss(&mut meter, MONSTER_UID);
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 1_000_000_000)]);
        assert!(meter.pending_end.is_none());
    }

    #[test]
    fn two_bosses_end_when_both_are_defeated() {
        let mut meter = DpsMeter::default();
        make_boss(&mut meter, MONSTER_UID);
        make_boss(&mut meter, 201);
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 100)]);
        feed(&mut meter, monster_uuid(201), vec![hit(player_uuid(PLAYER_UID), 100)]);

        feed(&mut meter, monster_uuid(MONSTER_UID), vec![killing_hit(10)]);
        assert!(meter.pending_end.is_none());

        feed(&mut meter, monster_uuid(201), vec![killing_hit(10)]);
        assert!(meter.pending_end.is_some());
        meter.poll(now_ms() + 2_000);
        let events = meter.take_end_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reason, EncounterEndReason::BossDefeated);
    }

    fn heal(value: i64) -> pb::SyncDamageInfo {
        pb::SyncDamageInfo {
            r#type: pb::EDamageType::Heal as i32,
            ..hit(player_uuid(PLAYER_UID), value)
        }
    }

    fn skill_hit(skill: i32, value: i64) -> pb::SyncDamageInfo {
        pb::SyncDamageInfo {
            owner_id: skill,
            ..hit(player_uuid(PLAYER_UID), value)
        }
    }

    #[test]
    fn player_details_split_by_skill_and_boss_scope() {
        let mut meter = DpsMeter::default();
        make_boss(&mut meter, MONSTER_UID);
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![skill_hit(1605, 100), skill_hit(1607, 50)]);
        // 呼び出された雑魚（ボスではない）へのダメージ
        feed(&mut meter, monster_uuid(300), vec![skill_hit(1607, 30)]);

        let all = meter.player_details(false);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].total_damage, 180.0);
        assert_eq!(all[0].skills[0].skill_id, 1605.0);
        assert_eq!(all[0].skills[1].total_damage, 80.0);

        let boss = meter.player_details(true);
        assert_eq!(boss[0].total_damage, 150.0);
        assert_eq!(boss[0].skills.len(), 2);
        assert_eq!(boss[0].skills[1].total_damage, 50.0);

        feed(&mut meter, monster_uuid(MONSTER_UID), vec![killing_hit(10)]);
        meter.poll(now_ms() + 2_000);
        let events = meter.take_end_events();
        assert!(events[0].details_boss_only);
        assert_eq!(events[0].details[0].total_damage, 160.0);
    }

    #[test]
    fn imagines_merge_derived_ids_and_keep_equipped_without_damage() {
        let mut meter = DpsMeter::default();
        let slots = [(1, 1901), (7, 3948), (8, 3921)]
            .into_iter()
            .map(|(id, skill_id)| (id, pb::SlotInfo { id, skill_id }))
            .collect();
        meter.process_sync_container_data(pb::SyncContainerData {
            v_data: Some(pb::CharSerialize {
                char_id: PLAYER_UID,
                slots: Some(pb::Slot { slots }),
                ..Default::default()
            }),
        });
        feed(
            &mut meter,
            monster_uuid(MONSTER_UID),
            vec![skill_hit(391001, 100), skill_hit(391002, 50), skill_hit(1605, 10)],
        );

        let details = meter.player_details(false);
        let imagines = &details[0].imagines;
        assert_eq!(imagines.len(), 3);
        // 装備中の2つ（ダメージなし）が先、使った他のイマジンは後ろに付く
        assert_eq!((imagines[0].skill_id, imagines[0].equipped, imagines[0].total_damage), (3948.0, true, 0.0));
        assert_eq!(imagines[1].skill_id, 3921.0);
        assert_eq!((imagines[2].skill_id, imagines[2].equipped), (3910.0, false));
        assert_eq!((imagines[2].total_damage, imagines[2].hits), (150.0, 2.0));
    }

    #[test]
    fn heal_outside_combat_does_not_start_encounter() {
        let mut meter = DpsMeter::default();
        let changed = meter.process_sync_near_delta_info(pb::SyncNearDeltaInfo {
            delta_infos: vec![pb::AoiSyncDelta {
                uuid: player_uuid(PLAYER_UID),
                attrs: None,
                skill_effects: Some(pb::SkillEffect { damages: vec![heal(500)] }),
            }],
        });
        assert!(!changed);
        assert!(meter.is_empty());
        meter.poll(now_ms() + 60_000);
        assert!(meter.take_end_events().is_empty());
    }

    #[test]
    fn heal_during_combat_is_counted() {
        let mut meter = DpsMeter::default();
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 100)]);
        feed(&mut meter, player_uuid(PLAYER_UID), vec![heal(40)]);
        assert_eq!(meter.snapshot().metrics.heal.total_value, 40.0);
    }

    #[test]
    fn heal_after_boss_defeat_does_not_start_new_encounter() {
        let mut meter = DpsMeter::default();
        make_boss(&mut meter, MONSTER_UID);
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 100)]);
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![killing_hit(50)]);
        meter.poll(now_ms() + 2_000);
        assert_eq!(meter.take_end_events().len(), 1);

        // 撃破後の回復では集計はリセットされず、数えもしない
        feed(&mut meter, player_uuid(PLAYER_UID), vec![heal(999)]);
        let snapshot = meter.snapshot();
        assert_eq!(snapshot.total_damage, 150.0);
        assert_eq!(snapshot.metrics.heal.total_value, 0.0);

        // 次のダメージで新しい戦闘が始まる
        feed(&mut meter, monster_uuid(300), vec![hit(player_uuid(PLAYER_UID), 5)]);
        assert_eq!(meter.snapshot().total_damage, 5.0);
    }

    #[test]
    fn non_boss_death_does_not_end_encounter() {
        let mut meter = DpsMeter::default();
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![killing_hit(100)]);
        assert!(meter.pending_end.is_none());
    }

    #[test]
    fn idle_ends_encounter_once_and_empty_meter_sends_nothing() {
        let mut meter = DpsMeter::default();
        meter.poll(now_ms() + 60_000);
        assert!(meter.take_end_events().is_empty());

        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 100)]);
        meter.poll(now_ms() + 10_000);
        assert!(meter.take_end_events().is_empty());
        meter.poll(now_ms() + 30_000);
        let events = meter.take_end_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reason, EncounterEndReason::Idle);
        meter.poll(now_ms() + 60_000);
        assert!(meter.take_end_events().is_empty());
    }

    #[test]
    fn manual_reset_and_server_change_send_result_before_clearing() {
        let mut meter = DpsMeter::default();
        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 100)]);
        meter.reset_manually();
        let events = meter.take_end_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reason, EncounterEndReason::Manual);
        assert_eq!(events[0].result.total_damage, 100.0);

        feed(&mut meter, monster_uuid(MONSTER_UID), vec![hit(player_uuid(PLAYER_UID), 40)]);
        meter.reset_for_server_change();
        let events = meter.take_end_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].reason, EncounterEndReason::SceneChange);
        assert_eq!(events[0].result.total_damage, 40.0);
    }
}
