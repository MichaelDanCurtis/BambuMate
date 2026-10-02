//! Finds Bambu printers on the local network by listening for the SSDP
//! `NOTIFY` messages they multicast. Discovery only listens; it sends nothing.
//!
//! Printers announce `NT: urn:bambulab-com:device:3dprinter:1` with the IP in
//! `Location`, the serial in `USN`, and `DevName.bambu.com` /
//! `DevModel.bambu.com` headers, to UDP 2021 and 1990 (some sources say
//! 1900, so all three are joined).
//!
//! Anyone on the LAN can send these datagrams, so every field is untrusted:
//! the serial must be a plain serial, names are cleaned and capped, the
//! `Location` IP must be the address the datagram came from, and the result
//! list is capped.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::settings::valid_serial;

pub const DISCOVERY_PORTS: &[u16] = &[2021, 1990, 1900];
/// A single bounded scan. Five seconds often missed the next announcement.
pub const DISCOVERY_WINDOW: Duration = Duration::from_secs(15);
const SSDP_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const BAMBU_URN: &str = "urn:bambulab-com:device:3dprinter";
/// Most printers one discovery run reports. A flood of forged announcements
/// can't grow the list past this.
pub const MAX_DISCOVERED: usize = 32;
/// Longest name or model kept, in characters.
const MAX_LABEL_CHARS: usize = 64;
/// Consecutive receive errors a listener tolerates before giving up.
const MAX_RECV_ERRORS: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPrinter {
    pub ip: String,
    pub serial: String,
    pub name: String,
    pub model: String,
    /// Another address announced the same serial during the window. The first
    /// announcement is kept; the UI should warn that the serial is claimed by
    /// more than one device.
    #[serde(default)]
    pub conflict: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryReport {
    pub printers: Vec<DiscoveredPrinter>,
    /// Partial listener failures remain visible even when another port works.
    pub warnings: Vec<String>,
    pub listen_seconds: u64,
}

enum DiscoveryEvent {
    Printer(DiscoveredPrinter),
    Failed(String),
}

/// A readable model name for a `DevModel.bambu.com` code. Codes are the
/// `model_id`s in Bambu Studio's `resources/profiles/BBL/machine/*.json`;
/// unknown codes are shown as sent.
pub fn model_name(code: &str) -> String {
    match code {
        "O1D" => "H2D",
        "O1E" => "H2D Pro",
        "O1C2" => "H2C",
        "O1S" => "H2S",
        "BL-P001" | "3DPrinter-X1-Carbon" => "X1 Carbon",
        "BL-P002" | "3DPrinter-X1" => "X1",
        "C13" => "X1E",
        "C11" => "P1P",
        "C12" => "P1S",
        "N7" => "P2S",
        "N1" => "A1 mini",
        "N2S" => "A1",
        other => other,
    }
    .to_string()
}

/// Drops control characters, trims, and keeps at most 64 characters.
fn clean_label(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .take(MAX_LABEL_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Whether `ip` can be a printer's own address: not unspecified, multicast or
/// broadcast, and loopback only when `allow_loopback` (a test seam).
fn usable_ip(ip: &IpAddr, allow_loopback: bool) -> bool {
    let broadcast = matches!(ip, IpAddr::V4(v4) if v4.is_broadcast());
    !(ip.is_unspecified()
        || ip.is_multicast()
        || broadcast
        || (ip.is_loopback() && !allow_loopback))
}

fn parse_fields(
    datagram: &[u8],
    source: Option<IpAddr>,
    allow_loopback: bool,
) -> Option<DiscoveredPrinter> {
    let text = std::str::from_utf8(datagram).ok()?;
    let headers: HashMap<String, String> = text
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect();
    let kind = headers.get("nt").or_else(|| headers.get("st"))?;
    if !kind.starts_with(BAMBU_URN) {
        return None;
    }
    let ip: IpAddr = headers.get("location")?.parse().ok()?;
    if !usable_ip(&ip, allow_loopback) {
        return None;
    }
    if source.is_some_and(|s| s != ip) {
        return None;
    }
    let serial = valid_serial(headers.get("usn")?)?;
    Some(DiscoveredPrinter {
        ip: ip.to_string(),
        name: headers
            .get("devname.bambu.com")
            .map(|n| clean_label(n))
            .unwrap_or_default(),
        model: headers
            .get("devmodel.bambu.com")
            .map(|m| clean_label(&model_name(&clean_label(m))))
            .unwrap_or_default(),
        serial,
        conflict: false,
    })
}

/// Parses one SSDP datagram. `None` unless it is a Bambu printer
/// announcement with a usable IP address and a valid serial.
///
/// This checks the fields only. It does not know where the datagram came
/// from, so it cannot catch a forged `Location`; discovery itself uses
/// [`parse_announcement`], which does.
pub fn parse_notify(datagram: &[u8]) -> Option<DiscoveredPrinter> {
    parse_fields(datagram, None, false)
}

/// Like [`parse_notify`], and additionally requires the `Location` IP to
/// equal `source`, the address the datagram was received from. A device can
/// then only announce itself, not point the app at another host.
///
/// Loopback addresses are rejected unless `allow_loopback` is set. That flag
/// exists so tests can send announcements to 127.0.0.1; production discovery
/// always passes `false`.
pub fn parse_announcement(
    datagram: &[u8],
    source: IpAddr,
    allow_loopback: bool,
) -> Option<DiscoveredPrinter> {
    parse_fields(datagram, Some(source), allow_loopback)
}

/// Adds `p` to `found`: the first announcement per serial wins, a later one
/// from a different IP marks the first as conflicting, and no new serial is
/// accepted once `MAX_DISCOVERED` are held.
fn merge(found: &mut HashMap<String, DiscoveredPrinter>, p: DiscoveredPrinter) {
    if let Some(first) = found.get_mut(&p.serial) {
        if first.ip != p.ip {
            first.conflict = true;
        }
    } else if found.len() < MAX_DISCOVERED {
        found.insert(p.serial.clone(), p);
    }
}

fn bind_listener(port: u16, allow_loopback: bool) -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // Bambu Studio may already be listening on 2021.
    socket.set_reuse_address(true)?;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    // Loopback tests neither bind a LAN listener nor join a multicast group.
    let address = if allow_loopback {
        Ipv4Addr::LOCALHOST
    } else {
        Ipv4Addr::UNSPECIFIED
    };
    socket.bind(&SocketAddrV4::new(address, port).into())?;
    if !allow_loopback {
        socket.join_multicast_v4(&SSDP_GROUP, &Ipv4Addr::UNSPECIFIED)?;
    }
    UdpSocket::from_std(socket.into())
}

async fn listen_on(
    socket: UdpSocket,
    port: u16,
    tx: mpsc::Sender<DiscoveryEvent>,
    allow_loopback: bool,
) {
    let mut buf = vec![0u8; 2048];
    let mut errors = 0;
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((n, from)) => {
                errors = 0;
                if let Some(p) = parse_announcement(&buf[..n], from.ip(), allow_loopback) {
                    if tx.send(DiscoveryEvent::Printer(p)).await.is_err() {
                        return;
                    }
                }
            }
            Err(e) => {
                errors += 1;
                if errors > MAX_RECV_ERRORS {
                    let _ = tx
                        .send(DiscoveryEvent::Failed(format!(
                            "UDP {port} stopped receiving: {e}"
                        )))
                        .await;
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// Sockets bound and listening. Dropping this stops the listeners.
struct Listening {
    rx: mpsc::Receiver<DiscoveryEvent>,
    warnings: Vec<String>,
    active: usize,
    _tasks: JoinSet<()>,
}

impl Listening {
    /// Binds every port in `ports` that can be bound, before returning, so
    /// anything sent afterwards is buffered by the socket.
    fn start(ports: &[u16], allow_loopback: bool) -> Result<Self, String> {
        Self::start_with(ports, allow_loopback, bind_listener)
    }

    fn start_with(
        ports: &[u16],
        allow_loopback: bool,
        mut bind: impl FnMut(u16, bool) -> std::io::Result<UdpSocket>,
    ) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel(64);
        let mut tasks = JoinSet::new();
        let mut warnings = Vec::new();
        for &port in ports {
            match bind(port, allow_loopback) {
                Ok(socket) => {
                    tasks.spawn(listen_on(socket, port, tx.clone(), allow_loopback));
                }
                Err(e) => warnings.push(format!("Cannot listen on UDP {port}: {e}")),
            }
        }
        let active = tasks.len();
        if active == 0 {
            return Err(discovery_error(&warnings));
        }
        Ok(Self {
            rx,
            warnings,
            active,
            _tasks: tasks,
        })
    }

    /// Collects announcements for `window`, sorted by name.
    async fn collect(mut self, window: Duration) -> Result<DiscoveryReport, String> {
        let mut found: HashMap<String, DiscoveredPrinter> = HashMap::new();
        let deadline = tokio::time::Instant::now() + window;
        while tokio::time::Instant::now() < deadline {
            let Ok(Some(event)) = tokio::time::timeout_at(deadline, self.rx.recv()).await else {
                break;
            };
            match event {
                DiscoveryEvent::Printer(p) => merge(&mut found, p),
                DiscoveryEvent::Failed(error) => {
                    self.warnings.push(error);
                    self.active = self.active.saturating_sub(1);
                    if self.active == 0 {
                        break;
                    }
                }
            }
        }
        if self.active == 0 && found.is_empty() {
            return Err(discovery_error(&self.warnings));
        }
        let mut out: Vec<DiscoveredPrinter> = found.into_values().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.serial.cmp(&b.serial)));
        Ok(DiscoveryReport {
            printers: out,
            warnings: self.warnings,
            listen_seconds: window.as_secs(),
        })
    }
}

fn discovery_error(warnings: &[String]) -> String {
    format!("Couldn't listen for printer announcements. Check local-network access and whether another app is using the discovery ports, or enter the printer's IP and serial manually. {}", warnings.join("; "))
}

/// Listens on `ports` for `window` and returns each printer heard, once,
/// sorted by name, at most [`MAX_DISCOVERED`] of them. Ports that can't be
/// bound are reported, and total listener failure is an error. The deadline
/// never resets on traffic, so a flood cannot extend a scan indefinitely.
pub async fn discover(ports: &[u16], window: Duration) -> Result<DiscoveryReport, String> {
    Listening::start(ports, false)?.collect(window).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTIFY: &str = "NOTIFY * HTTP/1.1\r\n\
        HOST: 239.255.255.250:1900\r\n\
        Server: UPnP/1.0\r\n\
        Location: 192.168.1.20\r\n\
        NT: urn:bambulab-com:device:3dprinter:1\r\n\
        NTS: ssdp:alive\r\n\
        USN: 0948AB000000001\r\n\
        Cache-Control: max-age=1800\r\n\
        DevModel.bambu.com: O1D\r\n\
        DevName.bambu.com: Workshop H2D\r\n\
        DevSignal.bambu.com: -45\r\n\
        DevConnect.bambu.com: cloud\r\n\
        DevBind.bambu.com: occupied\r\n\r\n";

    fn printer(ip: &str, serial: &str) -> DiscoveredPrinter {
        DiscoveredPrinter {
            ip: ip.into(),
            serial: serial.into(),
            name: String::new(),
            model: String::new(),
            conflict: false,
        }
    }

    #[test]
    fn parses_a_bambu_notify() {
        assert_eq!(
            parse_notify(NOTIFY.as_bytes()),
            Some(DiscoveredPrinter {
                ip: "192.168.1.20".into(),
                serial: "0948AB000000001".into(),
                name: "Workshop H2D".into(),
                model: "H2D".into(),
                conflict: false,
            })
        );
    }

    #[test]
    fn ignores_other_ssdp_traffic_and_junk() {
        let router = NOTIFY.replace(
            "urn:bambulab-com:device:3dprinter:1",
            "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
        );
        assert_eq!(parse_notify(router.as_bytes()), None);
        assert_eq!(
            parse_notify(NOTIFY.replace("192.168.1.20", "not-an-ip").as_bytes()),
            None
        );
        assert_eq!(parse_notify(&[0xff, 0xfe, 0x00]), None);
    }

    #[test]
    fn unknown_model_codes_are_shown_as_sent() {
        assert_eq!(model_name("C12"), "P1S");
        assert_eq!(model_name("Z9"), "Z9");
    }

    #[test]
    fn a_bad_serial_is_dropped_and_a_lowercase_one_is_uppercased() {
        for bad in ["09/48", "09 48AB", "../../x", "0948AB;rm", ""] {
            let n = NOTIFY.replace("0948AB000000001", bad);
            assert_eq!(parse_notify(n.as_bytes()), None, "serial {bad:?}");
        }
        let too_long = NOTIFY.replace("0948AB000000001", &"A".repeat(33));
        assert_eq!(parse_notify(too_long.as_bytes()), None);
        let lower = NOTIFY.replace("0948AB000000001", "0948ab000000001");
        assert_eq!(
            parse_notify(lower.as_bytes()).unwrap().serial,
            "0948AB000000001"
        );
    }

    #[test]
    fn names_and_models_are_capped_and_stripped_of_control_characters() {
        let long = "N".repeat(200);
        let n = NOTIFY
            .replace("Workshop H2D", &format!("Bad\u{7}\u{1b}[31m {long}"))
            .replace(": O1D", &format!(": M\u{0}{long}"));
        let p = parse_notify(n.as_bytes()).unwrap();
        assert_eq!(p.name.chars().count(), 64);
        assert!(p.name.starts_with("Bad[31m N"));
        assert!(!p.name.chars().any(char::is_control));
        assert_eq!(p.model.chars().count(), 64);
        assert!(p.model.starts_with("MN"));
    }

    #[test]
    fn unusable_location_addresses_are_dropped() {
        for bad in [
            "0.0.0.0",
            "224.0.0.251",
            "239.255.255.250",
            "255.255.255.255",
            "127.0.0.1",
        ] {
            let n = NOTIFY.replace("192.168.1.20", bad);
            assert_eq!(parse_notify(n.as_bytes()), None, "location {bad}");
        }
        // Loopback is only accepted through the test seam.
        let lo = NOTIFY.replace("192.168.1.20", "127.0.0.1");
        let src: IpAddr = "127.0.0.1".parse().unwrap();
        assert_eq!(parse_announcement(lo.as_bytes(), src, false), None);
        assert!(parse_announcement(lo.as_bytes(), src, true).is_some());
    }

    #[test]
    fn a_location_that_is_not_the_sender_is_dropped() {
        let from_printer: IpAddr = "192.168.1.20".parse().unwrap();
        let from_other: IpAddr = "192.168.1.99".parse().unwrap();
        assert!(parse_announcement(NOTIFY.as_bytes(), from_printer, false).is_some());
        assert_eq!(
            parse_announcement(NOTIFY.as_bytes(), from_other, false),
            None
        );
    }

    #[test]
    fn the_first_announcement_per_serial_wins_and_a_second_address_marks_a_conflict() {
        let mut found = HashMap::new();
        merge(&mut found, printer("192.168.1.20", "AAA"));
        merge(&mut found, printer("192.168.1.20", "AAA"));
        assert!(!found["AAA"].conflict, "same serial from the same address");
        merge(&mut found, printer("192.168.1.66", "AAA"));
        assert_eq!(found.len(), 1);
        assert_eq!(found["AAA"].ip, "192.168.1.20", "the first one is kept");
        assert!(found["AAA"].conflict);
    }

    #[test]
    fn results_are_capped() {
        let mut found = HashMap::new();
        for i in 0..(MAX_DISCOVERED + 10) {
            merge(&mut found, printer("192.168.1.20", &format!("S{i}")));
        }
        assert_eq!(found.len(), MAX_DISCOVERED);
        // A serial already held can still be marked once the list is full.
        merge(&mut found, printer("192.168.1.99", "S0"));
        assert!(found["S0"].conflict);
    }

    #[test]
    fn conflict_defaults_to_false_when_absent() {
        let p: DiscoveredPrinter =
            serde_json::from_str(r#"{"ip":"1.2.3.4","serial":"A","name":"","model":""}"#).unwrap();
        assert!(!p.conflict);
    }

    #[tokio::test]
    async fn collects_announcements_heard_during_the_window() {
        let port = std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        // The sockets are bound before `start` returns, so datagrams sent
        // next are buffered and no sleep is needed.
        let listening = Listening::start(&[port], true).unwrap();
        let from_loopback = NOTIFY.replace("192.168.1.20", "127.0.0.1");
        let forged = NOTIFY.replace("0948AB000000001", "FORGED0000000001");
        let sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        for _ in 0..2 {
            sender
                .send_to(from_loopback.as_bytes(), ("127.0.0.1", port))
                .unwrap();
        }
        // Location says 192.168.1.20 but the datagram came from 127.0.0.1.
        sender
            .send_to(forged.as_bytes(), ("127.0.0.1", port))
            .unwrap();
        let found = listening
            .collect(Duration::from_millis(400))
            .await
            .unwrap()
            .printers;
        assert_eq!(found.len(), 1, "the same printer twice is listed once");
        assert_eq!(found[0].serial, "0948AB000000001");
        assert_eq!(found[0].ip, "127.0.0.1");
        assert!(!found[0].conflict);
    }

    #[tokio::test]
    async fn total_listener_failure_is_an_error_not_an_empty_scan() {
        let result = Listening::start_with(&[2021, 1990, 1900], true, |_, _| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "permission denied",
            ))
        });
        let error = match result {
            Err(e) => e,
            Ok(_) => panic!("scan unexpectedly started"),
        };
        assert!(error.contains("Couldn't listen"));
        assert!(error.contains("local-network access"));
        for port in DISCOVERY_PORTS {
            assert!(error.contains(&format!("UDP {port}")));
        }
    }

    #[tokio::test]
    async fn partial_listener_failure_is_returned_without_blocking_a_healthy_scan() {
        let listening = Listening::start_with(&[2021, 1990], true, |port, _| {
            if port == 2021 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    "address in use",
                ))
            } else {
                // Ephemeral loopback socket; never joins a LAN multicast group.
                UdpSocket::from_std({
                    let socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
                    socket.set_nonblocking(true)?;
                    socket
                })
            }
        })
        .unwrap();
        let report = listening.collect(Duration::ZERO).await.unwrap();
        assert!(report.printers.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("UDP 2021"));
    }

    // A controlled event channel exercises timing without receiving from a
    // real network or waiting 15 wall-clock seconds.
    fn controlled() -> (Listening, mpsc::Sender<DiscoveryEvent>) {
        let (tx, rx) = mpsc::channel(64);
        (
            Listening {
                rx,
                warnings: vec![],
                active: 1,
                _tasks: JoinSet::new(),
            },
            tx,
        )
    }

    #[tokio::test(start_paused = true)]
    async fn an_announcement_after_five_seconds_is_kept_and_traffic_cannot_extend_the_deadline() {
        let (listening, tx) = controlled();
        let started = tokio::time::Instant::now();
        let scan = tokio::spawn(listening.collect(DISCOVERY_WINDOW));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(6)).await;
        assert!(!scan.is_finished());
        tx.send(DiscoveryEvent::Printer(printer("192.168.1.20", "LATE")))
            .await
            .unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(8)).await;
        tx.send(DiscoveryEvent::Printer(printer("192.168.1.21", "LATER")))
            .await
            .unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(1)).await;
        let report = scan.await.unwrap().unwrap();
        assert_eq!(started.elapsed(), DISCOVERY_WINDOW);
        assert_eq!(report.printers.len(), 2);
        assert_eq!(report.listen_seconds, 15);
    }

    #[tokio::test]
    async fn listeners_that_stop_receiving_produce_an_error() {
        let (listening, tx) = controlled();
        tx.send(DiscoveryEvent::Failed(
            "UDP 2021 stopped receiving: permission denied".into(),
        ))
        .await
        .unwrap();
        let error = listening.collect(DISCOVERY_WINDOW).await.unwrap_err();
        assert!(error.contains("stopped receiving"));
    }
}
