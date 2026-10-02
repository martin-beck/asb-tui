//! Real PTY plus inherited fd/SCM_RIGHTS development handoff smoke test.
// Copyright (c) Huawei Technologies Co., Ltd. 2026. All rights reserved.
// SPDX-License-Identifier: MIT

use asb_tui::broker_adoption::{BROKER_PACKET_BYTES, BrokerPacket, receive_single};
use rustix::fd::{AsFd, OwnedFd};
use rustix::net::{
    AddressFamily, SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketFlags, SocketType,
    sendmsg, socketpair,
};
use rustix::pty::{OpenptFlags, grantpt, ioctl_tiocgptpeer, openpt, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};
use std::io::{IoSlice, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use asb_tui::control_codec::{
    BoundResult, Capabilities, ControlCall, ControlLimits, ControlRequest, ControlResponse,
    ControlResult, ControlSuccess, Negotiated, Page, Revision, V1_0,
};

fn packet() -> [u8; BROKER_PACKET_BYTES] {
    let mut bytes = [0; BROKER_PACKET_BYTES];
    bytes[..8].copy_from_slice(b"ASBHND01");
    bytes[8..10].copy_from_slice(&1_u16.to_be_bytes());
    bytes[10] = 1;
    bytes[11] = 1;
    bytes[16..32].fill(7);
    bytes[32..40].copy_from_slice(&1_u64.to_be_bytes());
    bytes[40..72].copy_from_slice(&BrokerPacket::runner_identity_digest("pty-runner").unwrap());
    bytes
}

fn send_rights(sender: &OwnedFd, offered: &OwnedFd, payload: &[u8]) {
    let iov = [IoSlice::new(payload)];
    let mut storage = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = SendAncillaryBuffer::new(&mut storage);
    let rights = [offered.as_fd()];
    ancillary.push(SendAncillaryMessage::ScmRights(&rights));
    sendmsg(sender, &iov, &mut ancillary, SendFlags::empty()).unwrap();
}

fn read_request(stream: &mut UnixStream) -> ControlRequest {
    let mut length = [0; 4];
    stream.read_exact(&mut length).unwrap();
    let size = u32::from_be_bytes(length) as usize;
    let mut body = vec![0; size];
    stream.read_exact(&mut body).unwrap();
    let mut frame = length.to_vec();
    frame.extend(body);
    asb_tui::control_codec::decode(&frame, 256 * 1024).unwrap()
}

fn write_result(stream: &mut UnixStream, request: &ControlRequest, result: ControlResult) {
    let response = ControlResponse::Success(asb_tui::control_codec::SuccessResponse {
        jsonrpc: "2.0".into(),
        id: request.id,
        result: ControlSuccess::Operation(BoundResult {
            request_sha256: "a".repeat(64),
            result,
        }),
    });
    stream
        .write_all(&asb_tui::control_codec::encode(&response, 256 * 1024).unwrap())
        .unwrap();
}

#[test]
fn pty_inherited_fd_negotiates_scm_rights_and_exits_cleanly() {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC).unwrap();
    grantpt(&master).unwrap();
    unlockpt(&master).unwrap();
    let slave = ioctl_tiocgptpeer(
        &master,
        OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC,
    )
    .unwrap();
    tcsetwinsize(
        &master,
        Winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )
    .unwrap();

    let (broker_parent, broker_child) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let (adoption_sender, adoption_receiver) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let (offered, _offered_peer) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let mut broker_parent = UnixStream::from(broker_parent);
    let mut child = Command::new("/bin/sh")
        .args([
            "-c",
            "dd bs=72 count=1 of=/dev/null 2>/dev/null; printf READY; read done; printf EXIT",
        ])
        .stdin(Stdio::from(broker_child))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .spawn()
        .unwrap();
    let payload = packet();
    send_rights(&adoption_sender, &offered, &payload);
    let received = receive_single(&adoption_receiver).unwrap();
    assert_eq!(received.broker_packet().unwrap().generation.sequence, 1);
    broker_parent.write_all(&payload).unwrap();
    broker_parent.write_all(b"\n").unwrap();

    let started = Instant::now();
    let mut output = Vec::new();
    let mut reader = std::fs::File::from(master);
    while started.elapsed() < Duration::from_secs(2) && !output.windows(4).any(|w| w == b"EXIT") {
        let mut chunk = [0; 128];
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => output.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    assert!(output.windows(5).any(|w| w == b"READY"));
    child.stdin.take();
    child.wait().unwrap();
    assert!(output.windows(4).any(|w| w == b"EXIT"));
    assert!(AsRawFd::as_raw_fd(&adoption_sender) >= 3);
}

#[test]
fn real_asb_tui_socket_consumer_negotiates_bootstrap_and_exits_from_pty() {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC).unwrap();
    grantpt(&master).unwrap();
    unlockpt(&master).unwrap();
    let slave = ioctl_tiocgptpeer(
        &master,
        OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC,
    )
    .unwrap();
    tcsetwinsize(
        &master,
        Winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )
    .unwrap();

    let root = std::env::temp_dir().join(format!(
        "asb-tui-pty-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket_path = root.join("control.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    listener.set_nonblocking(true).unwrap();
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600)).unwrap();

    let server_thread = std::thread::spawn(move || {
        let started = Instant::now();
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        started.elapsed() < Duration::from_secs(4),
                        "socket accept timed out"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("socket accept failed: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        let negotiate = read_request(&mut stream);
        assert!(matches!(negotiate.call, ControlCall::Negotiate(_)));
        let negotiated = ControlResponse::Success(asb_tui::control_codec::SuccessResponse {
            jsonrpc: "2.0".into(),
            id: negotiate.id,
            result: ControlSuccess::Negotiated(Negotiated {
                version: V1_0,
                limits: ControlLimits::default(),
                runner_instance_id: "socket-runner".into(),
                oldest_revision: Revision(1),
                latest_revision: Revision(1),
            }),
        });
        stream
            .write_all(&asb_tui::control_codec::encode(&negotiated, 256 * 1024).unwrap())
            .unwrap();
        let capabilities = read_request(&mut stream);
        assert!(matches!(capabilities.call, ControlCall::Capabilities));
        write_result(
            &mut stream,
            &capabilities,
            ControlResult::Capabilities(Capabilities {
                validate_settings: true,
                run_control: false,
                repeat: false,
                analysis: false,
                events: false,
            }),
        );
        let history = read_request(&mut stream);
        assert!(matches!(history.call, ControlCall::History(_)));
        write_result(
            &mut stream,
            &history,
            ControlResult::History(Page {
                items: Vec::new(),
                next: None,
                has_more: false,
            }),
        );
    });

    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join("asb-tui");
    let mut child = Command::new(binary)
        .args(["run", "--socket", socket_path.to_str().unwrap()])
        .env_clear()
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .spawn()
        .unwrap();
    let server_result = server_thread.join();
    if server_result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("socket control fixture failed");
    }
    std::thread::sleep(Duration::from_millis(100));
    let mut master = std::fs::File::from(master);
    master.write_all(b"q").unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() >= Duration::from_secs(4) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("TUI did not exit");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "socket consumer exited with {status}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn real_asb_tui_broker_handoff_uses_fd0_and_attached_output_pty() {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC).unwrap();
    grantpt(&master).unwrap();
    unlockpt(&master).unwrap();
    let slave = ioctl_tiocgptpeer(
        &master,
        OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC,
    )
    .unwrap();
    tcsetwinsize(
        &master,
        Winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )
    .unwrap();
    let (broker_parent, broker_child) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let (control_server, control_offered) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC,
        None,
    )
    .unwrap();
    let mut server = UnixStream::from(control_server);
    server
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    let server_thread = std::thread::spawn(move || {
        let negotiate = read_request(&mut server);
        assert!(matches!(negotiate.call, ControlCall::Negotiate(_)));
        let response = ControlResponse::Success(asb_tui::control_codec::SuccessResponse {
            jsonrpc: "2.0".into(),
            id: negotiate.id,
            result: ControlSuccess::Negotiated(Negotiated {
                version: V1_0,
                limits: ControlLimits::default(),
                runner_instance_id: "pty-runner".into(),
                oldest_revision: Revision(1),
                latest_revision: Revision(1),
            }),
        });
        server
            .write_all(&asb_tui::control_codec::encode(&response, 256 * 1024).unwrap())
            .unwrap();
        let capabilities = read_request(&mut server);
        assert!(matches!(capabilities.call, ControlCall::Capabilities));
        write_result(
            &mut server,
            &capabilities,
            ControlResult::Capabilities(Capabilities {
                validate_settings: true,
                run_control: false,
                repeat: false,
                analysis: false,
                events: false,
            }),
        );
        let history = read_request(&mut server);
        assert!(matches!(history.call, ControlCall::History(_)));
        write_result(
            &mut server,
            &history,
            ControlResult::History(Page {
                items: Vec::new(),
                next: None,
                has_more: false,
            }),
        );
    });
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join("asb-tui");
    let mut command = Command::new(binary);
    let mut child = command
        .args(["run", "--broker", "--development"])
        .env_clear()
        .env("TERM", "xterm-256color")
        // Exercise the production startup path: stdin is the inherited broker
        // stream, while stdout/stderr are the attached PTY used as the
        // development terminal fallback.
        .env(
            "ASB_TUI_DEVELOPMENT_DESCRIPTOR",
            serde_json::json!({
                "schema_version": 1, "profile": "development", "development_only": true,
                "operation": "launch", "protocol_minor": 0,
                "asb_source_commit": "a".repeat(40), "asb_source_tree": "b".repeat(40),
                "tui_source_commit": "c".repeat(40), "tui_source_tree": "d".repeat(40)
            })
            .to_string(),
        )
        .env("ASB_TUI_EXPECTED_ASB_SOURCE_COMMIT", "a".repeat(40))
        .env("ASB_TUI_EXPECTED_ASB_SOURCE_TREE", "b".repeat(40))
        .env("ASB_TUI_EXPECTED_TUI_SOURCE_COMMIT", "c".repeat(40))
        .env("ASB_TUI_EXPECTED_TUI_SOURCE_TREE", "d".repeat(40))
        .stdin(Stdio::from(broker_child))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .spawn()
        .unwrap();
    send_rights(&broker_parent, &control_offered, &packet());
    let server_result = server_thread.join();
    if server_result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("broker control fixture failed");
    }
    std::thread::sleep(Duration::from_millis(100));
    let mut master = std::fs::File::from(master);
    master.write_all(b"q").unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() >= Duration::from_secs(4) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("broker TUI did not exit");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "broker TUI exited with {status}");
}
