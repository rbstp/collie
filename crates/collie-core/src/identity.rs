use std::sync::Arc;

use collie_tls::rustls::pki_types::SubjectPublicKeyInfoDer;
use collie_tls::rustls::sign::{CertifiedKey, Signer, SigningKey};
use collie_tls::rustls::{Error, SignatureAlgorithm, SignatureScheme};

/// The phone's TLS key stays in the app (the Secure Enclave on a device): collie-core
/// only asks it to sign handshakes.
#[uniffi::export(callback_interface)]
pub trait IdentitySigner: Send + Sync {
    /// ECDSA P-256 with SHA-256 over `message`, DER encoded; None when it cannot sign.
    fn sign(&self, message: Vec<u8>) -> Option<Vec<u8>>;
}

pub(crate) fn certified(
    public_key: Vec<u8>,
    signer: Box<dyn IdentitySigner>,
) -> Result<Arc<CertifiedKey>, Error> {
    collie_tls::certified(Arc::new(Foreign {
        public_key,
        signer: Arc::new(signer),
    }))
}

struct Foreign {
    public_key: Vec<u8>,
    signer: Arc<Box<dyn IdentitySigner>>,
}

impl std::fmt::Debug for Foreign {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Foreign")
    }
}

impl SigningKey for Foreign {
    fn choose_scheme(&self, offered: &[SignatureScheme]) -> Option<Box<dyn Signer>> {
        offered
            .contains(&collie_tls::SCHEME)
            .then(|| Box::new(ForeignSigner(self.signer.clone())) as Box<dyn Signer>)
    }

    fn public_key(&self) -> Option<SubjectPublicKeyInfoDer<'_>> {
        Some(self.public_key.as_slice().into())
    }

    fn algorithm(&self) -> SignatureAlgorithm {
        SignatureAlgorithm::ECDSA
    }
}

struct ForeignSigner(Arc<Box<dyn IdentitySigner>>);

impl std::fmt::Debug for ForeignSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ForeignSigner")
    }
}

impl Signer for ForeignSigner {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, Error> {
        self.0
            .sign(message.to_vec())
            .ok_or_else(|| Error::General("the phone's key did not sign".into()))
    }

    fn scheme(&self) -> SignatureScheme {
        collie_tls::SCHEME
    }
}
