//! Message framing of Chrome's `--remote-debugging-pipe`: every JSON message,
//! in either direction, is followed by a NUL byte.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::client::{MessageSink, MessageSource};

const DELIMITER: u8 = 0;
const READ_CHUNK: usize = 64 * 1024;

/// Splits the byte stream from the browser into messages. There is no size
/// limit: a script body arrives as one message, however large the script.
pub(crate) struct MessageReader<R> {
    source: R,
    buffered: Vec<u8>,
    /// Bytes at the front of `buffered` already known to hold no delimiter.
    searched: usize,
}

impl<R: AsyncRead + Unpin> MessageReader<R> {
    pub(crate) fn new(source: R) -> Self {
        Self {
            source,
            buffered: Vec::new(),
            searched: 0,
        }
    }

    /// The next message, or `None` once the browser closed its end. Safe to
    /// drop mid-read: received bytes stay buffered for the next call.
    pub(crate) async fn next(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        loop {
            let unsearched = &self.buffered[self.searched..];
            if let Some(offset) = unsearched.iter().position(|byte| *byte == DELIMITER) {
                let rest = self.buffered.split_off(self.searched + offset + 1);
                let mut message = std::mem::replace(&mut self.buffered, rest);
                message.pop();
                self.searched = 0;
                return Ok(Some(message));
            }
            self.searched = self.buffered.len();
            self.buffered.reserve(READ_CHUNK);
            if self.source.read_buf(&mut self.buffered).await? == 0 {
                return Ok(None);
            }
        }
    }
}

impl<R: AsyncRead + Unpin + Send + 'static> MessageSource for MessageReader<R> {
    async fn next(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        MessageReader::next(self).await
    }
}

/// Writes messages to the browser's end of the pipe. Dropping it closes
/// the stream, which is what makes Chrome exit.
pub(crate) struct MessageWriter<W>(W);

impl<W> MessageWriter<W> {
    pub(crate) fn new(sink: W) -> Self {
        Self(sink)
    }
}

impl<W: AsyncWrite + Unpin + Send + 'static> MessageSink for MessageWriter<W> {
    async fn send(&mut self, message: &str) -> std::io::Result<()> {
        write_message(&mut self.0, message).await
    }
}

pub(crate) async fn write_message<W: AsyncWrite + Unpin>(
    sink: &mut W,
    message: &str,
) -> std::io::Result<()> {
    sink.write_all(message.as_bytes()).await?;
    sink.write_all(&[DELIMITER]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn written_messages_are_read_back_one_by_one() {
        let (mut near, far) = tokio::io::duplex(READ_CHUNK);
        let mut reader = MessageReader::new(far);

        write_message(&mut near, r#"{"id":1}"#).await.unwrap();
        write_message(&mut near, "").await.unwrap();
        write_message(&mut near, r#"{"text":"NUL is escaped: \u0000"}"#)
            .await
            .unwrap();
        drop(near);

        assert_eq!(reader.next().await.unwrap().unwrap(), br#"{"id":1}"#);
        assert_eq!(reader.next().await.unwrap().unwrap(), b"");
        assert_eq!(
            reader.next().await.unwrap().unwrap(),
            br#"{"text":"NUL is escaped: \u0000"}"#
        );
        assert_eq!(reader.next().await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_message_split_across_reads_is_assembled() {
        let source = tokio_test_reader(&[b"{\"a\"", b":", b"1}\0{\"b\":2}", b"\0\0"]);
        let mut reader = MessageReader::new(source);

        assert_eq!(reader.next().await.unwrap().unwrap(), br#"{"a":1}"#);
        assert_eq!(reader.next().await.unwrap().unwrap(), br#"{"b":2}"#);
        assert_eq!(reader.next().await.unwrap().unwrap(), b"");
        assert_eq!(reader.next().await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_message_far_larger_than_one_read_arrives_whole() {
        let body = "x".repeat(5 * 1024 * 1024);
        let (mut near, far) = tokio::io::duplex(4096);
        let mut reader = MessageReader::new(far);
        let sent = body.clone();
        let writer = tokio::spawn(async move {
            write_message(&mut near, &sent).await.unwrap();
            write_message(&mut near, "next").await.unwrap();
        });

        assert_eq!(reader.next().await.unwrap().unwrap(), body.as_bytes());
        assert_eq!(reader.next().await.unwrap().unwrap(), b"next");
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn a_message_cut_off_by_the_end_of_the_stream_is_dropped() {
        let mut reader = MessageReader::new(tokio_test_reader(&[b"{\"a\":1}\0{\"b\""]));

        assert_eq!(reader.next().await.unwrap().unwrap(), br#"{"a":1}"#);
        assert_eq!(reader.next().await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_abandoned_read_loses_nothing() {
        let (mut near, far) = tokio::io::duplex(READ_CHUNK);
        let mut reader = MessageReader::new(far);
        near.write_all(b"{\"half\":").await.unwrap();

        let abandoned =
            tokio::time::timeout(std::time::Duration::from_millis(50), reader.next()).await;
        assert!(abandoned.is_err(), "half a message is not a message");

        near.write_all(b"true}\0").await.unwrap();
        assert_eq!(reader.next().await.unwrap().unwrap(), br#"{"half":true}"#);
    }

    /// A stream that hands out exactly these chunks, one per read.
    fn tokio_test_reader(chunks: &[&[u8]]) -> impl AsyncRead + Unpin {
        let (mut near, far) = tokio::io::duplex(1);
        let chunks: Vec<Vec<u8>> = chunks.iter().map(|chunk| chunk.to_vec()).collect();
        tokio::spawn(async move {
            for chunk in chunks {
                near.write_all(&chunk).await.unwrap();
            }
        });
        far
    }
}
