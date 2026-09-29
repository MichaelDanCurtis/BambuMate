//! TLS verification for the printer's MQTT broker.
//!
//! A printer presents a certificate whose CN is its serial number, not its
//! IP address, so the usual hostname check is replaced by a CN check. The
//! chain must lead to one of Bambu's printer CAs, bundled from
//! `src-tauri/resources/bambu-ca/`. When it doesn't (for example a model
//! with a CA we don't ship), the user may pin that exact certificate by its
//! SHA-256 fingerprint. The CN check applies either way. There is no option
//! that skips verification.

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme,
};
use sha2::{Digest, Sha256};

/// Bambu's printer CA bundle: BBL CA, BBL CA2 RSA and BBL CA2 ECC.
/// Source: bambulab/BambuStudio `resources/cert/printer.cer`.
const BAMBU_PRINTER_CA_PEM: &str = include_str!("../../resources/bambu-ca/bambu-printer-ca.pem");

/// Per-model intermediate CAs some printers don't send in their chain. All
/// are signed by BBL CA2 RSA, so they can't widen trust beyond Bambu's CAs.
/// Source: greghesp/ha-bambulab `pybambu/certs/`.
const BAMBU_DEVICE_CA_PEMS: &[&str] = &[
    include_str!("../../resources/bambu-ca/bbl-device-ca-n6-v2.pem"),
    include_str!("../../resources/bambu-ca/bbl-device-ca-n7-v2.pem"),
    include_str!("../../resources/bambu-ca/bbl-device-ca-o1c2-v2.pem"),
];

/// The only crypto provider BambuMate's printer code uses. Passed to every
/// config explicitly, so no process-wide default provider is needed.
pub fn crypto_provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Why the verifier refused a printer's certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// Not signed by a bundled CA and not the pinned certificate.
    Untrusted { fingerprint: String },
    /// The certificate belongs to a different printer.
    WrongSerial { presented: String },
}

/// Where the verifier records its last rejection, so the client can tell a
/// certificate problem apart from a network one after the handshake fails.
#[derive(Debug, Clone, Default)]
pub struct RejectionSlot(Arc<Mutex<Option<Rejection>>>);

impl RejectionSlot {
    fn set(&self, r: Rejection) {
        *self.0.lock().unwrap() = Some(r);
    }
    /// Returns and clears the last rejection.
    pub fn take(&self) -> Option<Rejection> {
        self.0.lock().unwrap().take()
    }
}

/// `AB:CD:…`, uppercase: the SHA-256 of the DER certificate.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Uppercase hex with separators removed, for comparing fingerprints.
pub fn normalize_fingerprint(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// The subject CN of a DER certificate.
pub fn leaf_common_name(der: &[u8]) -> Option<String> {
    let (_, cert) = x509_parser::parse_x509_certificate(der).ok()?;
    let cn = cert.subject().iter_common_name().next()?;
    cn.as_str().ok().map(str::to_string)
}

fn certs_from_pem(pem: &str) -> Result<Vec<CertificateDer<'static>>, String> {
    CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("bad certificate PEM: {e}"))
}

#[derive(Debug)]
pub struct PrinterCertVerifier {
    roots: RootCertStore,
    extra_intermediates: Vec<CertificateDer<'static>>,
    serial: String,
    pinned: Option<String>,
    provider: Arc<CryptoProvider>,
    rejection: RejectionSlot,
}

impl PrinterCertVerifier {
    /// A verifier that trusts Bambu's bundled printer CAs.
    pub fn bambu(
        serial: &str,
        pinned_fingerprint: Option<&str>,
        rejection: RejectionSlot,
    ) -> Result<Self, String> {
        Self::with_trust(
            BAMBU_PRINTER_CA_PEM,
            BAMBU_DEVICE_CA_PEMS,
            serial,
            pinned_fingerprint,
            rejection,
        )
    }

