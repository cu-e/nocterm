//! Converts session text; transport and terminal protocol messages stay bytes.
use encoding_rs::{CoderResult, Decoder, Encoding};
use nocterm_session::Charset;

#[derive(Debug, thiserror::Error)]
#[error("Some characters cannot be sent in {0}. Change the session charset or edit the text.")]
pub(crate) struct EncodeError(&'static str);

pub(crate) struct TextCodec {
    charset: Charset,
    decoder: Decoder,
}

impl TextCodec {
    pub(crate) fn new(charset: Charset) -> Self {
        Self {
            charset,
            decoder: encoding(charset).new_decoder_without_bom_handling(),
        }
    }
    pub(crate) fn decode(&mut self, mut bytes: &[u8], last: bool) -> Vec<u8> {
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
    pub(crate) fn encode(&self, text: &str) -> Result<Vec<u8>, EncodeError> {
        let (bytes, _, unmappable) = encoding(self.charset).encode(text);
        if unmappable {
            return Err(EncodeError(self.charset.label()));
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
