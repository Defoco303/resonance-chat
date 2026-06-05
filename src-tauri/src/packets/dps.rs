use crate::protocol::constants::{attr_type, damage, entity};
use crate::protocol::pb;
use bytes::{Buf, Bytes};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const DPS_SERVICE_UUID: u64 = 0x63335342;

/// Bundled default boss list (monster_id -> display name), parsed from the
/// JSON shipped with the frontend. Used to seed the editable runtime list.
static DEFAULT_BOSS_NAMES: LazyLock<HashMap<u32, String>> = LazyLock::new(|| {
    let data = include_str!("../../../src/lib/data/json/MonsterNameBoss.json");
    let raw: HashMap<String, String> =
        serde_json::from_str(data).expect("invalid MonsterNameBoss.json");
    raw.into_iter()
        .filter_map(|(key, value)| key.parse::<u32>().ok().map(|id| (id, value)))
        .collect()
});

/// Runtime, user-editable boss list. Seeded from the bundled defaults and
/// mutated by the editor (set/reset) and by auto-detection of unknown bosses.
static BOSS_NAMES: LazyLock<RwLock<HashMap<u32, String>>> =
    LazyLock::new(|| RwLock::new(DEFAULT_BOSS_NAMES.clone()));

/// Monster names that should never be treated as a boss even if flagged.
static BOSS_EXCLUSION_NAMES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut set = HashSet::new();
    set.insert("divine defense tower".to_string());
    set
});

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BossEntry {
    pub id: u32,
    pub name: String,
}

fn boss_entries(map: &HashMap<u32, String>) -> Vec<BossEntry> {
    let mut entries: Vec<BossEntry> = map
        .iter()
        .map(|(&id, name)| BossEntry {
            id,
            name: name.clone(),
        })
        .collect();
    entries.sort_by_key(|entry| entry.id);
    entries
}

/// Return the current runtime boss list (sorted by id).
pub fn get_boss_list() -> Vec<BossEntry> {
    match BOSS_NAMES.read() {
        Ok(guard) => boss_entries(&guard),
        Err(_) => Vec::new(),
    }
}

/// Replace the entire runtime boss list with the supplied entries.
pub fn set_boss_list(entries: Vec<BossEntry>) {
    if let Ok(mut guard) = BOSS_NAMES.write() {
        guard.clear();
        for entry in entries {
            guard.insert(entry.id, entry.name);
        }
    }
}

/// Reset the runtime boss list back to the bundled defaults and return it.
pub fn reset_boss_list() -> Vec<BossEntry> {
    match BOSS_NAMES.write() {
        Ok(mut guard) => {
            *guard = DEFAULT_BOSS_NAMES.clone();
            boss_entries(&guard)
        }
        Err(_) => Vec::new(),
    }
}

fn is_boss_listed(monster_id: u32) -> bool {
    BOSS_NAMES
        .read()
        .map(|guard| guard.contains_key(&monster_id))
        .unwrap_or(false)
}

fn boss_name_from_list(monster_id: u32) -> Option<String> {
    BOSS_NAMES
        .read()
        .ok()
        .and_then(|guard| guard.get(&monster_id).cloned())
}


const SYNC_NEAR_ENTITIES: u32 = 0x00000006;
const SYNC_SCENE_ATTRS: u32 = 0x00000007;
const SYNC_CONTAINER_DATA: u32 = 0x00000015;
const SYNC_NEAR_DELTA_INFO: u32 = 0x0000002d;
const SYNC_TO_ME_DELTA_INFO: u32 = 0x0000002e;
const ENTER_SCENE: u32 = 0x00000003;
const EMIT_INTERVAL: Duration = Duration::from_millis(250);
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
    dmg_stats_boss_only: CombatStats,
    heal_stats: CombatStats,
    tank_stats: CombatStats,
    skill_uid_to_dps_stats: HashMap<i32, CombatStats>,
    name: Option<String>,
    class: Option<Class>,
    class_spec: Option<ClassSpec>,
    ability_score: Option<i32>,
    monster_id: Option<u32>,
    elite_status: Option<i64>,
    damage_taken_total: i64,
}

type TankHitKey = (i64, i32);