    /// A verifier over explicit CAs. Tests use it with a generated CA.
    pub fn with_trust(
        roots_pem: &str,
        intermediates_pem: &[&str],
        serial: &str,
        pinned_fingerprint: Option<&str>,
        rejection: RejectionSlot,
    ) -> Result<Self, String> {
        let serial = serial.trim();
        if serial.is_empty() {
            return Err("the printer serial is empty".into());
        }
        let mut roots = RootCertStore::empty();
        let (added, ignored) = roots.add_parsable_certificates(certs_from_pem(roots_pem)?);
        if added == 0 {
            return Err("no usable CA certificate".into());
        }
        if ignored > 0 {
            return Err(format!("{ignored} CA certificate(s) could not be parsed"));
        }
        let mut extra_intermediates = Vec::new();
        for pem in intermediates_pem {
            extra_intermediates.extend(certs_from_pem(pem)?);
        }
        Ok(Self {
            roots,
            extra_intermediates,
            serial: serial.to_string(),
            pinned: pinned_fingerprint
                .map(normalize_fingerprint)
                .filter(|p| !p.is_empty()),
            provider: crypto_provider(),
            rejection,
        })
    }
}

impl ServerCertVerifier for PrinterCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        // A rejection from an earlier handshake must not be mistaken for this one.
        self.rejection.take();

        // The leaf must name this printer, whatever else vouches for it. A
        // missing or unreadable CN never matches.
        match leaf_common_name(end_entity) {
            Some(cn) if cn == self.serial => {}
            other => {
                self.rejection.set(Rejection::WrongSerial {
                    presented: other.unwrap_or_default(),
                });
                return Err(Error::InvalidCertificate(CertificateError::NotValidForName));
            }
        }

        let fp = fingerprint(end_entity);
        let untrusted = |err: Error| {
            self.rejection.set(Rejection::Untrusted {
                fingerprint: fp.clone(),
            });
            err
        };

        // Pinned mode is trust-on-first-use of this exact certificate, so it
        // deliberately skips chain building and the validity window: printers
        // have no reliable clock and often carry self-issued certificates.
        // The CN check above still applies.
        if self.pinned.as_deref() == Some(normalize_fingerprint(&fp).as_str()) {
            return Ok(ServerCertVerified::assertion());
        }

        let parsed = ParsedCertificate::try_from(end_entity).map_err(untrusted)?;
        let mut chain: Vec<CertificateDer<'_>> = intermediates.to_vec();
        chain.extend(self.extra_intermediates.iter().cloned());
        rustls::client::verify_server_cert_signed_by_trust_anchor(
            &parsed,
            &self.roots,
            &chain,
            now,
            self.provider.signature_verification_algorithms.all,
        )
        .map_err(untrusted)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The client config for one printer. TLS 1.2 only: some Bambu firmware
