use byteorder::{BigEndian, ReadBytesExt};
use std::collections::BTreeMap;
use std::io::{self, Cursor, Read};

#[inline]
pub fn tcp_sequence_before(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

pub struct TCPReassembler {
    cache: BTreeMap<u32, Vec<u8>>,
    next_seq: Option<u32>,
    buffered_bytes: usize,
}

const MAX_TCP_CACHE_SIZE: usize = 5 * 1024 * 1024;

impl TCPReassembler {
    pub fn new() -> Self {
        Self {
            cache: BTreeMap::new(),
            next_seq: None,
            buffered_bytes: 0,
        }
    }

    // シーケンス番号のギャップがこの値を超えたら別接続とみなしてリセット
    const RESET_THRESHOLD: u32 = 100 * 1024 * 1024;

    pub fn insert_segment(&mut self, sequence_number: u32, payload: &[u8]) -> Option<Vec<u8>> {
        if payload.is_empty() {
            return None;
        }

        let expected = match self.next_seq {
            Some(seq) => seq,
            None => {
                self.next_seq = Some(sequence_number);
                sequence_number
            }
        };

        // next_seq との差が閾値を超えた場合は再接続とみなしてリセット（SYN が取れなかった保険）
        let gap = sequence_number.wrapping_sub(expected);
        if gap > Self::RESET_THRESHOLD && gap < u32::MAX - Self::RESET_THRESHOLD {
            eprintln!(
                "TCPReassembler: large seq gap detected (gap={}), resetting",
                gap
            );
            self.cache.clear();
            self.buffered_bytes = 0;
            self.next_seq = Some(sequence_number);
        }

        let mut start_seq = sequence_number;
        let mut data = payload;

        if tcp_sequence_before(start_seq, expected) {
            let overlap = expected.wrapping_sub(start_seq) as usize;
            if overlap >= data.len() {
                return None;
            }
            start_seq = expected;
            data = &data[overlap..];
        }

        match self.cache.get_mut(&start_seq) {
            Some(existing) => {
                if data.len() > existing.len() {
                    self.buffered_bytes -= existing.len();
                    existing.clear();
                    existing.extend_from_slice(data);
                    self.buffered_bytes += existing.len();
                }
            }
            None => {
                self.cache.insert(start_seq, data.to_vec());
                self.buffered_bytes += data.len();
            }
        }

        while self.buffered_bytes > MAX_TCP_CACHE_SIZE {
            let Some(first_cached_seq) = self.cache.keys().next().copied() else {
                break;
            };
            let Some(removed) = self.cache.remove(&first_cached_seq) else {
                break;
            };
            self.buffered_bytes = self.buffered_bytes.saturating_sub(removed.len());
        }

        if let Some(first_cached_seq) = self.cache.keys().next().copied() {
            self.next_seq = Some(first_cached_seq);
        }

        let mut cursor = self.next_seq.unwrap();
        let mut output: Vec<u8> = Vec::new();

        while let Some(mut segment) = self.cache.remove(&cursor) {
            self.buffered_bytes -= segment.len();
            cursor = cursor.wrapping_add(segment.len() as u32);
            if output.is_empty() {
                output = std::mem::take(&mut segment);
            } else {
                output.extend_from_slice(&segment);
            }
        }

        if output.is_empty() {
            None
        } else {
            self.next_seq = Some(cursor);
            Some(output)
        }
    }

    pub fn reset(&mut self, next_seq: Option<u32>) {
        self.cache.clear();
        self.buffered_bytes = 0;
        self.next_seq = next_seq;
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
