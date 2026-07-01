use std::io::{Read, Result as IoResult, Seek, SeekFrom};
use symphonia::core::io::MediaSource;
use tokio::sync::broadcast::{self, error::RecvError, Receiver, Sender};

pub struct PcmBridge {
    sender: Sender<Vec<u8>>,
}

impl PcmBridge {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(1024);
        Self { sender }
    }

    pub fn sender(&self) -> Sender<Vec<u8>> {
        self.sender.clone()
    }

    pub fn source(&self) -> PcmBridgeSource {
        PcmBridgeSource {
            receiver: self.sender.subscribe(),
            pending: Vec::new(),
            offset: 0,
            finished: false,
        }
    }
}

pub struct PcmBridgeSource {
    receiver: Receiver<Vec<u8>>,
    pending: Vec<u8>,
    offset: usize,
    finished: bool,
}

impl Read for PcmBridgeSource {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if self.finished {
            return Ok(0);
        }

        while self.offset >= self.pending.len() {
            match self.receiver.blocking_recv() {
                Ok(chunk) => {
                    if chunk.is_empty() {
                        continue;
                    }

                    self.pending = chunk;
                    self.offset = 0;
                    break;
                }
                Err(RecvError::Lagged(_)) => {
                    continue;
                }
                Err(RecvError::Closed) => {
                    self.finished = true;
                    return Ok(0);
                }
            }
        }

        let remaining = self.pending.len() - self.offset;
        let to_copy = remaining.min(buf.len());
        buf[..to_copy].copy_from_slice(&self.pending[self.offset..self.offset + to_copy]);
        self.offset += to_copy;

        if self.offset >= self.pending.len() {
            self.pending.clear();
            self.offset = 0;
        }

        Ok(to_copy)
    }
}

impl Seek for PcmBridgeSource {
    fn seek(&mut self, _pos: SeekFrom) -> IoResult<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "live PCM bridge is not seekable",
        ))
    }
}

impl MediaSource for PcmBridgeSource {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}
