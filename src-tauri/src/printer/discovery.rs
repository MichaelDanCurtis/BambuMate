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

fn bind_listener(port: u16) -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // Bambu Studio may already be listening on 2021.
    socket.set_reuse_address(true)?;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into())?;
    if let Err(e) = socket.join_multicast_v4(&SSDP_GROUP, &Ipv4Addr::UNSPECIFIED) {
        tracing::debug!("SSDP multicast join on {port} failed: {e}");
    }
    UdpSocket::from_std(socket.into())
}

async fn listen_on(socket: UdpSocket, tx: mpsc::Sender<DiscoveredPrinter>, allow_loopback: bool) {
    let mut buf = vec![0u8; 2048];
    let mut errors = 0;
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((n, from)) => {
                errors = 0;
                if let Some(p) = parse_announcement(&buf[..n], from.ip(), allow_loopback) {
                    if tx.send(p).await.is_err() {
                        return;
                    }
                }
            }
            Err(e) => {
                errors += 1;
                if errors > MAX_RECV_ERRORS {
                    tracing::debug!("giving up listening for printers: {e}");
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// Sockets bound and listening. Dropping this stops the listeners.
struct Listening {
    rx: mpsc::Receiver<DiscoveredPrinter>,
    _tasks: JoinSet<()>,
}

impl Listening {
    /// Binds every port in `ports` that can be bound, before returning, so
    /// anything sent afterwards is buffered by the socket.
    fn start(ports: &[u16], allow_loopback: bool) -> Self {
        let (tx, rx) = mpsc::channel::<DiscoveredPrinter>(64);
        let mut tasks = JoinSet::new();
        for &port in ports {
            match bind_listener(port) {
                Ok(socket) => {
                    tasks.spawn(listen_on(socket, tx.clone(), allow_loopback));
                }
                Err(e) => tracing::debug!("cannot listen for printers on UDP {port}: {e}"),
            }
        }
        Self { rx, _tasks: tasks }
    }

    /// Collects announcements for `window`, sorted by name.
    async fn collect(mut self, window: Duration) -> Vec<DiscoveredPrinter> {
        let mut found: HashMap<String, DiscoveredPrinter> = HashMap::new();
        let deadline = tokio::time::Instant::now() + window;
        while let Ok(Some(p)) = tokio::time::timeout_at(deadline, self.rx.recv()).await {
            merge(&mut found, p);
        }
        let mut out: Vec<DiscoveredPrinter> = found.into_values().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.serial.cmp(&b.serial)));
        out
    }
}

/// Listens on `ports` for `window` and returns each printer heard, once,
/// sorted by name, at most [`MAX_DISCOVERED`] of them. Ports that can't be
/// bound are skipped.
pub async fn discover(ports: &[u16], window: Duration) -> Vec<DiscoveredPrinter> {
    Listening::start(ports, false).collect(window).await
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
        let listening = Listening::start(&[port], true);
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
        let found = listening.collect(Duration::from_millis(400)).await;
        assert_eq!(found.len(), 1, "the same printer twice is listed once");
        assert_eq!(found[0].serial, "0948AB000000001");
        assert_eq!(found[0].ip, "127.0.0.1");
        assert!(!found[0].conflict);
    }
}
