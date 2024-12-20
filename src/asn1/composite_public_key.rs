use der::{asn1::BitString, Decode, Encode};
use der_derive::Sequence;

use crate::QubitCryptError;

type Result<T> = std::result::Result<T, QubitCryptError>;

/// CompositeSignaturePublicKey ::= SEQUENCE SIZE (2) OF BIT STRING
/// CompositeKEMPublicKey ::= SEQUENCE SIZE (2) OF BIT STRING
#[derive(Debug, Clone, Sequence)]
struct CompositeSigKemPublicKey {
    pq_pk: BitString,
    trad_pk: BitString,
}

#[derive(Debug, Clone)]
/// A public key for a composite DSA / KEM
pub struct CompositePublicKey {
    /// The OID for the composite DSA / KEM
    oid: String,
    /// The public key for the post-quantum DSA / KEM
    pq_pk: Vec<u8>,
    /// The public key for the traditional DSA / KEM
    trad_pk: Vec<u8>,
}

impl CompositePublicKey {
    /// Create a new composite public key
    ///
    /// # Arguments
    ///
    /// * `oid` - The OID for the composite DSA
    /// * `pq_pk` - The public key for the post-quantum DSA / KEM
    /// * `trad_pk` - The public key for the traditional DSA / KEM
    ///
    /// # Returns
    ///
    /// A new composite DSA / KEM public key
    pub fn new(oid: &str, pq_pk: &[u8], trad_pk: &[u8]) -> Self {
        Self {
            oid: oid.to_string(),
            pq_pk: pq_pk.to_vec(),
            trad_pk: trad_pk.to_vec(),
        }
    }

    /// Get the OID for the composite DSA / KEM
    ///
    /// # Returns
    ///
    /// The OID for the composite DSA / KEM
    pub fn get_oid(&self) -> &str {
        &self.oid
    }

    /// Get the public key for the traditional DSA / KEM
    ///
    /// # Returns
    ///
    /// The public key for the traditional DSA / KEM
    pub fn get_trad_pk(&self) -> Vec<u8> {
        self.trad_pk.clone()
    }

    /// Get the public key for the post-quantum DSA / KEM
    ///
    /// # Returns
    ///
    /// The public key for the post-quantum DSA / KEM
    pub fn get_pq_pk(&self) -> Vec<u8> {
        self.pq_pk.clone()
    }

    /// Create a new composite public key from a DER-encoded public key
    ///
    /// # Arguments
    ///
    /// * `der` - The DER-encoded public key
    ///
    /// # Returns
    ///
    /// A new composite public key
    pub fn from_der(oid: &str, der: &[u8]) -> Result<Self> {
        // Parse as compressed public key
        let comp_pk = CompositeSigKemPublicKey::from_der(der)
            .map_err(|_| QubitCryptError::InvalidPublicKey)?;

        let pq_pk = if let Some(pq_pk) = comp_pk.pq_pk.as_bytes() {
            pq_pk
        } else {
            return Err(QubitCryptError::InvalidPublicKey);
        };

        let trad_pk = if let Some(trad_pk) = comp_pk.trad_pk.as_bytes() {
            trad_pk
        } else {
            return Err(QubitCryptError::InvalidPublicKey);
        };

        Ok(CompositePublicKey::new(oid, pq_pk, trad_pk))
    }

    /// Encode the composite public key as a DER-encoded public key
    ///
    /// # Returns
    ///
    /// The DER-encoded public key
    pub fn to_der(&self) -> Result<Vec<u8>> {
        let comp_sig_pk = CompositeSigKemPublicKey {
            pq_pk: BitString::new(0, self.pq_pk.as_slice())
                .map_err(|_| QubitCryptError::InvalidPublicKey)?,
            trad_pk: BitString::new(0, self.trad_pk.as_slice())
                .map_err(|_| QubitCryptError::InvalidPublicKey)?,
        };

        let comp_sig_pk = comp_sig_pk
            .to_der()
            .map_err(|_| QubitCryptError::InvalidPublicKey)?;

        Ok(comp_sig_pk.as_slice().to_vec())
    }
}
