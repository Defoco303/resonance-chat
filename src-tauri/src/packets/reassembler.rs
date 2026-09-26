use crate::packets::diag::{self, Counter};

// フレームは [u32 長さ(自身を含む)][u16 種別][本体] の形。長さの妥当範囲。
// 実測の最大は約 168KB なので、上限を超える長さは区切りずれとみなす。
const MIN_FRAME_LEN: usize = 6;
const MAX_FRAME_LEN: usize = 2 * 1024 * 1024;
const MAX_BUFFER_SIZE: usize = 10 * 1024 * 1024;
const COMPACT_THRESHOLD: usize = 4096;

const FRAG_NOTIFY: u16 = 2;
const FRAG_FRAME_DOWN: u16 = 6;
const COMPRESSED_FLAG: u16 = 0x8000;
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
// Notify: 長さ(4) 種別(2) service_uuid(8) stub_id(4) method_id(4) の後に本体
const NOTIFY_HEADER_LEN: usize = 22;
// FrameDown: 長さ(4) 種別(2) seq(4) の後に本体（圧縮ならzstd、非圧縮なら入れ子のフレーム）
const FRAME_DOWN_HEADER_LEN: usize = 10;

enum Candidate {
    Valid,
    Invalid,
    NeedMore,
}

pub struct Reassembler {
    buffer: Vec<u8>,
    cursor: usize,
    label: String,
    // 区切りを見失っている（または接続の途中から見始めた）ので、正しいフレーム先頭を探す
    resyncing: bool,
    resync_reason: &'static str,
    resync_skipped: usize,
}

fn read_u32(buffer: &[u8], pos: usize) -> u32 {
    u32::from_be_bytes([buffer[pos], buffer[pos + 1], buffer[pos + 2], buffer[pos + 3]])
}

fn read_u16(buffer: &[u8], pos: usize) -> u16 {
    u16::from_be_bytes([buffer[pos], buffer[pos + 1]])
}

fn valid_frame_len(len: usize) -> bool {
    (MIN_FRAME_LEN..=MAX_FRAME_LEN).contains(&len)
}

