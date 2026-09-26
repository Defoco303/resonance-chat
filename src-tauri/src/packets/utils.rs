use crate::packets::diag::{self, Counter};
use byteorder::{BigEndian, ReadBytesExt};
use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Cursor, Read};
use std::time::{Duration, Instant};

// 抜けを待つ時間。実測では遅れて届いたセグメントは 8〜235ms で到着した
const GAP_WAIT_TIMEOUT: Duration = Duration::from_millis(500);
// 抜けの後ろに保留できる量。超えたら待たずに抜けを飛ばす
const MAX_TCP_CACHE_SIZE: usize = 2 * 1024 * 1024;
// シーケンス番号の差がこれを超えたら別接続とみなしてリセット（SYN が取れなかった保険）
const RESET_THRESHOLD: u32 = 100 * 1024 * 1024;
const MAX_SKIPPED_RANGES: usize = 64;
// 32bit のシーケンス番号を 64bit に拡張して一周（ラップアラウンド）を気にせず比較する。
// 開始位置より前の再送が負にならないよう 2^32 から始める。
const EXTENDED_SEQ_BASE: u64 = 1 << 32;

pub struct TcpOutput {
    pub data: Vec<u8>,
    /// data の直前でストリームが途切れている（抜けを飛ばした、またはリセットした）
    pub discontinuity: bool,
}

struct PendingGap {
    at: u64,
    since: Instant,
}

pub struct TCPReassembler {
    // 拡張シーケンス番号 -> ペイロード（next より後ろで、まだ連続していないもの）
    cache: BTreeMap<u64, Vec<u8>>,
    next: Option<u64>,
    buffered_bytes: usize,
    pending_gap: Option<PendingGap>,
    discontinuity: bool,
    label: String,
    // 診断用: 飛ばした範囲 [start, end)
    skipped_ranges: VecDeque<(u64, u64)>,
}

impl TCPReassembler {
    pub fn with_label(label: String) -> Self {
        Self {
            cache: BTreeMap::new(),
            next: None,
            buffered_bytes: 0,
            pending_gap: None,
            discontinuity: false,
            label,
            skipped_ranges: VecDeque::new(),
        }
    }

    fn extend_seq(next: u64, sequence_number: u32) -> u64 {
        let delta = sequence_number.wrapping_sub(next as u32) as i32 as i64;
        (next as i64 + delta) as u64
    }

    /// セグメントを取り込み、新たに連続した部分を返す。
    /// 抜けがあれば最大 GAP_WAIT_TIMEOUT 待ち、それでも埋まらない（または保留が
    /// MAX_TCP_CACHE_SIZE を超えた）ときだけ飛ばして discontinuity を立てる。
    pub fn insert_segment(
        &mut self,
        sequence_number: u32,
        payload: &[u8],
        now: Instant,
    ) -> Vec<TcpOutput> {
        if payload.is_empty() {
            return Vec::new();
        }

        let next = match self.next {
            Some(next) => {
                let gap = sequence_number.wrapping_sub(next as u32);
                if gap > RESET_THRESHOLD && gap < u32::MAX - RESET_THRESHOLD {
                    eprintln!("TCPReassembler: large seq gap detected (gap={gap}), resetting");
                    diag::add(Counter::TcpLargeGapResets, 1);
                    diag::net(format!(
                        "tcp_large_gap_reset conn={} expected={} seq={sequence_number} gap={gap}",
                        self.label, next as u32
                    ));
                    self.reset(Some(sequence_number));
                    self.discontinuity = true;
                    EXTENDED_SEQ_BASE + sequence_number as u64
                } else {
                    next
                }
            }
            None => {
                let next = EXTENDED_SEQ_BASE + sequence_number as u64;
                self.next = Some(next);
                next
            }
        };

        let start = Self::extend_seq(next, sequence_number);
        let end = start + payload.len() as u64;
        if end <= next {
            self.record_discarded(start, end);
            return self.drain(now);
        }

        let (start, data) = if start < next {
            self.record_discarded(start, next);
            (next, &payload[(next - start) as usize..])
        } else {
            (start, payload)
        };

        match self.cache.get_mut(&start) {
            Some(existing) => {
                if data.len() > existing.len() {
                    self.buffered_bytes -= existing.len();
                    existing.clear();
                    existing.extend_from_slice(data);
                    self.buffered_bytes += existing.len();
                }
            }
            None => {
                self.cache.insert(start, data.to_vec());
                self.buffered_bytes += data.len();
            }
        }

        self.drain(now)
    }

