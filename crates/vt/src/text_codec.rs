//! Converts session text; transport and terminal protocol messages stay bytes.
use encoding_rs::{CoderResult, Decoder, Encoding};
use nocterm_session::{Charset, TextEncodingError};

/// Incrementally decodes a session charset to UTF-8 and encodes admitted text.
///
/// Transport control messages remain bytes and bypass this codec.
pub struct TextCodec {
    charset: Charset,
    decoder: Decoder,
}

impl TextCodec {
    /// Starts a fresh decoder without consuming byte-order markers specially.
    pub fn new(charset: Charset) -> Self {
        Self {
            charset,
            decoder: encoding(charset).new_decoder_without_bom_handling(),
        }
    }
    /// Decodes a fragment, replacing invalid bytes; `last` finalizes the stream.
    pub fn decode(&mut self, mut bytes: &[u8], last: bool) -> Vec<u8> {
        let mut text = String::with_capacity(bytes.len().saturating_mul(3) + 16);
        loop {
            let (result, read, _) = self.decoder.decode_to_string(bytes, &mut text, last);
            bytes = &bytes[read..];
            match result {
                CoderResult::InputEmpty => break,
                CoderResult::OutputFull => text.reserve(bytes.len().saturating_mul(3) + 16),
            }
        }
        text.into_bytes()
    }
    /// Encodes text or refuses characters unavailable in the configured charset.
    pub fn encode(&self, text: &str) -> Result<Vec<u8>, TextEncodingError> {
        let (bytes, _, unmappable) = encoding(self.charset).encode(text);
        if unmappable {
            return Err(TextEncodingError::new(self.charset));
        }
        Ok(bytes.into_owned())
    }
}

fn encoding(charset: Charset) -> &'static Encoding {
    match charset {
        Charset::Utf8 => encoding_rs::UTF_8,
        Charset::Windows1251 => encoding_rs::WINDOWS_1251,
        Charset::Koi8R => encoding_rs::KOI8_R,
        Charset::Windows1252 => encoding_rs::WINDOWS_1252,
        Charset::Gbk => encoding_rs::GBK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrepresentable_text_retains_configured_charset() {
        let codec = TextCodec::new(Charset::Windows1252);
        let error = codec.encode("中文").unwrap_err();
        assert_eq!(error.charset(), Charset::Windows1252);
        assert_eq!(
            error.to_string(),
            "Some characters cannot be sent in Windows-1252. Change the session charset or edit the text."
        );
        assert_eq!(codec.encode("plain text").unwrap(), b"plain text");
    }

    #[test]
    fn incomplete_utf8_is_buffered_and_replaced_only_when_finalized() {
        let mut codec = TextCodec::new(Charset::Utf8);
        assert!(codec.decode(&[0xe2, 0x82], false).is_empty());
        assert_eq!(codec.decode(&[0xac], false), "€".as_bytes());
        assert!(codec.decode(&[0xe2], false).is_empty());
        assert_eq!(codec.decode(&[], true), "�".as_bytes());
    }

    #[test]
    fn invalid_bytes_and_protocol_controls_remain_stable_across_fragments() {
        let input = b"\x1b]133;A\x07a\xff\xe2\x82";
        let expected = "\x1b]133;A\x07a��";
        for split in 0..=input.len() {
            let mut codec = TextCodec::new(Charset::Utf8);
            let mut output = codec.decode(&input[..split], false);
            output.extend(codec.decode(&input[split..], false));
            output.extend(codec.decode(&[], true));
            assert_eq!(output, expected.as_bytes());
        }
    }

    #[test]
    fn supported_charsets_round_trip_representable_text() {
        for (charset, text) in [
            (Charset::Utf8, "Привет 中文"),
            (Charset::Windows1251, "Привет"),
            (Charset::Koi8R, "Привет"),
            (Charset::Windows1252, "café €"),
            (Charset::Gbk, "中文"),
        ] {
            let mut codec = TextCodec::new(charset);
            let bytes = codec.encode(text).unwrap();
            assert_eq!(codec.decode(&bytes, true), text.as_bytes());
        }
    }

    #[test]
    fn fragmented_multibyte_and_control_sequences_remain_stable() {
        let input = "\x1b[31m中文\x1b[0m\r\n";
        let (bytes, _, errors) = encoding_rs::GBK.encode(input);
        assert!(!errors);
        for split in 0..=bytes.len() {
            let mut codec = TextCodec::new(Charset::Gbk);
            let mut output = codec.decode(&bytes[..split], false);
            output.extend(codec.decode(&bytes[split..], false));
            output.extend(codec.decode(&[], true));
            assert_eq!(output, input.as_bytes());
        }
    }
}
