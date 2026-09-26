//! Diagnostic logging for investigating missed DPS data.
//!
//! Records are handed to a background writer thread through a bounded channel so the
//! capture thread never blocks on disk I/O. Output goes to
//! `<app_log_dir>/diag-<UTC start time>/`:
//! - `net.log`        : TCP / frame / decode events and a counter summary every 10s
//! - `damage.csv`     : one row per SyncDamageInfo with the meter's decision
//! - `combat_raw.bin` : raw (decompressed) combat notify payloads for offline replay

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CHANNEL_CAPACITY: usize = 65_536;
const MAX_NET_LOG_BYTES: u64 = 20 * 1024 * 1024;
const MAX_DAMAGE_ROWS: u64 = 200_000;
const MAX_RAW_BYTES: u64 = 200 * 1024 * 1024;
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);
const SUMMARY_INTERVAL: Duration = Duration::from_secs(10);
const RAW_MAGIC: &[u8] = b"RCDIAG1\n";

pub const DAMAGE_CSV_HEADER: &str = "ts_ms,source,target_uuid,target_type,attacker_uuid,top_summoner_id,attacker_type,owner_id,type,type_flag,damage_source,is_crit,is_miss,is_dead,value,lucky_value,actual_value,hp_lessen,shield_lessen,hit_event_id,property,decision,counted_value,target_is_boss";

#[derive(Clone, Copy)]
pub enum Counter {
    CaptureOversizeDrops,
    TcpGapWaits,
    TcpGapFilled,
    TcpGapTimeoutSkips,
    TcpGapOverflowSkips,
    TcpGapSkipBytes,
    TcpLateAfterSkip,
    TcpLateAfterSkipBytes,
    TcpDuplicateDiscards,
    TcpLargeGapResets,
    FrameResyncs,
    FrameResyncSkippedBytes,
    FrameOverflowClears,
    FrameDownTrailing,
    DecompressFailures,
    OversizeDrops,
    DamageRows,
    RawRecords,
    ChannelDrops,
}

const COUNTER_COUNT: usize = 19;

impl Counter {
    const ALL: [Counter; COUNTER_COUNT] = [
        Counter::CaptureOversizeDrops,
        Counter::TcpGapWaits,
        Counter::TcpGapFilled,
        Counter::TcpGapTimeoutSkips,
        Counter::TcpGapOverflowSkips,
        Counter::TcpGapSkipBytes,
        Counter::TcpLateAfterSkip,
        Counter::TcpLateAfterSkipBytes,
        Counter::TcpDuplicateDiscards,
        Counter::TcpLargeGapResets,
        Counter::FrameResyncs,
        Counter::FrameResyncSkippedBytes,
        Counter::FrameOverflowClears,
        Counter::FrameDownTrailing,
        Counter::DecompressFailures,
        Counter::OversizeDrops,
        Counter::DamageRows,
        Counter::RawRecords,
        Counter::ChannelDrops,
    ];

    fn name(self) -> &'static str {
        match self {
            Counter::CaptureOversizeDrops => "capture_oversize_drops",
            Counter::TcpGapWaits => "tcp_gap_waits",
            Counter::TcpGapFilled => "tcp_gap_filled",
            Counter::TcpGapTimeoutSkips => "tcp_gap_timeout_skips",
            Counter::TcpGapOverflowSkips => "tcp_gap_overflow_skips",
            Counter::TcpGapSkipBytes => "tcp_gap_skip_bytes",
            Counter::TcpLateAfterSkip => "tcp_late_after_skip",
            Counter::TcpLateAfterSkipBytes => "tcp_late_after_skip_bytes",
            Counter::TcpDuplicateDiscards => "tcp_duplicate_discards",
            Counter::TcpLargeGapResets => "tcp_large_gap_resets",
            Counter::FrameResyncs => "frame_resyncs",
            Counter::FrameResyncSkippedBytes => "frame_resync_skipped_bytes",
            Counter::FrameOverflowClears => "frame_overflow_clears",
            Counter::FrameDownTrailing => "frame_down_trailing",
            Counter::DecompressFailures => "decompress_failures",
            Counter::OversizeDrops => "oversize_drops",
            Counter::DamageRows => "damage_rows",
            Counter::RawRecords => "raw_records",
            Counter::ChannelDrops => "channel_drops",
        }
    }
}

// 最上位フレームの種別ごとの件数（0..=15、それ以外は末尾にまとめる）
const FRAME_TYPE_SLOTS: usize = 17;

static COUNTERS: [AtomicU64; COUNTER_COUNT] = [const { AtomicU64::new(0) }; COUNTER_COUNT];
static FRAME_TYPES: [AtomicU64; FRAME_TYPE_SLOTS] = [const { AtomicU64::new(0) }; FRAME_TYPE_SLOTS];
static DECODE_FAILURES: OnceLock<Mutex<HashMap<u32, u64>>> = OnceLock::new();
static SENDER: OnceLock<SyncSender<Record>> = OnceLock::new();
static ENABLED: AtomicBool = AtomicBool::new(false);

