//! Generates a Lenovo `sn.img` unlock token from a serial number.
//!
//! This is a faithful port of the ZUI/ZUX file layouts used by NekoYuzu's
//! "Lenovo BootLoader Unlock Tool" (github.com/MlgmXyysd/lenovoubl), whose
//! output is what Lenovo bootloaders accept on the `unlock` partition. A file
//! is a fixed header, a fixed body, the serial number and a fixed signature,
//! laid out at fixed offsets.
//!
//! Two layouts exist: `ZUI` for a serial number of at most 16 characters and
//! `ZUX` for a 64-character `Bootloader_SN`. [`generate`] picks the layout
//! from the input length (the same distinction the tool's UI makes).

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

/// One file layout: where each region ends and the three fixed blobs.
struct Layout {
    /// `[serial_max, header_end, content_end, serial_end]`.
    offsets: [usize; 4],
    headers: &'static str,
    content: &'static str,
    signature: &'static str,
}

/// ZUI layout, for a serial number of up to 16 characters.
const ZUI: Layout = Layout {
    offsets: [16, 576, 2048, 4096],
    headers: "QU5EUk9JRCEKAAAAAIAAEAAAAAAAAAARAAAAAAAA8BAAAQAQAAgAAA==",
    content: "6r3GZB6a41IwAADzLHLILPET1TE=",
    signature: "MIIFBgIBATCCA+EwggLJoAMCAQICCQCiAISzMuYMgDANBgkqhkiG9w0BAQsFADCBhjELMAkGA1UEBhMCQ04xCzAJBgNVBAgMAkJKMRUwEwYDVQQHDAxCZWlqaW5nIFZpZXcxDzANBgNVBAoMBkxFTk9WTzEPMA0GA1UECwwGTW9iaWxlMQ8wDQYDVQQDDAZMRU5PVk8xIDAeBgkqhkiG9w0BCQEWEWxlbm92b0BsZW5vdm8uY29tMB4XDTE4MDEwNTA3MjEzM1oXDTQ1MDUyMzA3MjEzM1owgYYxCzAJBgNVBAYTAkNOMQswCQYDVQQIDAJCSjEVMBMGA1UEBwwMQmVpamluZyBWaWV3MQ8wDQYDVQQKDAZMRU5PVk8xDzANBgNVBAsMBk1vYmlsZTEPMA0GA1UEAwwGTEVOT1ZPMSAwHgYJKoZIhvcNAQkBFhFsZW5vdm9AbGVub3ZvLmNvbTCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAOFGyTPBefCQCBb5hrU3r8O9XofI6XoP7Rj5rJeD9eVikRjShvSAVIaKZWsxZXEfRgNndL70pM9L24qdpbWvOKfN4lUSbkpHdOw7ldaRjznB5I32zgRnzMrgKsM6MR9ePC+tx8lrC5BbJnMtr2ocuMY+xO5A2QlhUg2gujhksdFvX8mUf0yUS2SScEfO32tN8YU27oJUi9RQ6/eVCVMJ3HvEo9T/CV51Nqzdo7Ehd9bWndbE6A+TSicN4/QxNi7DjwC2qjPgp6m4+peNTqxwBMqkb8Q5ll2TgeglrmYA4M+kRlWsPhchR/BFTnUEOf/naM+xs90TzjN/RGs92YvuutkCAwEAAaNQME4wHQYDVR0OBBYEFHjfVJF3GxRNUk/7NLyabPMhwVrfMB8GA1UdIwQYMBaAFHjfVJF3GxRNUk/7NLyabPMhwVrfMAwGA1UdEwQFMAMBAf8wDQYJKoZIhvcNAQELBQADggEBANpz4bY3CQ3w9tIFl+/1VenrgLVD++sRAQxz2wIXRGAjmmOQzvTgJYXHNwywHHwC/QOqXp08ZFbrAhi6dbL0xyIVkvAeE/+Ji78IZrupfYccw6xNuUI1flJVfxDWDofp0YluQ45rme6VZRlRYxvqEvozAvNapfiLleFMEW68orWDxW3ahAK6aop9Y5DarmeZeNHBvN1l2kecdfGjrZIrcA7dFYFS4pfKUjRWp7QIdju55C0fFIhg2gBZU6cWUSjy12pHyor6E6QcrH8W49fBLI/IgvUGqvNk77xqrStw9n2jbyKBLu3Q7l7aOwlxTTrCfZfYioToRiAfry+w7X1fXIUwCwYJKoZIhvcNAQELMAsTBS9ib290AgIQAASCAQCB1hNRsL2Ocko2KTvJd0/IhdPXTqYf1mErWUwIcl2uvkKxi42xcxUUgH/67yIw77l4hmrxAFqdQD9sKtC7VmY8yvl2l0rU4hSwa0rbYT/ij39Ww+cANh19Qohpx16QQbc2OsvAoz+/IBL/ftqt+9C3wZJHM5tr/BN5AMS1ebPf5Xtg8RgAcMU1iBVR5psGWsaaBKDmkGHqtiRBTGeWqJzSp6NV85CB9aH/EJR9CPzVMOApCHc3Ek3Gj+n+X+j64bxSphdob1R0Rnn2q2Dud1A6JlG5t6tX0iP5exHdJMLfLC8mDkKfCkVkZjeVFnX/75i3LdkR0dLuHAHYXzDD+8Uo",
};

