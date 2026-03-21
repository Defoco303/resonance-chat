use crate::packets::chat;
use crate::packets::reassembler::Reassembler;
use crate::packets::utils::{BinaryReader, TCPReassembler};
use etherparse::NetSlice::Ipv4;
use etherparse::SlicedPacket;
use etherparse::TransportSlice::Tcp;
use std::collections::HashMap;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use windivert::WinDivert;
use windivert::prelude::WinDivertFlags;

const CHAT_SERVICE_UUID: u64 = 0x0000000009d4a768;
const CHAT_METHOD_ID: u32 = 1;
const FRAG_NOTIFY: u16 = 2;
const FRAG_FRAME_DOWN: u16 = 6;

pub fn start_capture(handle: AppHandle) {
    let wd = match WinDivert::network(
        "!loopback && ip && tcp",
        0,
        WinDivertFlags::new().set_sniff(),
    ) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to open WinDivert: {e}");
            return;
        }
    };

    let mut buf = vec![0u8; 10 * 1024 * 1024];
    let mut game_server_ip: Option<[u8; 4]> = None;
    // key = "src_ip:src_port", value = (TCPReassembler, Reassembler, last_seen)
    let mut secondary: HashMap<String, (TCPReassembler, Reassembler, Instant)> = HashMap::new();
    let mut packet_count: u64 = 0;

    loop {
        let pkt = match wd.recv(Some(&mut buf)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("WinDivert recv error: {e}");
                // 一時的なエラーは継続、バッファ関連エラーのみ停止
                if e.to_string().contains("invalid handle") || e.to_string().contains("access denied") {
                    break;
                }
                continue;
            }
        };

        packet_count += 1;

        // 古いエントリを定期削除（1万パケットごと、60秒以上アクセスなし）
        if packet_count % 10000 == 0 {
            secondary.retain(|_, v| v.2.elapsed().as_secs() < 60);
        }

        let Ok(sliced) = SlicedPacket::from_ip(&pkt.data) else {
            continue;
        };
        let Some(Ipv4(ip)) = sliced.net else {
            continue;
        };
        let Some(Tcp(tcp)) = sliced.transport else {
            continue;
        };

        let src_ip = ip.header().source();
        let payload = tcp.payload();

        // ログインパケットを常時監視（再接続・サーバーIP変更に対応）
        if let Some(detected) = detect_game_server_ip(src_ip, payload) {
            if game_server_ip != Some(detected) {
                eprintln!(
                    "Game server detected/changed: {}.{}.{}.{}",
                    detected[0], detected[1], detected[2], detected[3]
                );
                // サーバーが変わったので既存の接続状態をすべてクリア
                game_server_ip = Some(detected);
                secondary.clear();
            }
        }

        // Track all connections from same /16 subnet (includes game + chat servers)
        if let Some(game_ip) = game_server_ip {
            if src_ip[0] == game_ip[0] && src_ip[1] == game_ip[1] && !payload.is_empty() {
                let tcp_header = tcp.to_header();
                let src_port = tcp_header.source_port;
                let is_syn = tcp_header.syn;
                let seq = tcp_header.sequence_number;

                let label = format!(
                    "{}.{}.{}.{}:{}",
                    src_ip[0], src_ip[1], src_ip[2], src_ip[3], src_port
                );
                let entry = secondary
                    .entry(label)
                    .or_insert_with(|| (TCPReassembler::new(), Reassembler::new(), Instant::now()));

                // SYN = 新接続なのでリセット
                if is_syn {
                    entry.0.reset(Some(seq));
                    entry.1 = Reassembler::new();
                }

                entry.2 = Instant::now();

                if let Some(reassembled) = entry.0.insert_segment(seq, payload) {
                    entry.1.feed_owned(reassembled);
                }
                while let Some(frame) = entry.1.try_next() {
                    process_frame(frame, &handle);
                }
            }
        }
    }
}

fn detect_game_server_ip(src_ip: [u8; 4], payload: &[u8]) -> Option<[u8; 4]> {
    // Login response: exactly 98 bytes with known signature
    if payload.len() == 98 {
        const SIG1: [u8; 10] = [0x00, 0x00, 0x00, 0x62, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01];
        const SIG2: [u8; 6] = [0x00, 0x00, 0x00, 0x00, 0x0a, 0x4e];
        if payload[0..10] == SIG1 && payload[14..20] == SIG2 {
            return Some(src_ip);
        }
    }
    None
}

fn process_frame(frame: Vec<u8>, handle: &AppHandle) {
    let mut reader = BinaryReader::from(frame);
    process_frame_inner(&mut reader, handle);
}

fn process_frame_inner(reader: &mut BinaryReader, handle: &AppHandle) {
    if reader.read_u32().is_err() {
        return;
    }
    let packet_type = match reader.read_u16() {
        Ok(pt) => pt,
        Err(_) => return,
    };
    let is_compressed = (packet_type & 0x8000) != 0;
    let msg_type_id = packet_type & 0x7fff;

    match msg_type_id {
        FRAG_NOTIFY => {
            process_notify(reader, is_compressed, handle);
        }
        FRAG_FRAME_DOWN => {
            let _ = reader.read_u32(); // server_sequence_id
            if reader.remaining() == 0 {
                return;
            }
            let nested = reader.read_remaining().to_vec();
            let nested = if is_compressed {
                match zstd::decode_all(nested.as_slice()) {
                    Ok(d) => d,
                    Err(_) => return,
                }
            } else {
                nested
            };
            let mut nested_reader = BinaryReader::from(nested);
            loop {
                if nested_reader.remaining() < 4 {
                    break;
                }
                let frame_len = match nested_reader.peek_u32() {
                    Ok(l) => l as usize,
                    Err(_) => break,
                };
                if frame_len < 6 || nested_reader.remaining() < frame_len {
                    break;
                }
                let frame_bytes = match nested_reader.read_bytes(frame_len) {
                    Ok(b) => b,
                    Err(_) => break,
                };
                process_frame_inner(&mut BinaryReader::from(frame_bytes), handle);
            }
        }
        _ => {}
    }
}

fn process_notify(reader: &mut BinaryReader, is_compressed: bool, handle: &AppHandle) {
    let service_uuid = match reader.read_u64() {
        Ok(v) => v,
        Err(_) => return,
    };
    let _ = reader.read_u32(); // stub_id
    let method_id = match reader.read_u32() {
        Ok(v) => v,
        Err(_) => return,
    };

    if service_uuid != CHAT_SERVICE_UUID || method_id != CHAT_METHOD_ID {
        return;
    }

    let payload = reader.read_remaining().to_vec();
    let payload = if is_compressed {
        match zstd::decode_all(payload.as_slice()) {
            Ok(d) => d,
            Err(_) => return,
        }
    } else {
        payload
    };

    for msg in chat::parse_chat_notify(&payload) {
        let _ = handle.emit("chat-message", &msg);
    }
}
