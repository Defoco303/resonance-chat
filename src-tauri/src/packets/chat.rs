use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct ChatMessage {
    pub channel: i32,
    pub channel_name: String,
    pub sender_id: u64,
    pub sender_name: String,
    pub text: String,
    pub timestamp: u64,
}

fn channel_name(ch: i32) -> String {
    match ch {
        1 => "ワールド",
        2 => "チャンネル",
        3 => "パーティ",
        4 => "ギルド",
        _ => "その他",
    }
    .to_string()
}

fn read_varint(data: &[u8], offset: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        if *offset >= data.len() {
            return None;
        }
        let byte = data[*offset];
        *offset += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

fn read_len_delimited<'a>(data: &'a [u8], offset: &mut usize) -> Option<&'a [u8]> {
    let len = read_varint(data, offset)? as usize;
    if *offset + len > data.len() {
        return None;
    }
    let slice = &data[*offset..*offset + len];
    *offset += len;
    Some(slice)
}

fn skip_field(data: &[u8], offset: &mut usize, wire_type: u64) -> bool {
    match wire_type {
        0 => read_varint(data, offset).is_some(),
        1 => {
            if *offset + 8 > data.len() {
                false
            } else {
                *offset += 8;
                true
            }
        }
        2 => read_len_delimited(data, offset).is_some(),
        5 => {
            if *offset + 4 > data.len() {
                false
            } else {
                *offset += 4;
                true
            }
        }
        _ => false,
    }
}

fn parse_sender_info(data: &[u8]) -> (u64, String) {
    let mut player_id = 0u64;
    let mut name = String::new();
    let mut offset = 0;
    while offset < data.len() {
        let Some(tag) = read_varint(data, &mut offset) else {
            break;
        };
        let field = tag >> 3;
        let wire_type = tag & 0x7;
        match field {
            1 if wire_type == 0 => {
                player_id = read_varint(data, &mut offset).unwrap_or(0);
            }
            2 if wire_type == 2 => {
                if let Some(b) = read_len_delimited(data, &mut offset) {
                    name = String::from_utf8_lossy(b).into_owned();
                }
            }
            _ => {
                skip_field(data, &mut offset, wire_type);
            }
        }
    }
    (player_id, name)
}

fn parse_chat_content(data: &[u8]) -> String {
    let mut text = String::new();
    let mut offset = 0;
    while offset < data.len() {
        let Some(tag) = read_varint(data, &mut offset) else {
            break;
        };
        let field = tag >> 3;
        let wire_type = tag & 0x7;
        match field {
            3 if wire_type == 2 => {
                if let Some(b) = read_len_delimited(data, &mut offset) {
                    text = String::from_utf8_lossy(b).into_owned();
                }
            }
            _ => {
                skip_field(data, &mut offset, wire_type);
            }
        }
    }
    text
}

fn parse_chat_entry(data: &[u8]) -> (u64, String, u64, String) {
    let mut sender_id = 0u64;
    let mut sender_name = String::new();
    let mut timestamp = 0u64;
    let mut text = String::new();
    let mut offset = 0;
    while offset < data.len() {
        let Some(tag) = read_varint(data, &mut offset) else {
            break;
        };
        let field = tag >> 3;
        let wire_type = tag & 0x7;
        match field {
            2 if wire_type == 2 => {
                if let Some(b) = read_len_delimited(data, &mut offset) {
                    (sender_id, sender_name) = parse_sender_info(b);
                }
            }
            3 if wire_type == 0 => {
                timestamp = read_varint(data, &mut offset).unwrap_or(0);
            }
            4 if wire_type == 2 => {
                if let Some(b) = read_len_delimited(data, &mut offset) {
                    text = parse_chat_content(b);
                }
            }
            _ => {
                skip_field(data, &mut offset, wire_type);
            }
        }
    }
    (sender_id, sender_name, timestamp, text)
}

fn parse_chat_msg(data: &[u8]) -> Option<ChatMessage> {
    let mut channel = 0i32;
    let mut sender_id = 0u64;
    let mut sender_name = String::new();
    let mut timestamp = 0u64;
    let mut text = String::new();
    let mut offset = 0;
    while offset < data.len() {
        let Some(tag) = read_varint(data, &mut offset) else {
            break;
        };
        let field = tag >> 3;
        let wire_type = tag & 0x7;
        match field {
            1 if wire_type == 0 => {
                channel = read_varint(data, &mut offset).unwrap_or(0) as i32;
            }
            2 if wire_type == 2 => {
                if let Some(b) = read_len_delimited(data, &mut offset) {
                    (sender_id, sender_name, timestamp, text) = parse_chat_entry(b);
                }
            }
            _ => {
                skip_field(data, &mut offset, wire_type);
            }
        }
    }
    if text.is_empty() {
        return None;
    }
    Some(ChatMessage {
        channel,
        channel_name: channel_name(channel),
        sender_id,
        sender_name,
        text,
        timestamp,
    })
}

pub fn parse_chat_notify(data: &[u8]) -> Vec<ChatMessage> {
    let mut msgs = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let Some(tag) = read_varint(data, &mut offset) else {
            break;
        };
        let field = tag >> 3;
        let wire_type = tag & 0x7;
        match field {
            1 if wire_type == 2 => {
                if let Some(b) = read_len_delimited(data, &mut offset) {
                    if let Some(msg) = parse_chat_msg(b) {
                        msgs.push(msg);
                    }
                }
            }
            _ => {
                skip_field(data, &mut offset, wire_type);
            }
        }
    }
    msgs
}