impl Default for Entity {
    fn default() -> Self {
        Self {
            entity_type: pb::EEntityType::EntErrType,
            dmg_stats: CombatStats::default(),
            dmg_stats_boss_only: CombatStats::default(),
            heal_stats: CombatStats::default(),
            tank_stats: CombatStats::default(),
            skill_uid_to_dps_stats: HashMap::new(),
            name: None,
            class: None,
            class_spec: None,
            ability_score: None,
            monster_id: None,
            elite_status: None,
            damage_taken_total: 0,
        }
    }
}

impl Entity {
    /// Boss classification is driven solely by the editable boss list so the
    /// user's ENEMY-tab toggle is fully authoritative and deterministic.
    fn is_boss(&self) -> bool {
        if self.entity_type != pb::EEntityType::EntMonster {
            return false;
        }
        if let Some(name) = &self.name {
            if BOSS_EXCLUSION_NAMES.contains(&name.to_lowercase()) {
                return false;
            }
        }
        matches!(self.monster_id, Some(id) if is_boss_listed(id))
    }
}

#[derive(Debug, Default)]
pub struct DpsMeter {
    entity_uid_to_entity: HashMap<i64, Entity>,
    total_dmg_stats: CombatStats,
    total_dmg_stats_boss_only: CombatStats,
    total_heal_stats: CombatStats,
    total_tank_stats: CombatStats,
    local_player_uid: Option<i64>,
    current_scene_id: Option<i32>,
    current_scene_guid: Option<String>,
    current_boss_name: Option<String>,
    time_fight_start_ms: u128,
    time_last_combat_packet_ms: u128,
    last_emit: Option<Instant>,
}