/// ZUX layout, for a 64-character `Bootloader_SN`.
const ZUX: Layout = Layout {
    offsets: [64, 20, 36, 100],
    headers: "MWEyYmxlbm92bzNjNGQ1ZQ==",
    content: "AQAAAAABAAAgAAAAAQAAAA==",
    signature: "Sc07NLYUG9uwSNYyg/pRiVerEg9mxUKPT2DYN/8z07IlhimZE1lfzCLdG65CMNaTQVQ2up4vboBsQ/vncieUYHnu4lVZJLBpz3dtFYqUFbptJ7s5fp0XgR1m0600pcr6iMux9t9hsek1B9ksz+j2usGOELNQDoxDC1uEe2I9QqRGgrSpoVqJE+sHI06EJrN8vOrjDR7ZmqCMsZ5NbB+1Gyq7+DKsKNHeWqGWoR695eJVieCYG9OYQ2X8ySgXs/VKyAdn5Ox92BEzHRnBXY5DjxJ8vsmBsMh4SsmLsSSPupBq/fbCsEndt9oTnenacEVcjihN6b9TZ+quJW6AMeDsPQ==",
};

/// The tool's `isSerialLegitimate`: a non-empty alphanumeric string.
fn is_legitimate(serial: &str) -> bool {
    !serial.is_empty() && serial.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// Right-pads `content` with `padding` bytes until it is `length` long
/// (the tool's `append`: unchanged when it is already that long).
fn pad_end(mut content: Vec<u8>, padding: u8, length: usize) -> Vec<u8> {
    if content.len() < length {
        content.resize(length, padding);
    }

    content
}

fn decode(source: &str) -> Vec<u8> {
    STANDARD
        .decode(source)
        .expect("the embedded token data is valid base64")
}

/// Assembles one file out of its layout and the (already padded) serial.
fn build(layout: &Layout, serial: &[u8]) -> Vec<u8> {
    let [_, header_end, content_end, serial_end] = layout.offsets;

    let mut file = Vec::new();
    file.extend(pad_end(decode(layout.headers), 0, header_end));
    file.extend(pad_end(
        decode(layout.content),
        0,
        content_end - header_end,
    ));
    file.extend(pad_end(serial.to_vec(), 0, serial_end - content_end));
    file.extend(decode(layout.signature));
    file
}

/// Generates the `sn.img` for `serial`, or `None` when the serial does not fit
/// either layout (not alphanumeric, or a length neither layout accepts).
pub fn generate(serial: &str) -> Option<Vec<u8>> {
    if !is_legitimate(serial) {
        return None;
    }

    if serial.len() == ZUX.offsets[0] {
        return Some(build(&ZUX, serial.as_bytes()));
    }

    if serial.len() <= ZUI.offsets[0] {
        // The ZUI serial is padded with '0' characters up to eight characters,
        // then with NUL bytes up to the width of its field.
        let padded = pad_end(serial.as_bytes().to_vec(), b'0', 8);
        let serial = pad_end(padded, 0, ZUI.offsets[0]);

        return Some(build(&ZUI, &serial));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// The ported layouts must reproduce the reference tool byte for byte;
    /// the hashes below were computed with an independent implementation of
    /// `js/main.js` from the upstream repository.
    #[test]
    fn the_layouts_match_the_reference_tool() {
        let zui = generate("1234567890ABCDEF").expect("a 16-character serial is valid");
        assert_eq!(zui.len(), 5386);
        assert_eq!(
            hex(&Sha256::digest(&zui)),
            "91e860b240a3fa5796d40279915549ecbb4efc42d3bc538fedcdc4f8c4d69d49"
        );
        assert!(zui.starts_with(b"ANDROID!\n"));

        let zux = generate(&"A".repeat(64)).expect("a 64-character serial is valid");
        assert_eq!(zux.len(), 356);
        assert_eq!(
            hex(&Sha256::digest(&zux)),
            "c6919b194dfdedc7ed068c55c7e8b71fa5b3463bc6abb14a272c12c515c51085"
        );
        assert!(zux.starts_with(b"1a2blenovo3c4d5e"));
    }

    /// A short ZUI serial is padded with `0` characters, then NUL bytes.
    #[test]
    fn a_short_serial_is_padded_with_zeros() {
        let short = generate("1234").expect("a short serial is valid");
        assert_eq!(
            hex(&Sha256::digest(&short)),
            "a9afaa15f04e9b816a36b2b5c4e75d44c56803ae999e9e87dc4f4547cb2d1e3f"
        );
        assert_eq!(&short[2048..2064], b"12340000\0\0\0\0\0\0\0\0");
    }

    #[test]
    fn only_alphanumeric_serial_lengths_the_layouts_accept_are_generated() {
        assert!(generate("has-dash").is_none());
        assert!(generate("").is_none());
        assert!(generate(&"A".repeat(17)).is_none());
        assert!(generate(&"A".repeat(63)).is_none());
        assert!(generate(&"A".repeat(65)).is_none());
        assert!(generate(&"A".repeat(64)).is_some());
    }
}