/// never answers a TLS 1.3 ClientHello (ha-bambulab caps it the same way).
pub fn client_config(verifier: Arc<PrinterCertVerifier>) -> Result<Arc<ClientConfig>, String> {
    let config = ClientConfig::builder_with_provider(crypto_provider())
        .with_protocol_versions(&[&rustls::version::TLS12])
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

#[cfg(test)]
pub(crate) mod testpki {
    //! Generated CAs and printer certificates for tests.

    use rcgen::{
        date_time_ymd, BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa,
        Issuer, KeyPair, KeyUsagePurpose,
    };

    pub struct TestCa {
        pub pem: String,
        pub der: Vec<u8>,
        params: CertificateParams,
        key: KeyPair,
    }

    pub struct TestLeaf {
        pub cert_der: Vec<u8>,
        pub key_pem: String,
    }

    fn ca_params(name: &str) -> CertificateParams {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.distinguished_name.push(DnType::CommonName, name);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params
    }

    impl TestCa {
        pub fn new(name: &str) -> Self {
            let params = ca_params(name);
            let key = KeyPair::generate().unwrap();
            let cert = params.self_signed(&key).unwrap();
            Self {
                pem: cert.pem(),
                der: cert.der().to_vec(),
                params,
                key,
            }
        }

        /// An intermediate CA signed by this CA.
        pub fn intermediate(&self, name: &str) -> TestCa {
            let params = ca_params(name);
            let key = KeyPair::generate().unwrap();
            let issuer = Issuer::from_params(&self.params, &self.key);
            let cert = params.signed_by(&key, &issuer).unwrap();
            TestCa {
                pem: cert.pem(),
                der: cert.der().to_vec(),
                params,
                key,
            }
        }

        /// A printer certificate: CN = serial, no DNS or IP names.
        pub fn leaf(&self, serial: &str) -> TestLeaf {
            let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
            params.distinguished_name.push(DnType::CommonName, serial);
            self.sign_leaf(params)
        }

        /// A printer certificate with no subject CN at all.
        pub fn leaf_without_cn(&self) -> TestLeaf {
            let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
            params.distinguished_name = DistinguishedName::new();
            self.sign_leaf(params)
        }

        /// A printer certificate whose validity ended long ago.
        pub fn expired_leaf(&self, serial: &str) -> TestLeaf {
            let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
            params.distinguished_name.push(DnType::CommonName, serial);
            params.not_before = date_time_ymd(2000, 1, 1);
            params.not_after = date_time_ymd(2001, 1, 1);
            self.sign_leaf(params)
        }

        fn sign_leaf(&self, params: CertificateParams) -> TestLeaf {
            let key = KeyPair::generate().unwrap();
            let issuer = Issuer::from_params(&self.params, &self.key);
            let cert = params.signed_by(&key, &issuer).unwrap();
            TestLeaf {
                cert_der: cert.der().to_vec(),
                key_pem: key.serialize_pem(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testpki::TestCa;
    use super::*;

    const SERIAL: &str = "0948AB000000001";

    fn verify(v: &PrinterCertVerifier, leaf_der: &[u8]) -> Result<ServerCertVerified, Error> {
        verify_sent(v, leaf_der, &[])
    }

    /// Like `verify`, with intermediates the printer sends in its handshake.
    fn verify_sent(
        v: &PrinterCertVerifier,
        leaf_der: &[u8],
        sent: &[Vec<u8>],
    ) -> Result<ServerCertVerified, Error> {
        let sent: Vec<CertificateDer<'static>> = sent
            .iter()
            .map(|d| CertificateDer::from(d.clone()))
            .collect();
        v.verify_server_cert(
            &CertificateDer::from(leaf_der.to_vec()),
            &sent,
            &ServerName::try_from("192.168.1.20").unwrap(),
            &[],
            UnixTime::now(),
        )
    }

    #[test]
    fn accepts_a_chain_from_a_trusted_ca_with_the_right_cn() {
        let ca = TestCa::new("Test Printer CA");
        let leaf = ca.leaf(SERIAL);
        let slot = RejectionSlot::default();
        let v = PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, slot.clone()).unwrap();
        assert!(verify(&v, &leaf.cert_der).is_ok());
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn rejects_a_certificate_for_another_serial() {
        let ca = TestCa::new("Test Printer CA");
        let leaf = ca.leaf("01P00A000000002");
        let slot = RejectionSlot::default();
        let v = PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, slot.clone()).unwrap();
        assert!(verify(&v, &leaf.cert_der).is_err());
        assert_eq!(
            slot.take(),
            Some(Rejection::WrongSerial {
                presented: "01P00A000000002".into()
            })
        );
    }

    #[test]
    fn rejects_an_unknown_ca_and_reports_the_fingerprint() {
        let trusted = TestCa::new("Test Printer CA");
        let other = TestCa::new("Somebody Else");
        let leaf = other.leaf(SERIAL);
        let slot = RejectionSlot::default();
        let v =
            PrinterCertVerifier::with_trust(&trusted.pem, &[], SERIAL, None, slot.clone()).unwrap();
        assert!(verify(&v, &leaf.cert_der).is_err());
        assert_eq!(
            slot.take(),
            Some(Rejection::Untrusted {
                fingerprint: fingerprint(&leaf.cert_der)
            })
        );
    }

    #[test]
    fn accepts_the_pinned_certificate_from_an_unknown_ca() {
        let trusted = TestCa::new("Test Printer CA");
        let other = TestCa::new("Newer Bambu CA");
        let leaf = other.leaf(SERIAL);
        let pin = fingerprint(&leaf.cert_der).to_lowercase();
        let v = PrinterCertVerifier::with_trust(
            &trusted.pem,
            &[],
            SERIAL,
            Some(&pin),
            RejectionSlot::default(),
        )
        .unwrap();
        assert!(verify(&v, &leaf.cert_der).is_ok());
    }

    #[test]
    fn rejects_a_changed_certificate_despite_a_pin() {
        let trusted = TestCa::new("Test Printer CA");
        let other = TestCa::new("Newer Bambu CA");
        let old_leaf = other.leaf(SERIAL);
        let new_leaf = other.leaf(SERIAL);
        let slot = RejectionSlot::default();
        let v = PrinterCertVerifier::with_trust(
            &trusted.pem,
            &[],
            SERIAL,
            Some(&fingerprint(&old_leaf.cert_der)),
            slot.clone(),
        )
        .unwrap();
        assert!(verify(&v, &new_leaf.cert_der).is_err());
        assert_eq!(
            slot.take(),
            Some(Rejection::Untrusted {
                fingerprint: fingerprint(&new_leaf.cert_der)
            })
        );
    }

    #[test]
    fn a_pin_does_not_skip_the_cn_check() {
        let trusted = TestCa::new("Test Printer CA");
        let leaf = TestCa::new("Other").leaf("SOMEONE-ELSE");
        let slot = RejectionSlot::default();
        let v = PrinterCertVerifier::with_trust(
            &trusted.pem,
            &[],
            SERIAL,
            Some(&fingerprint(&leaf.cert_der)),
            slot.clone(),
        )
        .unwrap();
        assert!(verify(&v, &leaf.cert_der).is_err());
        assert_eq!(
            slot.take(),
            Some(Rejection::WrongSerial {
                presented: "SOMEONE-ELSE".into()
            })
        );
    }

    #[test]
    fn rejects_a_certificate_with_no_common_name() {
        let ca = TestCa::new("Test Printer CA");
        let leaf = ca.leaf_without_cn();
        assert_eq!(leaf_common_name(&leaf.cert_der), None);
        let slot = RejectionSlot::default();
        let v = PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, slot.clone()).unwrap();
        assert!(verify(&v, &leaf.cert_der).is_err());
        assert_eq!(
            slot.take(),
            Some(Rejection::WrongSerial {
                presented: String::new()
            })
        );
    }

    #[test]
    fn a_pinned_certificate_with_no_common_name_is_still_rejected() {
        let ca = TestCa::new("Test Printer CA");
        let leaf = ca.leaf_without_cn();
        let v = PrinterCertVerifier::with_trust(
            &ca.pem,
            &[],
            SERIAL,
            Some(&fingerprint(&leaf.cert_der)),
            RejectionSlot::default(),
        )
        .unwrap();
        assert!(verify(&v, &leaf.cert_der).is_err());
    }

    #[test]
    fn an_empty_serial_is_refused() {
        let ca = TestCa::new("Test Printer CA");
        for serial in ["", "   "] {
            let r = PrinterCertVerifier::with_trust(
                &ca.pem,
                &[],
                serial,
                None,
                RejectionSlot::default(),
            );
            assert!(r.is_err(), "{serial:?} must be refused");
        }
        assert!(PrinterCertVerifier::bambu("", None, RejectionSlot::default()).is_err());
    }

    #[test]
    fn an_unparsable_root_is_an_error_not_silently_dropped() {
        let ca = TestCa::new("Test Printer CA");
        let junk = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n";
        let pem = format!("{}{junk}", ca.pem);
        let r = PrinterCertVerifier::with_trust(&pem, &[], SERIAL, None, RejectionSlot::default());
        assert!(r.is_err());
    }

    #[test]
    fn accepts_a_chain_through_an_intermediate_the_printer_sends() {
        let root = TestCa::new("Test Root");
        let inter = root.intermediate("Test Device CA");
        let leaf = inter.leaf(SERIAL);
        let v =
            PrinterCertVerifier::with_trust(&root.pem, &[], SERIAL, None, RejectionSlot::default())
                .unwrap();
        assert!(verify(&v, &leaf.cert_der).is_err(), "not sent, not known");
        assert!(verify_sent(&v, &leaf.cert_der, std::slice::from_ref(&inter.der)).is_ok());
    }

    #[test]
    fn accepts_a_chain_through_a_bundled_intermediate_the_printer_omits() {
        let root = TestCa::new("Test Root");
        let inter = root.intermediate("Test Device CA");
        let leaf = inter.leaf(SERIAL);
        let v = PrinterCertVerifier::with_trust(
            &root.pem,
            &[&inter.pem],
            SERIAL,
            None,
            RejectionSlot::default(),
        )
        .unwrap();
        assert!(verify(&v, &leaf.cert_der).is_ok());
    }

    #[test]
    fn a_bundled_intermediate_is_not_a_trust_anchor() {
        let other_root = TestCa::new("Other Root");
        let inter = other_root.intermediate("Stray Device CA");
        let leaf = inter.leaf(SERIAL);
        let trusted = TestCa::new("Test Printer CA");
        let slot = RejectionSlot::default();
        let v = PrinterCertVerifier::with_trust(
            &trusted.pem,
            &[&inter.pem],
            SERIAL,
            None,
            slot.clone(),
        )
        .unwrap();
        assert!(verify(&v, &leaf.cert_der).is_err());
        assert!(matches!(slot.take(), Some(Rejection::Untrusted { .. })));
    }

    #[test]
    fn rejects_an_expired_certificate() {
        let ca = TestCa::new("Test Printer CA");
        let leaf = ca.expired_leaf(SERIAL);
        let slot = RejectionSlot::default();
        let v = PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, slot.clone()).unwrap();
        assert!(verify(&v, &leaf.cert_der).is_err());
        assert!(matches!(slot.take(), Some(Rejection::Untrusted { .. })));
    }

    #[test]
    fn a_pin_accepts_an_expired_certificate() {
        // Trust on first use of an exact certificate skips the validity window.
        let ca = TestCa::new("Test Printer CA");
        let other = TestCa::new("Unbundled CA");
        let leaf = other.expired_leaf(SERIAL);
        let v = PrinterCertVerifier::with_trust(
            &ca.pem,
            &[],
            SERIAL,
            Some(&fingerprint(&leaf.cert_der)),
            RejectionSlot::default(),
        )
        .unwrap();
        assert!(verify(&v, &leaf.cert_der).is_ok());
    }

    #[test]
    fn a_stale_rejection_is_cleared_by_the_next_handshake() {
        let ca = TestCa::new("Test Printer CA");
        let bad = ca.leaf("01P00A000000002");
        let good = ca.leaf(SERIAL);
        let slot = RejectionSlot::default();
        let v = PrinterCertVerifier::with_trust(&ca.pem, &[], SERIAL, None, slot.clone()).unwrap();
        assert!(verify(&v, &bad.cert_der).is_err());
        assert!(verify(&v, &good.cert_der).is_ok());
        assert_eq!(slot.take(), None);
    }

    #[test]
    fn the_bundled_bambu_cas_load() {
        let v = PrinterCertVerifier::bambu(SERIAL, None, RejectionSlot::default()).unwrap();
        // BBL CA, plus BBL CA2 RSA and ECC each self-signed and cross-signed.
        assert_eq!(v.roots.len(), 5);
        assert_eq!(v.extra_intermediates.len(), 3);
        for der in &v.extra_intermediates {
            let cn = leaf_common_name(der).unwrap();
            assert!(cn.starts_with("BBL Device CA"), "{cn}");
        }
    }

    #[test]
    fn fingerprints_are_colon_separated_uppercase_sha256() {
        let fp = fingerprint(b"abc");
        assert_eq!(
            fp,
            "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23:B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD"
        );
        assert_eq!(
            normalize_fingerprint("ba:78 16"),
            normalize_fingerprint("BA7816")
        );
    }

    #[test]
    fn the_client_config_builds_with_the_ring_provider() {
        let v = PrinterCertVerifier::bambu(SERIAL, None, RejectionSlot::default()).unwrap();
        assert!(client_config(Arc::new(v)).is_ok());
    }
}