impl Reassembler {
    pub fn with_label(label: String) -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
            cursor: 0,
            label,
            resyncing: true,
            resync_reason: "initial",
            resync_skipped: 0,
        }
    }

    pub fn try_next(&mut self) -> Option<Vec<u8>> {
        loop {
            if self.resyncing && !self.resync() {
                return None;
            }

            if self.available_len() < 4 {
                return None;
            }

            let frame_len = read_u32(&self.buffer, self.cursor) as usize;
            if !valid_frame_len(frame_len) {
                self.start_resync("invalid_length");
                continue;
            }

            if self.available_len() < frame_len {
                return None;
            }

            let start = self.cursor;
            let end = start + frame_len;
            diag::frame_type(read_u16(&self.buffer, start + 4) & !COMPRESSED_FLAG);
            let frame = self.buffer[start..end].to_vec();

            self.cursor = end;
            if self.cursor > COMPACT_THRESHOLD {
                self.compact();
            }

            return Some(frame);
        }
    }

    /// TCP 側で抜けを飛ばした場合は discontinuity=true で渡す。
    /// 組み立て途中のフレームは続きが来ないので捨て、区切り探しに入る。
    pub fn feed_owned(&mut self, bytes: Vec<u8>, discontinuity: bool) {
        if discontinuity {
            self.buffer.clear();
            self.cursor = 0;
            self.start_resync("tcp_gap");
        }

        if self.cursor > 0 {
            self.compact();
        }

        if self.buffer.len().saturating_add(bytes.len()) > MAX_BUFFER_SIZE {
            diag::add(Counter::FrameOverflowClears, 1);
            diag::net(format!(
                "frame_overflow_clear conn={} buffered={} incoming={}",
                self.label,
                self.buffer.len(),
                bytes.len()
            ));
            self.buffer.clear();
            self.cursor = 0;
            self.start_resync("overflow");
            if bytes.len() > MAX_BUFFER_SIZE {
                return;
            }
        }

        if self.buffer.is_empty() {
            self.buffer = bytes;
            return;
        }
        self.buffer.extend_from_slice(&bytes);
    }

    fn start_resync(&mut self, reason: &'static str) {
        if !self.resyncing {
            self.resyncing = true;
            self.resync_reason = reason;
            self.resync_skipped = 0;
        }
    }

    /// cursor から1バイトずつずらしてフレーム先頭を探す。見つかれば true。
    fn resync(&mut self) -> bool {
        let start = self.cursor;
        let mut pos = self.cursor;
        loop {
            match self.evaluate_candidate(pos) {
                Candidate::Valid => {
                    self.resync_skipped += pos - start;
                    self.cursor = pos;
                    self.resyncing = false;
                    if self.resync_reason != "initial" {
                        diag::add(Counter::FrameResyncs, 1);
                        diag::add(Counter::FrameResyncSkippedBytes, self.resync_skipped as u64);
                    }
                    diag::net(format!(
                        "frame_resync conn={} reason={} skipped_bytes={}",
                        self.label, self.resync_reason, self.resync_skipped
                    ));
                    return true;
                }
                Candidate::Invalid => pos += 1,
                Candidate::NeedMore => {
                    self.resync_skipped += pos - start;
                    self.cursor = pos;
                    self.compact();
                    return false;
                }
            }
        }
    }

    // 処理対象の Notify / FrameDown に限り、先頭らしさを厳しめに確認する。
    // さらに直後のフレームの長さも妥当か（読める範囲で）確かめて偶然の一致を避ける。
    fn evaluate_candidate(&self, pos: usize) -> Candidate {
        let buffer = &self.buffer;
        let available = buffer.len().saturating_sub(pos);
        if available < 6 {
            return Candidate::NeedMore;
        }

        let frame_len = read_u32(buffer, pos) as usize;
        if !valid_frame_len(frame_len) {
            return Candidate::Invalid;
        }
        let packet_type = read_u16(buffer, pos + 4);
        let is_compressed = (packet_type & COMPRESSED_FLAG) != 0;

        match packet_type & !COMPRESSED_FLAG {
            FRAG_FRAME_DOWN => {
                if frame_len < FRAME_DOWN_HEADER_LEN + 4 {
                    return Candidate::Invalid;
                }
                if available < FRAME_DOWN_HEADER_LEN + 4 {
                    return Candidate::NeedMore;
                }
                let body = pos + FRAME_DOWN_HEADER_LEN;
                if is_compressed {
                    if buffer[body..body + 4] != ZSTD_MAGIC {
                        return Candidate::Invalid;
                    }
                } else {
                    let nested_len = read_u32(buffer, body) as usize;
                    if nested_len < MIN_FRAME_LEN || nested_len > frame_len - FRAME_DOWN_HEADER_LEN {
                        return Candidate::Invalid;
                    }
                }
            }
            FRAG_NOTIFY => {
                if frame_len < NOTIFY_HEADER_LEN {
                    return Candidate::Invalid;
                }
                if available < NOTIFY_HEADER_LEN {
                    return Candidate::NeedMore;
                }
                // 既知の service_uuid はいずれも上位32bitが0
                if read_u32(buffer, pos + 6) != 0 {
                    return Candidate::Invalid;
                }
                if is_compressed {
                    if frame_len < NOTIFY_HEADER_LEN + 4 {
                        return Candidate::Invalid;
                    }
                    if available < NOTIFY_HEADER_LEN + 4 {
                        return Candidate::NeedMore;
                    }
                    let body = pos + NOTIFY_HEADER_LEN;
                    if buffer[body..body + 4] != ZSTD_MAGIC {
                        return Candidate::Invalid;
                    }
                }
            }
            _ => return Candidate::Invalid,
        }

        let next = pos + frame_len;
        if next + 4 <= buffer.len() && !valid_frame_len(read_u32(buffer, next) as usize) {
            return Candidate::Invalid;
        }
        Candidate::Valid
    }

    fn available_len(&self) -> usize {
        self.buffer.len().saturating_sub(self.cursor)
    }

    fn compact(&mut self) {
        if self.cursor == 0 {
            return;
        }
        if self.cursor >= self.buffer.len() {
            self.buffer.clear();
            self.cursor = 0;
            return;
        }
        let remaining = self.buffer.split_off(self.cursor);
        self.buffer = remaining;
        self.cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::Reassembler;

    fn frame(packet_type: u16, body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 6) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(&packet_type.to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    fn notify(method_id: u32, payload: &[u8]) -> Vec<u8> {
        let mut body = 0x6333_5342u64.to_be_bytes().to_vec();
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&method_id.to_be_bytes());
        body.extend_from_slice(payload);
        frame(2, &body)
    }

    fn frame_down_compressed(payload: &[u8]) -> Vec<u8> {
        let mut body = 7u32.to_be_bytes().to_vec();
        body.extend_from_slice(&[0x28, 0xb5, 0x2f, 0xfd]);
        body.extend_from_slice(payload);
        frame(0x8006, &body)
    }

    fn drain(reassembler: &mut Reassembler) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        while let Some(frame) = reassembler.try_next() {
            frames.push(frame);
        }
        frames
    }

    #[test]
    fn complete_frames_are_split() {
        let a = notify(0x2d, b"first");
        let b = frame_down_compressed(b"second");
        let mut reassembler = Reassembler::with_label(String::new());
        reassembler.feed_owned([a.clone(), b.clone()].concat(), false);
        assert_eq!(drain(&mut reassembler), vec![a, b]);
    }

    #[test]
    fn frame_split_across_feeds_is_joined() {
        let a = notify(0x2d, b"split-payload");
        let mut reassembler = Reassembler::with_label(String::new());
        reassembler.feed_owned(a[..10].to_vec(), false);
        assert!(drain(&mut reassembler).is_empty());
        reassembler.feed_owned(a[10..].to_vec(), false);
        assert_eq!(drain(&mut reassembler), vec![a]);
    }

    #[test]
    fn starting_mid_stream_finds_next_frame() {
        let a = notify(0x2d, b"aaaa");
        let b = notify(0x2e, b"bbbb");
        let mut stream = vec![0x13, 0x37, 0x00, 0x42, 0x99];
        stream.extend_from_slice(&a);
        stream.extend_from_slice(&b);
        let mut reassembler = Reassembler::with_label(String::new());
        reassembler.feed_owned(stream, false);
        assert_eq!(drain(&mut reassembler), vec![a, b]);
    }

    #[test]
    fn discontinuity_drops_partial_frame_and_resyncs() {
        let first = notify(0x2d, b"first");
        let lost = notify(0x2d, b"this frame loses its tail");
        let b = notify(0x2d, b"after-gap-1");
        let c = frame_down_compressed(b"after-gap-2");

        let mut reassembler = Reassembler::with_label(String::new());
        reassembler.feed_owned([first.clone(), lost[..12].to_vec()].concat(), false);
        assert_eq!(drain(&mut reassembler), vec![first]);

        // 抜けの後ろは lost の途中から始まる
        let after_gap = [lost[20..].to_vec(), b.clone(), c.clone()].concat();
        reassembler.feed_owned(after_gap, true);
        assert_eq!(drain(&mut reassembler), vec![b, c]);
    }

    #[test]
    fn invalid_length_triggers_resync() {
        let a = notify(0x2d, b"aaaa");
        let b = notify(0x2d, b"bbbb");
        let mut reassembler = Reassembler::with_label(String::new());
        reassembler.feed_owned(a.clone(), false);
        assert_eq!(drain(&mut reassembler), vec![a]);
        // 同期済みの状態で異常な長さが来たら、区切りを探し直して次のフレームから再開する
        reassembler.feed_owned([vec![0xff; 10], b.clone()].concat(), false);
        assert_eq!(drain(&mut reassembler), vec![b]);
    }
}
