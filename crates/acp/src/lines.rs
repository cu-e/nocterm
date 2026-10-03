use std::io;

use futures::{
    AsyncBufReadExt, AsyncWriteExt, Sink, Stream,
    io::{AsyncRead, AsyncWrite, BufReader},
};

pub(crate) const ACP_LINE_LIMIT: usize = 16 * 1024 * 1024;

/// Reads without letting a malicious peer allocate an unbounded line.
pub(crate) async fn read_line<R: futures::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Protocol line missing newline",
                ))
            };
        }
        let end = available.iter().position(|&b| b == b'\n');
        let count = end.map_or(available.len(), |end| end + 1);
        if bytes.len().saturating_add(count) > limit + 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Protocol line exceeds its limit",
            ));
        }
        bytes.extend_from_slice(&available[..count]);
        reader.consume_unpin(count);
        if end.is_some() {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            return String::from_utf8(bytes).map(Some).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "Protocol line is not UTF-8")
            });
        }
    }
}

pub(crate) fn incoming<R: AsyncRead + Unpin + Send>(
    reader: R,
) -> impl Stream<Item = io::Result<String>> + Send {
    futures::stream::unfold(Some(BufReader::new(reader)), async |reader| {
        let mut reader = reader?;
        match read_line(&mut reader, ACP_LINE_LIMIT).await {
            Ok(Some(line)) => Some((Ok(line), Some(reader))),
            Ok(None) => None,
            Err(error) => Some((Err(error), None)),
        }
    })
}

pub(crate) fn outgoing<W: AsyncWrite + Unpin + Send>(
    writer: W,
) -> impl Sink<String, Error = io::Error> + Send {
    futures::sink::unfold(writer, async |mut writer, line: String| {
        if line.len() > ACP_LINE_LIMIT || line.contains('\n') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid outgoing protocol line",
            ));
        }
        writer.write_all(line.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok(writer)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_lines_and_rejects_partial_or_invalid_utf8() {
        futures::executor::block_on(async {
            for bytes in [b"12345\n".as_slice(), b"abc".as_slice(), &[255, 10]] {
                assert!(
                    read_line(&mut futures::io::Cursor::new(bytes), 4)
                        .await
                        .is_err()
                );
            }
            let mut reader = futures::io::Cursor::new(b"abcd\nnext\n");
            assert_eq!(read_line(&mut reader, 4).await.unwrap().unwrap(), "abcd");
            assert_eq!(read_line(&mut reader, 4).await.unwrap().unwrap(), "next");
            assert!(read_line(&mut reader, 4).await.unwrap().is_none());
        });
    }
}
