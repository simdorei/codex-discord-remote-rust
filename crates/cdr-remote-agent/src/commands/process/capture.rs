use tokio::io::{AsyncRead, AsyncReadExt};

pub const TRUNCATION_MARKER: &[u8] = b"\n...[output truncated]...\n";
const READ_BYTES: usize = 64 * 1024;

#[cfg(not(windows))]
pub async fn capture(
    stream: impl AsyncRead + Unpin,
    limit: usize,
) -> Result<BoundedCapture, std::io::Error> {
    capture_identified(stream, limit, "portable".into()).await
}

pub async fn capture_identified(
    mut stream: impl AsyncRead + Unpin,
    limit: usize,
    label: String,
) -> Result<BoundedCapture, std::io::Error> {
    let mut capture = BoundedCapture::new(limit);
    let mut buffer = vec![0; READ_BYTES];
    let mut samples = 0;
    eprintln!("[process-diag:{label}] reader-start");
    loop {
        let count = stream.read(&mut buffer).await.inspect_err(|error| {
            eprintln!("[process-diag:{label}] reader-error={error}");
        })?;
        if count == 0 {
            eprintln!(
                "[process-diag:{label}] reader-eof bytes={}",
                capture.bytes_seen()
            );
            return Ok(capture);
        }
        if samples < 4 {
            eprintln!(
                "[process-diag:{label}] read={count} sample={:?}",
                String::from_utf8_lossy(&buffer[..count.min(256)])
            );
            samples += 1;
        }
        capture.append(&buffer[..count]);
    }
}

pub struct BoundedCapture {
    limit: usize,
    total: u64,
    complete: Option<Vec<u8>>,
    head: Vec<u8>,
    tail: Vec<u8>,
}

impl BoundedCapture {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            total: 0,
            complete: Some(Vec::new()),
            head: Vec::new(),
            tail: Vec::new(),
        }
    }

    fn append(&mut self, data: &[u8]) {
        self.total = self
            .total
            .saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
        if let Some(complete) = &mut self.complete {
            if complete.len() + data.len() <= self.limit {
                complete.extend_from_slice(data);
                return;
            }
            let mut combined = std::mem::take(complete);
            combined.extend_from_slice(data);
            let retained = self.limit - TRUNCATION_MARKER.len();
            let head_limit = retained * 2 / 3;
            let tail_limit = retained - head_limit;
            self.head.extend_from_slice(&combined[..head_limit]);
            self.tail
                .extend_from_slice(&combined[combined.len() - tail_limit..]);
            self.complete = None;
            return;
        }
        let tail_limit = self.limit - TRUNCATION_MARKER.len() - self.head.len();
        self.tail.extend_from_slice(data);
        if self.tail.len() > tail_limit {
            self.tail.drain(..self.tail.len() - tail_limit);
        }
    }

    pub fn value(&self) -> Vec<u8> {
        self.complete.clone().unwrap_or_else(|| {
            [
                self.head.as_slice(),
                TRUNCATION_MARKER,
                self.tail.as_slice(),
            ]
            .concat()
        })
    }

    pub fn truncated(&self) -> bool {
        self.complete.is_none()
    }

    pub fn bytes_seen(&self) -> u64 {
        self.total
    }
}
