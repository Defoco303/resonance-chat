use crate::packets::{chat, dps};
use crate::packets::reassembler::Reassembler;
use crate::packets::utils::{BinaryReader, TCPReassembler};
use etherparse::NetSlice::Ipv4;
use etherparse::SlicedPacket;
use etherparse::TransportSlice::Tcp;
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use windivert::WinDivert;
use windivert::prelude::{CloseAction, WinDivertFlags, WinDivertParam};

#[derive(Clone, serde::Serialize)]
struct CaptureStatusPayload {
    code: &'static str,
    level: &'static str,
    message: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ConnectionKey {
    src_ip: [u8; 4],
    src_port: u16,
    dst_ip: [u8; 4],
    dst_port: u16,
}

const CHAT_SERVICE_UUID: u64 = 0x0000000009d4a768;
const CHAT_METHOD_ID: u32 = 1;
const FRAG_NOTIFY: u16 = 2;
const FRAG_FRAME_DOWN: u16 = 6;

const INITIAL_CAPTURE_FILTER: &str = "inbound && !loopback && ip && tcp && tcp.PayloadLength > 0";
const MAX_CAPTURE_PACKET_SIZE: usize = 128 * 1024;
const MAX_TRACKED_CONNECTIONS: usize = 128;
const MAX_DECOMPRESSED_SIZE: usize = 1024 * 1024;
const MAX_FRAME_SIZE: usize = 1024 * 1024;
const MAX_FRAME_NESTING: usize = 8;

static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn request_stop() {
    STOP_REQUESTED.store(true, Ordering::SeqCst);
}

pub fn stop_driver() {
    if let Err(err) = WinDivert::uninstall() {
        eprintln!("failed to stop WinDivert driver: {err}");
    }
}

pub fn start_capture(handle: AppHandle) {
    STOP_REQUESTED.store(false, Ordering::SeqCst);

    let mut wd = match WinDivert::network(
        INITIAL_CAPTURE_FILTER,
        0,
        WinDivertFlags::new().set_recv_only().set_sniff(),
    ) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to open WinDivert: {e}");
            emit_capture_status(&handle, &e.to_string());
            return;
        }
    };

    // Maximize the WinDivert receive queue so bursty combat traffic isn't
    // dropped while userspace catches up. A dropped TCP segment desyncs the
    // per-connection reassembler and stalls that stream until it reconnects.
    for (param, value) in [
        (WinDivertParam::QueueLength, 16_384),
        (WinDivertParam::QueueSize, 33_554_432),
        (WinDivertParam::QueueTime, 16_000),
    ] {
        if let Err(err) = wd.set_param(param, value) {
            eprintln!("failed to set WinDivert {param:?}: {err}");
        }
    }

    let mut buf = vec![0u8; 10 * 1024 * 1024];
    let mut game_server_ip: Option<[u8; 4]> = None;
    let mut game_server_connection: Option<ConnectionKey> = None;
    let mut secondary: HashMap<String, (TCPReassembler, Reassembler, Instant)> = HashMap::new();
    let mut dps_meter = dps::DpsMeter::default();
    let mut packet_count: u64 = 0;

    loop {
        if STOP_REQUESTED.load(Ordering::SeqCst) {
            break;
        }

        let pkt = match wd.recv(Some(&mut buf)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("WinDivert recv error: {e}");
                let error_text = e.to_string();
                if STOP_REQUESTED.load(Ordering::SeqCst)
                    || error_text.contains("NoData")
                    || error_text.contains("invalid handle")
                    || error_text.contains("access denied")
                {
                    if !STOP_REQUESTED.load(Ordering::SeqCst) {
                        emit_capture_status(&handle, &error_text);
                    }
                    break;
                }
                continue;
            }
        };

        packet_count += 1;

        if pkt.data.len() > MAX_CAPTURE_PACKET_SIZE {
            continue;
        }

        if packet_count % 10_000 == 0 {
            secondary.retain(|_, value| value.2.elapsed().as_secs() < 60);
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
        let dst_ip = ip.header().destination();
        let tcp_header = tcp.to_header();
        let connection = ConnectionKey {
            src_ip,
            src_port: tcp_header.source_port,
            dst_ip,
            dst_port: tcp_header.destination_port,
        };
        let payload = tcp.payload();

        if detect_scene_server_change(payload) && game_server_connection != Some(connection) {
            let was_connected = game_server_connection.is_some();
            eprintln!(
                "Game server detected/changed by scene signature: {}.{}.{}.{}:{}",
                src_ip[0], src_ip[1], src_ip[2], src_ip[3], connection.src_port
            );
            game_server_ip = Some(src_ip);
            game_server_connection = Some(connection);
            secondary.clear();
            if was_connected {
                reset_dps_meter(&handle, &mut dps_meter);
            }
        }

        // Learn the server IP from the login response before reassembling chat traffic.
        if let Some(detected) = detect_game_server_ip(src_ip, payload) {
            if game_server_ip != Some(detected) || game_server_connection != Some(connection) {
                let was_connected = game_server_ip.is_some() || game_server_connection.is_some();
                eprintln!(
                    "Game server detected/changed: {}.{}.{}.{}",
                    detected[0], detected[1], detected[2], detected[3]
                );
                game_server_ip = Some(detected);
                game_server_connection = Some(connection);
                secondary.clear();
                if was_connected {
                    reset_dps_meter(&handle, &mut dps_meter);
                }
            }
        }

        // Chat traffic may come from sibling hosts on the same /16 as the game server.
        if let Some(game_ip) = game_server_ip {
            if src_ip[0] == game_ip[0] && src_ip[1] == game_ip[1] && !payload.is_empty() {
                let src_port = tcp_header.source_port;
                let is_syn = tcp_header.syn;
                let seq = tcp_header.sequence_number;

                let label = format!(
                    "{}.{}.{}.{}:{}",
                    src_ip[0], src_ip[1], src_ip[2], src_ip[3], src_port
                );

                trim_secondary_connections(&mut secondary);
                let entry = secondary
                    .entry(label)
                    .or_insert_with(|| (TCPReassembler::new(), Reassembler::new(), Instant::now()));

                if is_syn {
                    entry.0.reset(Some(seq));
                    entry.1 = Reassembler::new();
                }

                entry.2 = Instant::now();

                if let Some(reassembled) = entry.0.insert_segment(seq, payload) {
                    entry.1.feed_owned(reassembled);
                }

                while let Some(frame) = entry.1.try_next() {
                    process_frame(frame, &handle, &mut dps_meter);
                }
            }
        }
    }

    if let Err(err) = wd.close(CloseAction::Uninstall) {
        eprintln!("failed to close WinDivert handle: {err}");
        stop_driver();
    }
}