impl DpsMeter {
    pub fn process_packet(&mut self, method_id: u32, payload: &[u8]) -> bool {
        match method_id {
            ENTER_SCENE => {
                let Some(packet) = decode_packet::<pb::EnterScene>(payload) else {
                    return false;
                };
                self.process_enter_scene(packet)
            }
            SYNC_SCENE_ATTRS => {
                let Some(packet) = decode_packet::<pb::SyncSceneAttrs>(payload) else {
                    return false;
                };
                self.process_sync_scene_attrs(packet)
            }
            SYNC_NEAR_ENTITIES => {
                let Some(packet) = decode_packet::<pb::SyncNearEntities>(payload) else {
                    return false;
                };
                self.process_sync_near_entities(packet);
                false
            }
            SYNC_CONTAINER_DATA => {
                let Some(packet) = decode_packet::<pb::SyncContainerData>(payload) else {
                    return false;
                };
                self.process_sync_container_data(packet);
                false
            }
            SYNC_TO_ME_DELTA_INFO => {
                let Some(packet) = decode_packet::<pb::SyncToMeDeltaInfo>(payload) else {
                    return false;
                };
                self.process_sync_to_me_delta_info(packet)
            }
            SYNC_NEAR_DELTA_INFO => {
                let Some(packet) = decode_packet::<pb::SyncNearDeltaInfo>(payload) else {
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
        self.total_dmg_stats_boss_only = CombatStats::default();
        self.total_heal_stats = CombatStats::default();
        self.total_tank_stats = CombatStats::default();
        self.current_boss_name = None;
        self.time_fight_start_ms = 0;
        self.time_last_combat_packet_ms = 0;
        self.last_emit = None;
        for entity in self.entity_uid_to_entity.values_mut() {
            entity.dmg_stats = CombatStats::default();
            entity.dmg_stats_boss_only = CombatStats::default();
            entity.heal_stats = CombatStats::default();
            entity.tank_stats = CombatStats::default();
            entity.damage_taken_total = 0;
            entity.skill_uid_to_dps_stats.clear();
        }
    }

    pub fn reset_for_server_change(&mut self) {
        self.current_scene_id = None;
        self.current_scene_guid = None;
        self.reset_combat_state();
    }

    pub fn snapshot(&self) -> DpsMeterPayload {
        let elapsed_ms = self
            .time_last_combat_packet_ms
            .saturating_sub(self.time_fight_start_ms);
        let elapsed_secs = elapsed_ms as f64 / 1000.0;
        let dps = self.metric_snapshot(&self.total_dmg_stats, elapsed_secs, |entity| {
            &entity.dmg_stats
        });
        let dps_boss_only =
            self.metric_snapshot(&self.total_dmg_stats_boss_only, elapsed_secs, |entity| {
                &entity.dmg_stats_boss_only
            });
        let tank = self.tank_metric_snapshot(elapsed_secs);
        let heal = self.metric_snapshot(&self.total_heal_stats, elapsed_secs, |entity| {
            &entity.heal_stats
        });

        DpsMeterPayload {
            total_dps: dps.total_rate,
            total_damage: dps.total_value,
            elapsed_ms: elapsed_ms as f64,
            top_damage: dps.top_value,
            local_player_uid: self.local_player_uid.unwrap_or(-1) as f64,
            boss_engaged: self.total_dmg_stats_boss_only.value > 0,
            boss_name: self.current_boss_name.clone().unwrap_or_default(),
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
                    name: entity
                        .monster_id
                        .and_then(boss_name_from_list)
                        .or_else(|| entity.name.clone())
                        .unwrap_or_else(|| {
                            if id > 0 {
                                format!("Monster {id}")
                            } else {
                                "Monster (ID不明)".to_string()
                            }
                        }),
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

    fn metric_snapshot<F>(
        &self,
        total_stats: &CombatStats,
        elapsed_secs: f64,
        stats_for_entity: F,
    ) -> DpsMetricPayload
    where
        F: Fn(&Entity) -> &CombatStats,
    {
        let total_value = total_stats.value as f64;
        let mut players = Vec::new();
        let mut top_value: f64 = 0.0;

        for (&uid, entity) in &self.entity_uid_to_entity {
            let stats = stats_for_entity(entity);
            if entity.entity_type != pb::EEntityType::EntChar || stats.value <= 0 {
                continue;
            }

            let total = stats.value as f64;
            top_value = top_value.max(total);
            players.push(DpsPlayerRow {
                uid: uid as f64,
                name: entity.name.clone().unwrap_or_else(|| "Unknown".to_string()),
                class_name: class_name(entity.class.unwrap_or_default()),
                class_spec_name: class_spec_name(entity.class_spec.unwrap_or_default()),
                ability_score: entity.ability_score.unwrap_or(-1) as f64,
                total_damage: total,
                dps: finite_or_zero(total / elapsed_secs),
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
                target_entity.class = Some(Class::from(profession_list.cur_profession_id));
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
        self.process_aoi_sync_delta(base_delta, None)
    }

    fn process_sync_near_delta_info(&mut self, packet: pb::SyncNearDeltaInfo) -> bool {
        let tank_hit_target_counts = collect_tank_hit_target_counts(&packet.delta_infos);
        let mut changed = false;
        for delta in packet.delta_infos {
            changed |= self.process_aoi_sync_delta(delta, Some(&tank_hit_target_counts));
        }
        changed
    }

    fn process_aoi_sync_delta(
        &mut self,
        delta: pb::AoiSyncDelta,
        tank_hit_target_counts: Option<&HashMap<TankHitKey, usize>>,
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

        let (target_is_boss, target_boss_name) = self.evaluate_boss_target(target_uid);

        let mut changed = false;
        let mut boss_took_damage = false;
        for damage_info in skill_effect.damages {
            if damage_info.is_miss {
                continue;
            }

            let is_heal = damage_info.r#type == pb::EDamageType::Heal as i32;
            let attacker_uuid = if damage_info.top_summoner_id != 0 {
                damage_info.top_summoner_id
            } else if damage_info.attacker_uuid != 0 {
                damage_info.attacker_uuid
            } else {
                0
            };
            let skill_uid = damage_info.owner_id;

            if attacker_uuid != 0 && skill_uid != 0 {
                let attacker_uid = entity::get_player_uid(attacker_uuid);
                let attacker_entity_type = pb::EEntityType::from(attacker_uuid);
                let attacker_entity = self.entity_uid_to_entity.entry(attacker_uid).or_default();
                attacker_entity.entity_type = attacker_entity_type;
                infer_class_from_skill(attacker_entity, skill_uid);

                if is_heal {
                    process_stats(&damage_info, &mut attacker_entity.heal_stats);
                    process_stats(&damage_info, &mut self.total_heal_stats);
                } else {
                    let skill_stats = attacker_entity
                        .skill_uid_to_dps_stats
                        .entry(skill_uid)
                        .or_default();
                    process_stats(&damage_info, skill_stats);
                    process_stats(&damage_info, &mut attacker_entity.dmg_stats);
                    process_stats(&damage_info, &mut self.total_dmg_stats);
                    if target_is_boss {
                        process_stats(&damage_info, &mut attacker_entity.dmg_stats_boss_only);
                        process_stats(&damage_info, &mut self.total_dmg_stats_boss_only);
                        boss_took_damage = true;
                    }
                }
                changed = true;
            }

            if !is_heal && target_entity_type == pb::EEntityType::EntChar {
                let hp_loss = damage_info.hp_lessen_value.max(0);
                let shield_loss = damage_info.shield_lessen_value.max(0);
                let taken_value = hp_loss.saturating_add(shield_loss);
                if taken_value > 0 {
                    if let Some(tank_key) = tank_hit_key(&damage_info) {
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

            // Track damage dealt to monsters so the UI can offer them as
            // candidates to add to the boss list.
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
                if value > 0 {
                    let target_entity = self.entity_uid_to_entity.entry(target_uid).or_default();
                    target_entity.entity_type = target_entity_type;
                    target_entity.damage_taken_total =
                        target_entity.damage_taken_total.saturating_add(value);
                }
            }
        }

        if boss_took_damage {
            self.current_boss_name = Some(target_boss_name);
        }

        if changed {
            let timestamp_ms = now_ms();
            if self.time_fight_start_ms == 0 {
                self.time_fight_start_ms = timestamp_ms;
            }
            self.time_last_combat_packet_ms = timestamp_ms;
        }

        changed
    }

    fn evaluate_boss_target(&self, target_uid: i64) -> (bool, String) {
        let Some(entity) = self.entity_uid_to_entity.get(&target_uid) else {
            return (false, String::new());
        };
        if !entity.is_boss() {
            return (false, String::new());
        }
        let name = entity
            .monster_id
            .and_then(boss_name_from_list)
            .or_else(|| entity.name.clone())
            .unwrap_or_else(|| "Unknown Boss".to_string());
        (true, name)
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
                        target_entity.class = Some(Class::from(class_id));
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

fn decode_packet<T: Message + Default>(payload: &[u8]) -> Option<T> {
    T::decode(Bytes::copy_from_slice(payload)).ok()
}

fn decode_protobuf_int32(data: &[u8]) -> Result<i32, prost::DecodeError> {
    let mut cursor = Cursor::new(data);
    prost::encoding::decode_varint(&mut cursor).map(|v| v as i32)
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

fn process_stats(damage_info: &pb::SyncDamageInfo, stats: &mut CombatStats) {
    let actual_value = if damage_info.lucky_value != 0 {
        damage_info.lucky_value
    } else {
        damage_info.value
    };
    process_stats_value(damage_info, actual_value, stats);
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

fn infer_class_from_skill(entity: &mut Entity, skill_uid: i32) {
    if entity.class_spec.is_none_or(|spec| spec == ClassSpec::Unknown) {
        let spec = ClassSpec::from_skill_id(skill_uid);
        entity.class_spec = Some(spec);
        if entity
            .class
            .is_none_or(|class| matches!(class, Class::Unknown | Class::Unimplemented))
        {
            entity.class = Some(class_from_spec(spec));
        }
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
    Unknown,
}

impl Default for ClassSpec {
    fn default() -> Self {
        Self::Unknown
    }
}

impl ClassSpec {
    fn from_skill_id(skill_id: i32) -> Self {
        match skill_id {
            1714 | 1734 => ClassSpec::Iaido,
            44701 | 179906 => ClassSpec::Moonstrike,
            120901 | 120902 => ClassSpec::Icicle,
            1241 => ClassSpec::Frostbeam,
            1405 | 1418 => ClassSpec::Vanguard,
            1419 => ClassSpec::Skyward,
            1518 | 1541 | 21402 => ClassSpec::Smite,
            20301 => ClassSpec::Lifebind,
            199902 => ClassSpec::Earthfort,
            1930 | 1931 | 1934 | 1935 => ClassSpec::Block,
            220112 | 2203622 => ClassSpec::Falconry,
            2292 | 1700820 | 1700825 | 1700827 => ClassSpec::Wildpack,
            2405 => ClassSpec::Recovery,
            2406 => ClassSpec::Shield,
            2306 => ClassSpec::Dissonance,
            2307 | 2361 | 55302 => ClassSpec::Concerto,
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
        ClassSpec::Unknown => "Unknown Spec",
    }
    .to_string()
}