enum Record {
    Net(String),
    Damage(String),
    Raw {
        ts_ms: u64,
        method_id: u32,
        payload: Vec<u8>,
    },
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

#[inline]
pub fn add(counter: Counter, value: u64) {
    COUNTERS[counter as usize].fetch_add(value, Ordering::Relaxed);
}

#[inline]
pub fn frame_type(msg_type_id: u16) {
    let slot = (msg_type_id as usize).min(FRAME_TYPE_SLOTS - 1);
    FRAME_TYPES[slot].fetch_add(1, Ordering::Relaxed);
}

fn send(record: Record) {
    let Some(sender) = SENDER.get() else {
        return;
    };
    if sender.try_send(record).is_err() {
        add(Counter::ChannelDrops, 1);
    }
}

/// Writes one event line to `net.log` (timestamp is prepended).
pub fn net(line: String) {
    if !enabled() {
        return;
    }
    send(Record::Net(format!("{} {}", now_ms(), line)));
}

/// Writes one pre-formatted row to `damage.csv`.
pub fn damage_row(row: String) {
    if !enabled() {
        return;
    }
    add(Counter::DamageRows, 1);
    send(Record::Damage(row));
}

/// Stores one decompressed combat notify payload in `combat_raw.bin`.
pub fn raw(method_id: u32, payload: &[u8]) {
    if !enabled() {
        return;
    }
    add(Counter::RawRecords, 1);
    send(Record::Raw {
        ts_ms: now_ms() as u64,
        method_id,
        payload: payload.to_vec(),
    });
}

pub fn decode_failure(method_id: u32, payload_len: usize) {
    if let Ok(mut map) = DECODE_FAILURES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        *map.entry(method_id).or_insert(0) += 1;
    }
    net(format!(
        "decode_failure method={method_id:#x} payload_len={payload_len}"
    ));
}

// 残しておく調査用ログの数（新しいものから）
const KEEP_SESSIONS: usize = 5;
const SETTING_FILE: &str = "diag_enabled";

static LOG_ROOT: OnceLock<PathBuf> = OnceLock::new();
static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();
static START_LOCK: Mutex<()> = Mutex::new(());

/// 調査用ログの保存先と設定の置き場所を覚える。保存は設定がオンのときだけ始める。
pub fn init(log_root: Option<PathBuf>, config_dir: Option<PathBuf>) {
    if let Some(log_root) = log_root {
        let _ = LOG_ROOT.set(log_root);
    }
    if let Some(config_dir) = config_dir {
        let _ = CONFIG_DIR.set(config_dir);
    }
    if saved_setting() {
        set_enabled(true);
    }
}

/// 設定画面の「調査用ログを保存」の状態
pub fn saved_setting() -> bool {
    CONFIG_DIR
        .get()
        .map(|dir| dir.join(SETTING_FILE).exists())
        .unwrap_or(false)
}

/// オン・オフを切り替えて設定を保存する。オンにした最初の1回だけ、ログのフォルダを作る。
pub fn set_enabled(on: bool) -> bool {
    if let Some(dir) = CONFIG_DIR.get() {
        let path = dir.join(SETTING_FILE);
        if on {
            let _ = fs::create_dir_all(dir);
            let _ = File::create(&path);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
    if !on {
        ENABLED.store(false, Ordering::Relaxed);
        return false;
    }
    let _guard = START_LOCK.lock();
    if SENDER.get().is_none() && !start_session() {
        return false;
    }
    ENABLED.store(true, Ordering::Relaxed);
    true
}

fn start_session() -> bool {
    let Some(log_root) = LOG_ROOT.get() else {
        eprintln!("[diag] log directory unavailable; diagnostics disabled");
        return false;
    };
    prune_old_sessions(log_root, KEEP_SESSIONS - 1);
    let session_dir = log_root.join(format!("diag-{}", utc_stamp(now_ms() / 1000)));
    if let Err(err) = fs::create_dir_all(&session_dir) {
        eprintln!("[diag] failed to create {}: {err}", session_dir.display());
        return false;
    }

    let writers = match Writers::open(&session_dir) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("[diag] failed to open log files: {err}");
            return false;
        }
    };

    let (sender, receiver) = sync_channel(CHANNEL_CAPACITY);
    if SENDER.set(sender).is_err() {
        return false;
    }
    std::thread::spawn(move || writer_loop(receiver, writers));
    eprintln!("[diag] writing diagnostics to {}", session_dir.display());
    true
}

/// diag-* フォルダを新しい順に keep 個だけ残して消す（名前が UTC 時刻なので名前順＝時刻順）
fn prune_old_sessions(log_root: &PathBuf, keep: usize) {
    let Ok(entries) = fs::read_dir(log_root) else {
        return;
    };
    let mut sessions: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("diag-"))
        })
        .collect();
    sessions.sort();
    let remove_count = sessions.len().saturating_sub(keep);
    for path in sessions.into_iter().take(remove_count) {
        let _ = fs::remove_dir_all(path);
    }
}