fn emit_capture_status(handle: &AppHandle, error_text: &str) {
    let lower = error_text.to_ascii_lowercase();
    let (code, message) = if lower.contains("access denied") {
        (
            "admin_required",
            "Administrator permission is required to start chat capture. Please relaunch the app as administrator.".to_string(),
        )
    } else if lower.contains("invalid handle") {
        (
            "capture_stopped",
            "Chat capture stopped unexpectedly. Please relaunch the app or restart Windows and try again.".to_string(),
        )
    } else {
        (
            "capture_error",
            format!("Failed to initialize chat capture: {error_text}"),
        )
    };

    let _ = handle.emit(
        "capture-status",
        CaptureStatusPayload {
            code,
            level: "error",
            message,
        },
    );
}

fn detect_game_server_ip(src_ip: [u8; 4], payload: &[u8]) -> Option<[u8; 4]> {
    if payload.len() == 98 {
        const SIG1: [u8; 10] = [0x00, 0x00, 0x00, 0x62, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01];
        const SIG2: [u8; 6] = [0x00, 0x00, 0x00, 0x00, 0x0a, 0x4e];
        if payload[0..10] == SIG1 && payload[14..20] == SIG2 {
            return Some(src_ip);
        }
    }

    None
}

fn detect_scene_server_change(payload: &[u8]) -> bool {
    if payload.len() < 10 || payload[4] != 0 {
        return false;
    }

    const FRAG_LENGTH_SIZE: usize = 4;
    const SIGNATURE: [u8; 6] = [0x00, 0x63, 0x33, 0x53, 0x42, 0x00];
    const MAX_FRAG_ITERATIONS: usize = 2000;

    let mut reader = BinaryReader::from(payload.to_vec());
    if reader.read_bytes(10).is_err() {
        return false;
    }

    for _ in 0..MAX_FRAG_ITERATIONS {
        if reader.remaining() < FRAG_LENGTH_SIZE {
            break;
        }

        let Ok(frame_len) = reader.read_u32() else {
            break;
        };
        let frag_len = frame_len.saturating_sub(FRAG_LENGTH_SIZE as u32) as usize;
        if frag_len == 0 || reader.remaining() < frag_len {
            break;
        }

        let Ok(fragment) = reader.read_bytes(frag_len) else {
            break;
        };
        if fragment.len() >= 5 + SIGNATURE.len()
            && fragment[5..5 + SIGNATURE.len()] == SIGNATURE
        {
            return true;
        }
    }

    false
}

