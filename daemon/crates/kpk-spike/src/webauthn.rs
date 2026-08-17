//! CTAP2 credential creation and assertions.
//!
//! Only the parts a browser actually exercises: `authenticatorMakeCredential` and
//! `authenticatorGetAssertion`, with `none` attestation. Attestation formats that carry a
//! certificate would tie this to a vendor identity we deliberately do not claim.

use anyhow::{Context as _, Result, bail};
use ciborium::value::Value;
use sha2::{Digest as _, Sha256};

/// Our own AAGUID — never a certified vendor's.
pub const AAGUID: [u8; 16] = [
    0x6f, 0x1d, 0x2b, 0x1a, 0x9c, 0x4e, 0x4f, 0x57, 0xa1, 0xd3, 0x7b, 0x8e, 0x2c, 0x05, 0xf9, 0xa4,
];

pub const FLAG_USER_PRESENT: u8 = 0x01;
pub const FLAG_USER_VERIFIED: u8 = 0x04;
pub const FLAG_ATTESTED: u8 = 0x40;

pub struct MakeCredential {
    pub rp_id: String,
    pub user_name: String,
}

pub struct GetAssertion {
    pub client_data_hash: Vec<u8>,
    pub rp_id: String,
    /// Credential ids the site will accept, in its order of preference.
    pub allowed: Vec<Vec<u8>>,
}

/// CTAP2 numbers its request parameters rather than naming them; these are the indices from
/// the spec, not arbitrary constants.
pub fn parse_make_credential(payload: &[u8]) -> Result<MakeCredential> {
    let map = top_level_map(payload)?;
    // clientDataHash (parameter 1) is deliberately unused: `none` attestation signs nothing,
    // so there is no attestation statement for it to be bound into.
    let rp = entry(&map, 2).context("makeCredential has no rp")?;
    let rp_id = text_field(rp, "id").context("rp has no id")?;

    let user_name = entry(&map, 3)
        .and_then(|user| text_field(user, "name"))
        .unwrap_or_else(|| "user".to_owned());

    Ok(MakeCredential { rp_id, user_name })
}

pub fn parse_get_assertion(payload: &[u8]) -> Result<GetAssertion> {
    let map = top_level_map(payload)?;
    let rp_id = match entry(&map, 1).context("getAssertion has no rpId")? {
        Value::Text(text) => text.clone(),
        _ => bail!("rpId is not a string"),
    };
    let client_data_hash = bytes(entry(&map, 2).context("getAssertion has no clientDataHash")?)?;

    let allowed = match entry(&map, 3) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| field(item, "id"))
            .filter_map(|value| bytes(value).ok())
            .collect(),
        _ => Vec::new(),
    };

    Ok(GetAssertion {
        client_data_hash,
        rp_id,
        allowed,
    })
}

/// `rpIdHash ‖ flags ‖ signCount`, optionally followed by attested credential data.
pub fn authenticator_data(
    rp_id: &str,
    flags: u8,
    sign_count: u32,
    attested: Option<&[u8]>,
) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&Sha256::digest(rp_id.as_bytes()));
    data.push(flags);
    data.extend_from_slice(&sign_count.to_be_bytes());
    if let Some(extra) = attested {
        data.extend_from_slice(extra);
    }
    data
}

/// # Errors
/// Fails if the credential id is longer than a `u16` can describe.
pub fn attested_credential_data(credential_id: &[u8], cose_key: &[u8]) -> Result<Vec<u8>> {
    let length = u16::try_from(credential_id.len()).context("credential id is too long")?;
    let mut data = Vec::new();
    data.extend_from_slice(&AAGUID);
    data.extend_from_slice(&length.to_be_bytes());
    data.extend_from_slice(credential_id);
    data.extend_from_slice(cose_key);
    Ok(data)
}

/// `COSE_Key` for an ES256 public key: kty EC2, alg ES256, curve P-256, affine x and y.
///
/// # Errors
/// Fails if the coordinates are not 32 bytes or the map cannot be encoded.
pub fn cose_key(x: &[u8], y: &[u8]) -> Result<Vec<u8>> {
    if x.len() != 32 || y.len() != 32 {
        bail!("P-256 coordinates must be 32 bytes each");
    }
    let map = Value::Map(vec![
        (Value::Integer(1.into()), Value::Integer(2.into())),
        (Value::Integer(3.into()), Value::Integer((-7).into())),
        (Value::Integer((-1).into()), Value::Integer(1.into())),
        (Value::Integer((-2).into()), Value::Bytes(x.to_vec())),
        (Value::Integer((-3).into()), Value::Bytes(y.to_vec())),
    ]);
    encode(&map)
}

/// # Errors
/// Fails if the response cannot be encoded.
pub fn make_credential_response(auth_data: &[u8]) -> Result<Vec<u8>> {
    let map = Value::Map(vec![
        (Value::Integer(1.into()), Value::Text("none".to_owned())),
        (Value::Integer(2.into()), Value::Bytes(auth_data.to_vec())),
        (Value::Integer(3.into()), Value::Map(Vec::new())),
    ]);
    encode(&map)
}

/// # Errors
/// Fails if the response cannot be encoded.
pub fn get_assertion_response(
    credential_id: &[u8],
    auth_data: &[u8],
    signature: &[u8],
) -> Result<Vec<u8>> {
    let descriptor = Value::Map(vec![
        (
            Value::Text("type".to_owned()),
            Value::Text("public-key".to_owned()),
        ),
        (
            Value::Text("id".to_owned()),
            Value::Bytes(credential_id.to_vec()),
        ),
    ]);
    let map = Value::Map(vec![
        (Value::Integer(1.into()), descriptor),
        (Value::Integer(2.into()), Value::Bytes(auth_data.to_vec())),
        (Value::Integer(3.into()), Value::Bytes(signature.to_vec())),
    ]);
    encode(&map)
}

fn encode(value: &Value) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    ciborium::ser::into_writer(value, &mut buffer).context("encoding a CTAP2 response")?;
    Ok(buffer)
}

fn top_level_map(payload: &[u8]) -> Result<Vec<(Value, Value)>> {
    let value: Value =
        ciborium::de::from_reader(payload).context("request body is not valid CBOR")?;
    match value {
        Value::Map(entries) => Ok(entries),
        _ => bail!("request body is not a CBOR map"),
    }
}

fn entry(map: &[(Value, Value)], key: i128) -> Option<&Value> {
    map.iter()
        .find(|(candidate, _)| match candidate {
            Value::Integer(number) => i128::from(*number) == key,
            _ => false,
        })
        .map(|(_, value)| value)
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Map(entries) => entries
            .iter()
            .find(|(key, _)| matches!(key, Value::Text(text) if text == name))
            .map(|(_, found)| found),
        _ => None,
    }
}

fn text_field(value: &Value, name: &str) -> Option<String> {
    match field(value, name) {
        Some(Value::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

fn bytes(value: &Value) -> Result<Vec<u8>> {
    match value {
        Value::Bytes(raw) => Ok(raw.clone()),
        _ => bail!("expected a CBOR byte string"),
    }
}