struct Writers {
    net: BufWriter<File>,
    net_bytes: u64,
    net_capped: bool,
    damage: BufWriter<File>,
    damage_rows: u64,
    damage_capped: bool,
    raw: BufWriter<File>,
    raw_bytes: u64,
    raw_capped: bool,
}

impl Writers {
    fn open(dir: &PathBuf) -> std::io::Result<Self> {
        let mut net = BufWriter::new(File::create(dir.join("net.log"))?);
        let mut damage = BufWriter::new(File::create(dir.join("damage.csv"))?);
        let mut raw = BufWriter::new(File::create(dir.join("combat_raw.bin"))?);
        let header = format!(
            "{} session_start version={}\n",
            now_ms(),
            env!("CARGO_PKG_VERSION")
        );
        net.write_all(header.as_bytes())?;
        damage.write_all(DAMAGE_CSV_HEADER.as_bytes())?;
        damage.write_all(b"\n")?;
        raw.write_all(RAW_MAGIC)?;
        Ok(Self {
            net,
            net_bytes: header.len() as u64,
            net_capped: false,
            damage,
            damage_rows: 0,
            damage_capped: false,
            raw,
            raw_bytes: RAW_MAGIC.len() as u64,
            raw_capped: false,
        })
    }

    fn write_net(&mut self, line: &str) {
        if self.net_capped {
            return;
        }
        if self.net_bytes + line.len() as u64 + 1 > MAX_NET_LOG_BYTES {
            self.net_capped = true;
            let _ = writeln!(self.net, "{} net_log_cap_reached", now_ms());
            return;
        }
        self.net_bytes += line.len() as u64 + 1;
        let _ = writeln!(self.net, "{line}");
    }

    fn write_damage(&mut self, row: &str) {
        if self.damage_capped {
            return;
        }
        if self.damage_rows >= MAX_DAMAGE_ROWS {
            self.damage_capped = true;
            self.write_net(&format!("{} damage_csv_cap_reached", now_ms()));
            return;
        }
        self.damage_rows += 1;
        let _ = writeln!(self.damage, "{row}");
    }

    fn write_raw(&mut self, ts_ms: u64, method_id: u32, payload: &[u8]) {
        if self.raw_capped {
            return;
        }
        let record_len = 16 + payload.len() as u64;
        if self.raw_bytes + record_len > MAX_RAW_BYTES {
            self.raw_capped = true;
            self.write_net(&format!("{} combat_raw_cap_reached", now_ms()));
            return;
        }
        self.raw_bytes += record_len;
        let _ = self.raw.write_all(&ts_ms.to_le_bytes());
        let _ = self.raw.write_all(&method_id.to_le_bytes());
        let _ = self.raw.write_all(&(payload.len() as u32).to_le_bytes());
        let _ = self.raw.write_all(payload);
    }

    fn flush(&mut self) {
        let _ = self.net.flush();
        let _ = self.damage.flush();
        let _ = self.raw.flush();
    }
}

fn writer_loop(receiver: Receiver<Record>, mut writers: Writers) {
    let mut last_flush = Instant::now();
    let mut last_summary = Instant::now();

    loop {
        match receiver.recv_timeout(FLUSH_INTERVAL) {
            Ok(Record::Net(line)) => writers.write_net(&line),
            Ok(Record::Damage(row)) => writers.write_damage(&row),
            Ok(Record::Raw {
                ts_ms,
                method_id,
                payload,
            }) => writers.write_raw(ts_ms, method_id, &payload),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                writers.flush();
                return;
            }
        }

        if last_summary.elapsed() >= SUMMARY_INTERVAL {
            last_summary = Instant::now();
            writers.write_net(&summary_line());
        }
        if last_flush.elapsed() >= FLUSH_INTERVAL {
            last_flush = Instant::now();
            writers.flush();
        }
    }
}

fn summary_line() -> String {
    let mut line = format!("{} summary", now_ms());
    for counter in Counter::ALL {
        line.push_str(&format!(
            " {}={}",
            counter.name(),
            COUNTERS[counter as usize].load(Ordering::Relaxed)
        ));
    }
    for (slot, count) in FRAME_TYPES.iter().enumerate() {
        let count = count.load(Ordering::Relaxed);
        if count == 0 {
            continue;
        }
        if slot == FRAME_TYPE_SLOTS - 1 {
            line.push_str(&format!(" frame_type_other={count}"));
        } else {
            line.push_str(&format!(" frame_type_{slot}={count}"));
        }
    }
    if let Some(map) = DECODE_FAILURES.get() {
        if let Ok(map) = map.lock() {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort();
            for (method_id, count) in entries {
                line.push_str(&format!(" decode_failures_{method_id:#x}={count}"));
            }
        }
    }
    line
}

/// Formats unix seconds as `YYYYMMDDTHHMMSSZ` (UTC).
fn utc_stamp(unix_secs: u128) -> String {
    let secs = unix_secs as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::utc_stamp;

    #[test]
    fn utc_stamp_formats_known_instants() {
        assert_eq!(utc_stamp(0), "19700101T000000Z");
        assert_eq!(utc_stamp(1_709_210_096), "20240229T123456Z");
    }
}
