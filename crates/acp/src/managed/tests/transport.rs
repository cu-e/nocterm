//! Actual original source ownership reaches raw native IO before decoding.
use super::fixture;
use crate::managed::transport::{Reader, Writer};
use agent_client_protocol::managed::Source;
use futures::{executor::block_on, io::Cursor};

#[test]
fn managed_lines_preserve_original_crlf_eof_utf8_and_line_limit() {
    block_on(async {
        let fixture = fixture();
        let mut reader = Reader::new(
            Cursor::new(b"a\r\nb\n\n".as_slice()),
            fixture.budget.clone(),
        );
        assert_eq!(reader.line().await.unwrap().unwrap().text(), "a");
        assert_eq!(reader.line().await.unwrap().unwrap().text(), "b");
        assert_eq!(reader.line().await.unwrap().unwrap().text(), "");
        assert!(reader.line().await.unwrap().is_none());
        for raw in [b"partial".as_slice(), &[0xff, b'\n']] {
            let mut reader = Reader::new(Cursor::new(raw), fixture.budget.clone());
            assert!(reader.line().await.is_err());
        }
        let mut raw = "x".repeat(crate::lines::ACP_LINE_LIMIT);
        raw.push('\n');
        let mut original = futures::io::BufReader::new(Cursor::new(raw.as_bytes()));
        assert!(
            crate::lines::read_line(&mut original, crate::lines::ACP_LINE_LIMIT)
                .await
                .is_ok()
        );
        let mut reader = Reader::new(Cursor::new(raw.as_bytes()), fixture.budget.clone());
        assert_eq!(
            reader.line().await.unwrap().unwrap().text().len(),
            crate::lines::ACP_LINE_LIMIT
        );
        raw.insert(0, 'x');
        let mut original = futures::io::BufReader::new(Cursor::new(raw.as_bytes()));
        assert!(
            crate::lines::read_line(&mut original, crate::lines::ACP_LINE_LIMIT)
                .await
                .is_err()
        );
        let mut reader = Reader::new(Cursor::new(raw.as_bytes()), fixture.budget.clone());
        assert!(reader.line().await.is_err());
    });
}
#[test]
fn managed_output_rejects_foreign_sources_and_preserves_exact_newline_bytes() {
    block_on(async {
        let fixture = fixture();
        let foreign = super::fixture();
        let source = Source::copy(r#"{"literal":"é"}"#, fixture.budget.clone()).unwrap();
        let foreign_source = Source::copy("{}", foreign.budget.clone()).unwrap();
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = Writer::new(&mut output, fixture.budget.clone());
            assert!(writer.line(&foreign_source).await.is_err());
            writer.line(&source).await.unwrap();
            let invalid = Source::copy("a\nb", fixture.budget.clone()).unwrap();
            assert!(writer.line(&invalid).await.is_err());
        }
        assert_eq!(output.into_inner(), "{\"literal\":\"é\"}\n".as_bytes());
    });
}

#[test]
fn cancelling_a_blocked_line_future_preserves_original_already_consumed_bytes() {
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };
    struct Split(u8);
    impl futures::io::AsyncRead for Split {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            output: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            let bytes: &[u8] = match self.0 {
                0 => b"part",
                1 => {
                    self.0 = 2;
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                2 => b"ial\r\nnext\n",
                _ => b"",
            };
            self.0 += 1;
            output[..bytes.len()].copy_from_slice(bytes);
            Poll::Ready(Ok(bytes.len()))
        }
    }
    let fixture = fixture();
    let mut reader = Reader::new(Split(0), fixture.budget.clone());
    let waker = futures::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    {
        let future = reader.line();
        futures::pin_mut!(future);
        assert!(future.as_mut().poll(&mut cx).is_pending());
    }
    assert_eq!(block_on(reader.line()).unwrap().unwrap().text(), "partial");
    assert_eq!(block_on(reader.line()).unwrap().unwrap().text(), "next");
    assert!(block_on(reader.line()).unwrap().is_none());
}

#[test]
fn cancelled_partial_write_resumes_without_repeating_bytes_or_newline() {
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll},
    };
    struct Partial {
        bytes: Vec<u8>,
        block: bool,
    }
    impl futures::io::AsyncWrite for Partial {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if self.block {
                self.block = false;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            let count = bytes.len().min(3);
            self.bytes.extend_from_slice(&bytes[..count]);
            self.block = true;
            Poll::Ready(Ok(count))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
    let fixture = fixture();
    let source = Source::copy("abcdefg", fixture.budget.clone()).unwrap();
    let mut output = Partial {
        bytes: Vec::new(),
        block: false,
    };
    {
        let mut writer = Writer::new(&mut output, fixture.budget.clone());
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        for _ in 0..3 {
            let future = writer.line(&source);
            futures::pin_mut!(future);
            assert!(future.as_mut().poll(&mut cx).is_pending());
        }
        block_on(writer.line(&source)).unwrap();
    }
    assert_eq!(output.bytes, b"abcdefg\n");
}