    fn drain(&mut self, now: Instant) -> Vec<TcpOutput> {
        let Some(mut cursor) = self.next else {
            return Vec::new();
        };
        let mut outputs = Vec::new();
        let mut output: Vec<u8> = Vec::new();

        loop {
            // cursor から連続している部分を取り出す（区切りの違う再送の重なりは切り落とす）
            while let Some((&key, _)) = self.cache.first_key_value() {
                if key > cursor {
                    break;
                }
                let Some(segment) = self.cache.remove(&key) else {
                    break;
                };
                self.buffered_bytes -= segment.len();
                let segment_end = key + segment.len() as u64;
                if segment_end <= cursor {
                    continue;
                }
                output.extend_from_slice(&segment[(cursor - key) as usize..]);
                cursor = segment_end;
            }

            if let Some(gap) = &self.pending_gap {
                if gap.at < cursor {
                    let waited = now.saturating_duration_since(gap.since);
                    diag::add(Counter::TcpGapFilled, 1);
                    diag::net(format!(
                        "tcp_gap_filled conn={} seq={} wait_ms={}",
                        self.label,
                        gap.at as u32,
                        waited.as_millis()
                    ));
                    self.pending_gap = None;
                }
            }

            let Some((&first_cached, _)) = self.cache.first_key_value() else {
                self.pending_gap = None;
                break;
            };

            // cursor..first_cached が抜けている
            let since = match &self.pending_gap {
                Some(gap) if gap.at == cursor => gap.since,
                _ => {
                    diag::add(Counter::TcpGapWaits, 1);
                    diag::net(format!(
                        "tcp_gap_wait conn={} seq={} gap_bytes={}",
                        self.label,
                        cursor as u32,
                        first_cached - cursor
                    ));
                    self.pending_gap = Some(PendingGap { at: cursor, since: now });
                    now
                }
            };
            let waited = now.saturating_duration_since(since);
            let overflow = self.buffered_bytes > MAX_TCP_CACHE_SIZE;
            if !overflow && waited < GAP_WAIT_TIMEOUT {
                break;
            }

            self.record_gap_skip(cursor, first_cached, waited, overflow);
            self.pending_gap = None;
            if !output.is_empty() {
                outputs.push(TcpOutput {
                    data: std::mem::take(&mut output),
                    discontinuity: std::mem::take(&mut self.discontinuity),
                });
            }
            self.discontinuity = true;
            cursor = first_cached;
        }

        self.next = Some(cursor);
        if !output.is_empty() {
            outputs.push(TcpOutput {
                data: output,
                discontinuity: std::mem::take(&mut self.discontinuity),
            });
        }
        outputs
    }

    fn record_gap_skip(&mut self, from: u64, to: u64, waited: Duration, overflow: bool) {
        let gap = to - from;
        if overflow {
            diag::add(Counter::TcpGapOverflowSkips, 1);
        } else {
            diag::add(Counter::TcpGapTimeoutSkips, 1);
        }
        diag::add(Counter::TcpGapSkipBytes, gap);
        diag::net(format!(
            "tcp_gap_skip conn={} reason={} from_seq={} to_seq={} gap_bytes={gap} wait_ms={} buffered={}",
            self.label,
            if overflow { "overflow" } else { "timeout" },
            from as u32,
            to as u32,
            waited.as_millis(),
            self.buffered_bytes
        ));
        if self.skipped_ranges.len() >= MAX_SKIPPED_RANGES {
            self.skipped_ranges.pop_front();
        }
        self.skipped_ranges.push_back((from, to));
    }

    // next より前にあって捨てる部分 [start, end) が、飛ばした範囲と重なるか（診断用）
    fn record_discarded(&self, start: u64, end: u64) {
        let lost_to_skip = self
            .skipped_ranges
            .iter()
            .any(|&(skip_start, skip_end)| start < skip_end && skip_start < end);
        if lost_to_skip {
            diag::add(Counter::TcpLateAfterSkip, 1);
            diag::add(Counter::TcpLateAfterSkipBytes, end - start);
            diag::net(format!(
                "tcp_late_after_skip conn={} seq={} len={}",
                self.label,
                start as u32,
                end - start
            ));
        } else {
            diag::add(Counter::TcpDuplicateDiscards, 1);
        }
    }

    pub fn reset(&mut self, next_seq: Option<u32>) {
        self.cache.clear();
        self.buffered_bytes = 0;
        self.next = next_seq.map(|seq| EXTENDED_SEQ_BASE + seq as u64);
        self.pending_gap = None;
        self.discontinuity = false;
        self.skipped_ranges.clear();
    }
}

pub struct BinaryReader {
    pub cursor: Cursor<Vec<u8>>,
}

impl BinaryReader {
    pub fn from(data: Vec<u8>) -> Self {
        Self {
            cursor: Cursor::new(data),
        }
    }

    pub fn read_u16(&mut self) -> io::Result<u16> {
        self.cursor.read_u16::<BigEndian>()
    }

    pub fn read_u32(&mut self) -> io::Result<u32> {
        self.cursor.read_u32::<BigEndian>()
    }

    pub fn peek_u32(&mut self) -> io::Result<u32> {
        let pos = self.cursor.position();
        let value = self.cursor.read_u32::<BigEndian>()?;
        self.cursor.set_position(pos);
        Ok(value)
    }

    pub fn read_u64(&mut self) -> io::Result<u64> {
        self.cursor.read_u64::<BigEndian>()
    }

