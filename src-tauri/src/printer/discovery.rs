//! Finds Bambu printers on the local network by listening for the SSDP
//! `NOTIFY` messages they multicast. Discovery only listens; it sends nothing.
//!
//! Printers announce `NT: urn:bambulab-com:device:3dprinter:1` with the IP in
//! `Location`, the serial in `USN`, and `DevName.bambu.com` /
//! `DevModel.bambu.com` headers, to UDP 2021 and 1990 (some sources say
//! 1900, so all three are joined).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

pub const DISCOVERY_PORTS: &[u16] = &[2021, 1990, 1900];
const SSDP_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const BAMBU_URN: &str = "urn:bambulab-com:device:3dprinter";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPrinter {
    pub ip: String,
    pub serial: String,
    pub name: String,
    pub model: String,
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

/// Parses one SSDP datagram. `None` unless it is a Bambu printer
/// announcement with an IP address and a serial.
pub fn parse_notify(datagram: &[u8]) -> Option<DiscoveredPrinter> {
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
    let serial = headers.get("usn")?.trim().to_string();
    if serial.is_empty() {
        return None;
    }
    Some(DiscoveredPrinter {
        ip: ip.to_string(),
        name: headers
            .get("devname.bambu.com")
            .cloned()
            .unwrap_or_default(),
        model: headers
            .get("devmodel.bambu.com")
            .map(|m| model_name(m))
            .unwrap_or_default(),
        serial,
    })
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

/// Listens on `ports` for `window` and returns each printer heard, once,
/// sorted by name. Ports that can't be bound are skipped.
pub async fn discover(ports: &[u16], window: Duration) -> Vec<DiscoveredPrinter> {
    let (tx, mut rx) = mpsc::channel::<DiscoveredPrinter>(64);
    let mut tasks = Vec::new();
    for &port in ports {
        match bind_listener(port) {
            Ok(socket) => {
                let tx = tx.clone();
                tasks.push(tokio::spawn(async move {
                    let mut buf = vec![0u8; 2048];
                    while let Ok((n, _)) = socket.recv_from(&mut buf).await {
                        if let Some(p) = parse_notify(&buf[..n]) {
                            if tx.send(p).await.is_err() {
                                return;
                            }
                        }
                    }
                }));
            }
            Err(e) => tracing::debug!("cannot listen for printers on UDP {port}: {e}"),
        }
    }
    drop(tx);
    let mut found: HashMap<String, DiscoveredPrinter> = HashMap::new();
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Some(p)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        found.insert(p.serial.clone(), p);
    }
    for t in tasks {
        t.abort();
    }
    let mut out: Vec<DiscoveredPrinter> = found.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.serial.cmp(&b.serial)));
    out
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

    #[test]
    fn parses_a_bambu_notify() {
        assert_eq!(
            parse_notify(NOTIFY.as_bytes()),
            Some(DiscoveredPrinter {
                ip: "192.168.1.20".into(),
                serial: "0948AB000000001".into(),
                name: "Workshop H2D".into(),
                model: "H2D".into(),
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

    #[tokio::test]
    async fn collects_announcements_heard_during_the_window() {
        let port = std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let listen =
            tokio::spawn(async move { discover(&[port], Duration::from_millis(600)).await });
        tokio::time::sleep(Duration::from_millis(150)).await;
        let sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        for _ in 0..2 {
            sender
                .send_to(NOTIFY.as_bytes(), ("127.0.0.1", port))
                .unwrap();
        }
        let found = listen.await.unwrap();
        assert_eq!(found.len(), 1, "the same printer twice is listed once");
        assert_eq!(found[0].serial, "0948AB000000001");
    }
}