fn reset_dps_meter(handle: &AppHandle, dps_meter: &mut dps::DpsMeter) {
    dps_meter.reset_for_server_change();
    let _ = handle.emit("dps-meter", dps_meter.snapshot());
}

fn process_frame(frame: Vec<u8>, handle: &AppHandle, dps_meter: &mut dps::DpsMeter) {
    let mut reader = BinaryReader::from(frame);
    process_frame_inner(&mut reader, handle, dps_meter, 0);
}

fn process_frame_inner(
    reader: &mut BinaryReader,
    handle: &AppHandle,
    dps_meter: &mut dps::DpsMeter,
    depth: usize,
) {
    if depth > MAX_FRAME_NESTING {
        return;
    }

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
            process_notify(reader, is_compressed, handle, dps_meter);
        }
        FRAG_FRAME_DOWN => {
            let _ = reader.read_u32();
            if reader.remaining() == 0 {
                return;
            }

            let nested = reader.read_remaining().to_vec();
            let nested = if is_compressed {
                match decode_limited(nested.as_slice(), MAX_DECOMPRESSED_SIZE) {
                    Ok(d) => d,
                    Err(_) => return,
                }
            } else {
                if nested.len() > MAX_DECOMPRESSED_SIZE {
                    return;
                }
                nested
            };

            let mut nested_reader = BinaryReader::from(nested);
            loop {
                if nested_reader.remaining() < 4 {
                    break;
                }

                let frame_len = match nested_reader.peek_u32() {
                    Ok(length) => length as usize,
                    Err(_) => break,
                };

                if frame_len < 6
                    || frame_len > MAX_FRAME_SIZE
                    || nested_reader.remaining() < frame_len
                {
                    break;
                }

                let frame_bytes = match nested_reader.read_bytes(frame_len) {
                    Ok(bytes) => bytes,
                    Err(_) => break,
                };

                process_frame_inner(
                    &mut BinaryReader::from(frame_bytes),
                    handle,
                    dps_meter,
                    depth + 1,
                );
            }
        }
        _ => {}
    }
}

fn process_notify(
    reader: &mut BinaryReader,
    is_compressed: bool,
    handle: &AppHandle,
    dps_meter: &mut dps::DpsMeter,
) {
    let service_uuid = match reader.read_u64() {
        Ok(v) => v,
        Err(_) => return,
    };
    let _ = reader.read_u32();
    let method_id = match reader.read_u32() {
        Ok(v) => v,
        Err(_) => return,
    };

    let payload = reader.read_remaining().to_vec();
    let payload = if is_compressed {
        match decode_limited(payload.as_slice(), MAX_DECOMPRESSED_SIZE) {
            Ok(d) => d,
            Err(_) => return,
        }
    } else {
        if payload.len() > MAX_DECOMPRESSED_SIZE {
            return;
        }
        payload
    };

    if service_uuid == CHAT_SERVICE_UUID && method_id == CHAT_METHOD_ID {
        for msg in chat::parse_chat_notify(&payload) {
            let _ = handle.emit("chat-message", &msg);
        }
    } else if service_uuid == dps::DPS_SERVICE_UUID {
        let changed = dps_meter.process_packet(method_id, &payload);
        if changed && (dps_meter.is_empty() || dps_meter.should_emit()) {
            let _ = handle.emit("dps-meter", dps_meter.snapshot());
        }
    }
}

fn decode_limited(payload: &[u8], max_output: usize) -> std::io::Result<Vec<u8>> {
    let mut decoder = zstd::stream::read::Decoder::new(payload)?;
    let mut out = Vec::new();
    decoder
        .by_ref()
        .take((max_output + 1) as u64)
        .read_to_end(&mut out)?;

    if out.len() > max_output {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "decompressed payload exceeded limit",
        ));
    }

    Ok(out)
}

fn trim_secondary_connections(
    secondary: &mut HashMap<String, (TCPReassembler, Reassembler, Instant)>,
) {
    while secondary.len() + 1 > MAX_TRACKED_CONNECTIONS {
        let Some(oldest_key) = secondary
            .iter()
            .max_by_key(|(_, value)| value.2.elapsed())
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        secondary.remove(&oldest_key);
    }
}
