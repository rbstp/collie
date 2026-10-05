//! TLS 1.3 with raw public keys (RFC 7250): each side pins the other's P-256 key by the
//! SHA-256 of its SubjectPublicKeyInfo.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use protocol::KeyPin;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls13_signature_with_raw_key};
use rustls::pki_types::{CertificateDer, ServerName, SubjectPublicKeyInfoDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::sign::CertifiedKey;
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, DistinguishedName, Error,
    PeerIncompatible, ServerConfig, SignatureScheme,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::{TlsAcceptor, TlsConnector};

pub use rustls;
pub use tokio_rustls::{client, server};

pub const SCHEME: SignatureScheme = SignatureScheme::ECDSA_NISTP256_SHA256;
const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a,
    0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];
const P256_SPKI_LEN: usize = 91;

pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub fn pin(spki: &[u8]) -> KeyPin {
    KeyPin::new(URL_SAFE_NO_PAD.encode(Sha256::digest(spki))).expect("a SHA-256 is 43 chars")
}

pub fn is_p256(spki: &[u8]) -> bool {
    spki.len() == P256_SPKI_LEN && spki.starts_with(&P256_SPKI_PREFIX)
}

pub fn certified(key: Arc<dyn rustls::sign::SigningKey>) -> Result<Arc<CertifiedKey>, Error> {
    let spki = key
        .public_key()
        .filter(|s| is_p256(s.as_ref()))
        .ok_or(Error::General("not a P-256 key".into()))?;
    let cert = CertificateDer::from(spki.as_ref().to_vec());
    Ok(Arc::new(CertifiedKey::new(vec![cert], key)))
}

/// `expect` is None only in a pairing session, which records the phone's key. No
/// resumption: a resumed session would skip the key check.
pub fn server_config(key: Arc<CertifiedKey>, expect: Option<KeyPin>) -> ServerConfig {
    let mut config = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("ring supports TLS 1.3")
        .with_client_cert_verifier(Arc::new(Pinned(expect)))
        .with_cert_resolver(Arc::new(
            rustls::server::AlwaysResolvesServerRawPublicKeys::new(key),
        ));
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    config
}

pub fn client_config(expect: KeyPin, identity: Arc<CertifiedKey>) -> ClientConfig {
    let mut config = ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("ring supports TLS 1.3")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned(Some(expect))))
        .with_client_cert_resolver(Arc::new(
            rustls::client::AlwaysResolvesClientRawPublicKeys::new(identity),
        ));
    config.resumption = rustls::client::Resumption::disabled();
    config
}

pub fn generate() -> Result<Vec<u8>, Error> {
    let rng = ring::rand::SystemRandom::new();
    ring::signature::EcdsaKeyPair::generate_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &rng,
    )
    .map(|k| k.as_ref().to_vec())
    .map_err(|_| Error::General("key generation failed".into()))
}

pub fn load(pkcs8: &[u8]) -> Result<Arc<dyn rustls::sign::SigningKey>, Error> {
    provider()
        .key_provider
        .load_private_key(rustls::pki_types::PrivateKeyDer::Pkcs8(
            pkcs8.to_vec().into(),
        ))
}

pub fn peer_pin(conn: &rustls::CommonState) -> Option<KeyPin> {
    conn.peer_certificates()
        .and_then(|c| c.first())
        .map(|c| pin(c.as_ref()))
}

#[derive(Debug)]
pub enum ConnectError {
    /// collied's fixed refusal, sent in clear before any TLS to a peer the gate rejects.
    Forbidden,
    Tls(std::io::Error),
}

pub async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    host: &str,
    expect: KeyPin,
    identity: Arc<CertifiedKey>,
) -> Result<client::TlsStream<Sniff<S>>, ConnectError> {
    let name = ServerName::try_from(host.to_owned())
        .unwrap_or_else(|_| ServerName::try_from("collie").expect("a valid name"));
    let connector = TlsConnector::from(Arc::new(client_config(expect, identity)));
    match connector
        .connect(name, Sniff::new(stream))
        .into_fallible()
        .await
    {
        Ok(s) => Ok(s),
        Err((_, sniff)) if sniff.forbidden() => Err(ConnectError::Forbidden),
        Err((e, _)) => Err(ConnectError::Tls(e)),
    }
}

pub async fn accept<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    key: Arc<CertifiedKey>,
    expect: Option<KeyPin>,
) -> std::io::Result<(server::TlsStream<S>, KeyPin)> {
    let acceptor = TlsAcceptor::from(Arc::new(server_config(key, expect)));
    let stream = acceptor.accept(stream).await?;
    let presented =
        peer_pin(stream.get_ref().1).ok_or_else(|| std::io::Error::other("no client key"))?;
    Ok((stream, presented))
}

const SNIFFED: usize = 12;

/// Keeps the first bytes read: a failed handshake may be collied's clear 403.
#[derive(Debug)]
pub struct Sniff<S> {
    inner: S,
    head: Vec<u8>,
}