    pub fn read_bytes(&mut self, count: usize) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0u8; count];
        self.cursor.read_exact(&mut buffer)?;
        Ok(buffer)
    }

    pub fn read_remaining(&mut self) -> &[u8] {
        let pos = self.cursor.position() as usize;
        let buf = self.cursor.get_ref();
        &buf[pos..]
    }

    pub fn remaining(&self) -> usize {
        let total_len = self.cursor.get_ref().len() as u64;
        let current_pos = self.cursor.position();
        (total_len.saturating_sub(current_pos)) as usize
    }
}


#[cfg(test)]
mod tests {
    use super::{TCPReassembler, TcpOutput};
    use std::time::{Duration, Instant};

    fn joined(outputs: &[TcpOutput]) -> Vec<u8> {
        outputs.iter().flat_map(|output| output.data.clone()).collect()
    }

    #[test]
    fn in_order_segments_are_joined() {
        let now = Instant::now();
        let mut reassembler = TCPReassembler::with_label(String::new());
        let first = reassembler.insert_segment(1000, b"abc", now);
        assert_eq!(joined(&first), b"abc");
        assert!(!first[0].discontinuity);
        assert_eq!(joined(&reassembler.insert_segment(1003, b"def", now)), b"def");
    }

    #[test]
    fn out_of_order_segment_waits_for_missing_data() {
        let now = Instant::now();
        let mut reassembler = TCPReassembler::with_label(String::new());
        assert_eq!(joined(&reassembler.insert_segment(1000, b"aaa", now)), b"aaa");
        // 1003..1006 が抜けた状態で 1006 が先に届く -> 待つ
        assert!(reassembler.insert_segment(1006, b"ccc", now).is_empty());
        // 遅れて 1003 が届くと、つながって出てくる
        let outputs = reassembler.insert_segment(1003, b"bbb", now + Duration::from_millis(235));
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].data, b"bbbccc");
        assert!(!outputs[0].discontinuity);
    }

    #[test]
    fn gap_is_skipped_after_timeout_with_discontinuity() {
        let now = Instant::now();
        let mut reassembler = TCPReassembler::with_label(String::new());
        reassembler.insert_segment(1000, b"aaa", now);
        assert!(reassembler.insert_segment(1006, b"ccc", now).is_empty());
        assert!(reassembler
            .insert_segment(1009, b"ddd", now + Duration::from_millis(499))
            .is_empty());

        let outputs = reassembler.insert_segment(1012, b"eee", now + Duration::from_millis(500));
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].data, b"cccdddeee");
        assert!(outputs[0].discontinuity);

        // 飛ばした後に届いた欠落分は捨てる
        assert!(reassembler
            .insert_segment(1003, b"bbb", now + Duration::from_millis(600))
            .is_empty());
        let next = reassembler.insert_segment(1015, b"fff", now + Duration::from_millis(600));
        assert_eq!(next[0].data, b"fff");
        assert!(!next[0].discontinuity);
    }

    #[test]
    fn gap_is_skipped_when_buffer_exceeds_limit() {
        let now = Instant::now();
        let mut reassembler = TCPReassembler::with_label(String::new());
        reassembler.insert_segment(1000, b"a", now);
        let large = vec![7u8; 2 * 1024 * 1024 + 1];
        let outputs = reassembler.insert_segment(1002, &large, now);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].data.len(), large.len());
        assert!(outputs[0].discontinuity);
    }

    #[test]
    fn retransmit_with_different_boundaries_does_not_stall() {
        let now = Instant::now();
        let mut reassembler = TCPReassembler::with_label(String::new());
        assert_eq!(joined(&reassembler.insert_segment(1000, b"abcd", now)), b"abcd");
        assert!(reassembler.insert_segment(1008, b"ijkl", now).is_empty());
        // 1002..1010 の再送（既に受け取った 1002..1004 を含み、保留中の 1008.. と重なる）
        let outputs = reassembler.insert_segment(1002, b"cdefghij", now);
        assert_eq!(joined(&outputs), b"efghijkl");
        assert_eq!(joined(&reassembler.insert_segment(1012, b"mn", now)), b"mn");
    }

    #[test]
    fn sequence_wraparound_is_handled() {
        let now = Instant::now();
        let mut reassembler = TCPReassembler::with_label(String::new());
        assert_eq!(
            joined(&reassembler.insert_segment(u32::MAX - 2, b"abc", now)),
            b"abc"
        );
        assert!(reassembler.insert_segment(3, b"ghi", now).is_empty());
        assert_eq!(joined(&reassembler.insert_segment(0, b"def", now)), b"defghi");
    }

    #[test]
    fn duplicate_segment_is_ignored() {
        let now = Instant::now();
        let mut reassembler = TCPReassembler::with_label(String::new());
        reassembler.insert_segment(1000, b"abc", now);
        assert!(reassembler.insert_segment(1000, b"abc", now).is_empty());
        assert_eq!(joined(&reassembler.insert_segment(1003, b"d", now)), b"d");
    }
}
