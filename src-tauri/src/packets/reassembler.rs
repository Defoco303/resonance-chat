use std::convert::TryInto;

pub struct Reassembler {
    buffer: Vec<u8>,
    cursor: usize,
    max_buffer_size: usize,
}

impl Reassembler {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
            cursor: 0,
            max_buffer_size: 10 * 1024 * 1024,
        }
    }

    pub fn try_next(&mut self) -> Option<Vec<u8>> {
        if self.available_len() < 4 {
            return None;
        }

        let len_bytes = &self.buffer[self.cursor..self.cursor + 4];
        let frame_len = u32::from_be_bytes(len_bytes.try_into().unwrap()) as usize;

        if frame_len == 0 || frame_len > self.max_buffer_size {
            self.cursor = self.buffer.len();
            self.compact();
            return None;
        }

        if self.available_len() < frame_len {
            return None;
        }

        let start = self.cursor;
        let end = self.cursor + frame_len;
        let frame = self.buffer[start..end].to_vec();

        self.cursor = end;
        if self.cursor > 4096 {
            self.compact();
        }

        Some(frame)
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

    pub fn feed_owned(&mut self, bytes: Vec<u8>) {
        if self.cursor > 0 {
            self.compact();
        }

        if bytes.len() > self.max_buffer_size
            || self.buffer.len().saturating_add(bytes.len()) > self.max_buffer_size
        {
            self.buffer.clear();
            self.cursor = 0;
            return;
        }

        if self.cursor == 0 && self.buffer.is_empty() {
            self.buffer = bytes;
            return;
        }
        self.buffer.extend_from_slice(&bytes);
    }

}