impl<S> Sniff<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            head: Vec::with_capacity(SNIFFED),
        }
    }

    fn forbidden(&self) -> bool {
        self.head == b"HTTP/1.1 403"
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Sniff<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        ready!(Pin::new(&mut self.inner).poll_read(cx, buf))?;
        let room = SNIFFED - self.head.len();
        let read: Vec<u8> = buf.filled()[before..].iter().take(room).copied().collect();
        self.head.extend(read);
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Sniff<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[derive(Debug)]
struct Pinned(Option<KeyPin>);

impl Pinned {
    fn check(&self, presented: &CertificateDer<'_>) -> Result<(), Error> {
        if !is_p256(presented.as_ref()) {
            return Err(Error::InvalidCertificate(CertificateError::BadEncoding));
        }
        match &self.0 {
            Some(expected) if pin(presented.as_ref()) != *expected => Err(
                Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure),
            ),
            _ => Ok(()),
        }
    }

    fn signature(
        message: &[u8],
        key: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        if dss.scheme != SCHEME {
            return Err(PeerIncompatible::NoSignatureSchemesInCommon.into());
        }
        verify_tls13_signature_with_raw_key(
            message,
            &SubjectPublicKeyInfoDer::from(key.as_ref()),
            dss,
            &provider().signature_verification_algorithms,
        )
    }
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        if !intermediates.is_empty() {
            return Err(Error::InvalidCertificate(CertificateError::BadEncoding));
        }
        self.check(end_entity)
            .map(|()| ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.2 is not offered".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Self::signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SCHEME]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

impl ClientCertVerifier for Pinned {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        if !intermediates.is_empty() {
            return Err(Error::InvalidCertificate(CertificateError::BadEncoding));
        }
        self.check(end_entity)
            .map(|()| ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Err(Error::General("TLS 1.2 is not offered".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Self::signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SCHEME]
    }

    fn requires_raw_public_keys(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::{ClientConnection, ServerConnection};

    fn key() -> (Arc<CertifiedKey>, KeyPin) {
        let key = certified(load(&generate().unwrap()).unwrap()).unwrap();
        let pin = pin(key.cert[0].as_ref());
        (key, pin)
    }

    fn handshake(client: ClientConfig, server: ServerConfig) -> Result<Option<KeyPin>, Error> {
        let name = ServerName::try_from("collie.example").unwrap();
        let mut c = ClientConnection::new(Arc::new(client), name)?;
        let mut s = ServerConnection::new(Arc::new(server))?;
        for _ in 0..10 {
            let mut buf = Vec::new();
            c.write_tls(&mut buf).unwrap();
            s.read_tls(&mut &buf[..]).unwrap();
            s.process_new_packets()?;
            let mut buf = Vec::new();
            s.write_tls(&mut buf).unwrap();
            c.read_tls(&mut &buf[..]).unwrap();
            c.process_new_packets()?;
            if !c.is_handshaking() && !s.is_handshaking() {
                return Ok(peer_pin(&s));
            }
        }
        panic!("handshake did not finish");
    }

    #[test]
    fn both_sides_pin_the_other_key() {
        let (machine, machine_pin) = key();
        let (phone, phone_pin) = key();
        let seen = handshake(
            client_config(machine_pin.clone(), phone.clone()),
            server_config(machine.clone(), Some(phone_pin.clone())),
        )
        .unwrap();
        assert_eq!(seen, Some(phone_pin.clone()));

        let paired = handshake(
            client_config(machine_pin.clone(), phone.clone()),
            server_config(machine.clone(), None),
        )
        .unwrap();
        assert_eq!(paired, Some(phone_pin.clone()), "pairing records the key");

        let (other, other_pin) = key();
        assert!(
            handshake(
                client_config(machine_pin.clone(), other.clone()),
                server_config(machine.clone(), Some(phone_pin.clone())),
            )
            .is_err(),
            "the machine rejects an unpinned phone key"
        );
        assert!(
            handshake(
                client_config(machine_pin, phone.clone()),
                server_config(other, Some(phone_pin.clone())),
            )
            .is_err(),
            "the phone rejects an unpinned machine key"
        );
        assert!(
            handshake(
                client_config(other_pin, phone),
                server_config(machine, Some(phone_pin)),
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn streams_connect_and_see_a_clear_refusal() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (machine, machine_pin) = key();
        let (phone, phone_pin) = key();
        let (a, b) = tokio::io::duplex(4096);
        let server = tokio::spawn(accept(b, machine, Some(phone_pin.clone())));
        let mut c = connect(a, "collie.example", machine_pin.clone(), phone.clone())
            .await
            .unwrap();
        let (mut s, seen) = server.await.unwrap().unwrap();
        assert_eq!(seen, phone_pin);
        c.write_all(b"ping").await.unwrap();
        c.flush().await.unwrap();
        let mut got = [0; 4];
        s.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"ping");

        let (a, mut b) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            b.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let refused = connect(a, "collie.example", machine_pin.clone(), phone.clone()).await;
        assert!(matches!(refused, Err(ConnectError::Forbidden)));

        let (a, mut b) = tokio::io::duplex(4096);
        tokio::spawn(async move { b.write_all(b"garbage that is not TLS").await });
        let broken = connect(a, "collie.example", machine_pin, phone).await;
        assert!(matches!(broken, Err(ConnectError::Tls(_))));
    }

    #[test]
    fn only_p256_keys() {
        let (k, _) = key();
        assert!(is_p256(k.cert[0].as_ref()));
        let mut spki = k.cert[0].as_ref().to_vec();
        spki[12] ^= 1;
        assert!(!is_p256(&spki));
        assert!(!is_p256(&spki[..90]));
        assert_eq!(pin(b"x").as_str().len(), 43);
    }
}
