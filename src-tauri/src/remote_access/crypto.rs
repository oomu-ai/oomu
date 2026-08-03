use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand_core::{OsRng, RngCore};

pub fn random_hex(bytes: usize) -> String {
    let mut value = vec![0u8; bytes];
    OsRng.fill_bytes(&mut value);
    hex::encode(value)
}

pub fn uuid_id(prefix: &str) -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!("{prefix}_{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",bytes[0],bytes[1],bytes[2],bytes[3],bytes[4],bytes[5],bytes[6],bytes[7],bytes[8],bytes[9],bytes[10],bytes[11],bytes[12],bytes[13],bytes[14],bytes[15])
}

pub fn verify(public_key: &str, message: &str, signature: &str) -> Result<(), String> {
    let key: [u8; 32] = hex::decode(public_key)
        .map_err(|_| "Remote device key is invalid.".to_string())?
        .try_into()
        .map_err(|_| "Remote device key is invalid.".to_string())?;
    let key =
        VerifyingKey::from_bytes(&key).map_err(|_| "Remote device key is invalid.".to_string())?;
    let signature = Signature::from_slice(
        &hex::decode(signature).map_err(|_| "Remote command signature is invalid.".to_string())?,
    )
    .map_err(|_| "Remote command signature is invalid.".to_string())?;
    key.verify(message.as_bytes(), &signature)
        .map_err(|_| "Remote command signature did not match this device.".to_string())
}
