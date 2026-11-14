//! Gzip decoding on a thread of its own, so decompression runs while the parser encodes what came
//! before. The decoder hands over chunks of [`PIECE`] bytes through a channel of [`QUEUE`] slots,
//! so at most [`IN_FLIGHT`] bytes are held between the two.
//!
//! A BGZF file (gzip members of at most 64 KiB, each saying its own size in a `BC` extra field,
//! as `bgzip` writes them) is decoded [`GROUP`] members at a time, the members in parallel. Any
//! other gzip, single or multi-member, is decoded in order on the decoder thread; so is the rest
//! of a file from the first member that doesn't look like BGZF.

use std::cell::Cell;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Cursor, Read};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

use rayon::prelude::*;

use crate::parse::{Cancelled, Progress};

/// Decoded bytes per chunk handed to the parser.
const PIECE: usize = 2 << 20;

/// Chunks waiting in the channel.
const QUEUE: usize = 3;

/// Bytes held between decoder and parser at most: the queued chunks, the one the decoder is
/// filling, the one the parser is reading, and a BGZF group's compressed members (a BGZF member
/// is at most 64 KiB compressed too, so a group is at most one chunk).
pub(crate) const IN_FLIGHT: usize = (QUEUE + 3) * PIECE;

/// BGZF members decoded together; their decoded sizes add up to about [`PIECE`].
const GROUP: usize = PIECE / BGZF_MAX;

/// Largest decoded member BGZF allows.
const BGZF_MAX: usize = 1 << 16;

/// Decoded bytes, and the compressed bytes read from the file since the previous chunk.
struct Chunk {
    data: Vec<u8>,
    read: u64,
}

/// Decompressed content of a gzip file, decoded on another thread. Reading it counts compressed
/// bytes into `progress` and fails with [`Cancelled`] once the load is cancelled; dropping it
/// stops the decoder.
pub(crate) struct Piped<'p> {
    rx: Option<Receiver<io::Result<Chunk>>>,
    decoder: Option<JoinHandle<()>>,
    cur: Vec<u8>,
    pos: usize,
    progress: Option<&'p Progress>,
}

impl<'p> Piped<'p> {
    pub(crate) fn new(file: File, progress: Option<&'p Progress>) -> io::Result<Self> {
        let (tx, rx) = sync_channel(QUEUE);
        let decoder = std::thread::Builder::new()
            .name("gzip".into())
            .spawn(move || decode(file, &tx))?;
        Ok(Piped {
            rx: Some(rx),
            decoder: Some(decoder),
            cur: Vec::new(),
            pos: 0,
            progress,
        })
    }
}

impl Drop for Piped<'_> {
    fn drop(&mut self) {
        // Closing the channel makes the decoder's next send fail, and it returns.
        self.rx = None;
        if let Some(d) = self.decoder.take() {
            d.join().ok();
        }
    }
}

impl Read for Piped<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let have = self.fill_buf()?;
        let n = have.len().min(buf.len());
        buf[..n].copy_from_slice(&have[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for Piped<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if let Some(p) = self.progress
            && p.is_cancelled()
        {
            return Err(io::Error::other(Cancelled));
        }
        while self.pos == self.cur.len() {
            // Free the chunk read before waiting for the next one.
            self.cur = Vec::new();
            self.pos = 0;
            let Some(rx) = &self.rx else {
                break;
            };
            match rx.recv() {
                Ok(Ok(c)) => {
                    if let Some(p) = self.progress {
                        p.add_bytes(c.read);
                    }
                    self.cur = c.data;
                }
                Ok(Err(e)) => {
                    self.rx = None;
                    return Err(e);
                }
                // The decoder finished, or panicked: that must not pass for the end of the file.
                Err(_) => {
                    self.rx = None;
                    if self.decoder.take().is_some_and(|d| d.join().is_err()) {
                        return Err(io::Error::other("gzip decoder failed"));
                    }
                }
            }
        }
        Ok(&self.cur[self.pos..])
    }

    fn consume(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.cur.len());
    }
}

/// A reader that counts the bytes read through it.
struct Tally<'a, R> {
    inner: R,
    n: &'a Cell<u64>,
}

impl<R: Read> Read for Tally<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let k = self.inner.read(buf)?;
        self.n.set(self.n.get() + k as u64);
        Ok(k)
    }
}

/// The decoder thread: everything goes to `tx` until the end of the file, an error, or the reader
/// hanging up.
fn decode(file: File, tx: &SyncSender<io::Result<Chunk>>) {
    let n = Cell::new(0);
    let mut sent = 0;
    let mut send = |r: io::Result<Vec<u8>>| -> bool {
        let r = r.map(|data| {
            let read = n.get() - sent;
            sent = n.get();
            Chunk { data, read }
        });
        let ok = r.is_ok();
        tx.send(r).is_ok() && ok
    };
    let mut r = BufReader::with_capacity(
        1 << 16,
        Tally {
            inner: file,
            n: &n,
        },
    );
    let rest = match bgzf(&mut r, &mut send) {
        Ok(Some(rest)) => rest,
        Ok(None) => return,
        Err(e) => {
            send(Err(e));
            return;
        }
    };
    let mut d = flate2::read::MultiGzDecoder::new(Cursor::new(rest).chain(r));
    loop {
        let mut data = Vec::with_capacity(PIECE);
        match (&mut d).take(PIECE as u64).read_to_end(&mut data) {
            Ok(0) => {
                // The last of the file (a trailer, padding) may have been read for no output.
                send(Ok(data));
                return;
            }
            Ok(_) => {
                if !send(Ok(data)) {
                    return;
                }
            }
            Err(e) => {
                send(Err(e));
                return;
            }
        }
    }
}

/// Decode BGZF members from `r` while they come, [`GROUP`] at a time in parallel, handing each
/// group to `send`. Returns `None` at the end of the file or once `send` refuses, or the bytes
/// read of the first member that isn't BGZF (the rest of the file follows them in `r`).
fn bgzf(
    r: &mut impl BufRead,
    send: &mut impl FnMut(io::Result<Vec<u8>>) -> bool,
) -> io::Result<Option<Vec<u8>>> {
    loop {
        let mut members = Vec::with_capacity(GROUP);
        let mut other = None;
        while members.len() < GROUP {
            match member(r)? {
                Member::Bgzf(m) => members.push(m),
                Member::Other(head) => {
                    other = Some(head);
                    break;
                }
                Member::End => break,
            }
        }
        if !members.is_empty() {
            let size: usize = members.iter().map(|m| isize_of(m)).sum();
            let mut out = vec![0u8; size];
            let mut parts = Vec::with_capacity(members.len());
            let mut rest = &mut out[..];
            for m in &members {
                let (a, b) = rest.split_at_mut(isize_of(m));
                parts.push((m, a));
                rest = b;
            }
            let ok = parts
                .into_par_iter()
                .try_for_each(|(m, out)| inflate(m, out));
            drop(members);
            if !send(ok.map(|()| out)) {
                return Ok(None);
            }
        }
        match other {
            Some(head) => return Ok(Some(head)),
            None if r.fill_buf()?.is_empty() => return Ok(None),
            None => {}
        }
    }
}

enum Member {
    /// A whole BGZF member, header to trailer.
    Bgzf(Vec<u8>),
    /// The bytes read of a member that isn't BGZF, or is malformed.
    Other(Vec<u8>),
    End,
}

/// The member's decoded size, from its trailer.
fn isize_of(m: &[u8]) -> usize {
    u32::from_le_bytes(m[m.len() - 4..].try_into().unwrap()) as usize
}

/// Read one member if it is BGZF: gzip header with only the extra-field flag set, a `BC` subfield
/// giving the member's size, and a decoded size of at most 64 KiB.
fn member(r: &mut impl BufRead) -> io::Result<Member> {
    let mut head = Vec::with_capacity(18);
    if r.fill_buf()?.is_empty() {
        return Ok(Member::End);
    }
    if !take(r, &mut head, 12)? || head[..4] != [0x1f, 0x8b, 8, 4] {
        return Ok(Member::Other(head));
    }
    let xlen = u16::from_le_bytes([head[10], head[11]]) as usize;
    if !take(r, &mut head, xlen)? {
        return Ok(Member::Other(head));
    }
    let mut extra = &head[12..];
    let mut size = None;
    while extra.len() >= 4 {
        let len = u16::from_le_bytes([extra[2], extra[3]]) as usize;
        if extra[..2] == *b"BC" && len == 2 && extra.len() >= 6 {
            size = Some(u16::from_le_bytes([extra[4], extra[5]]) as usize + 1);
        }
        extra = &extra[(4 + len).min(extra.len())..];
    }
    let Some(size) = size.filter(|&s| s >= head.len() + 8) else {
        return Ok(Member::Other(head));
    };
    let need = size - head.len();
    head.reserve_exact(need);
    if !take(r, &mut head, need)? || isize_of(&head) > BGZF_MAX {
        return Ok(Member::Other(head));
    }
    Ok(Member::Bgzf(head))
}

/// Append `n` bytes of `r` to `out`; false if the file ended first (what was read is kept).
fn take(r: &mut impl Read, out: &mut Vec<u8>, n: usize) -> io::Result<bool> {
    Ok(r.take(n as u64).read_to_end(out)? == n)
}

/// Decode one gzip member into `out`, which has the size its trailer gives; the decoder checks
/// the CRC and the size when it reaches the end.
fn inflate(m: &[u8], out: &mut [u8]) -> io::Result<()> {
    let mut d = flate2::bufread::GzDecoder::new(m);
    d.read_exact(out)?;
    if d.read(&mut [0u8])? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "gzip member longer than its trailer says",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// `data` as BGZF: members of `step` input bytes and the empty end-of-file member.
    pub(crate) fn to_bgzf(data: &[u8], step: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut block = |d: &[u8]| {
            let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
            e.write_all(d).unwrap();
            let body = e.finish().unwrap();
            let bsize = 18 + body.len() + 8;
            out.extend_from_slice(&[0x1f, 0x8b, 8, 4, 0, 0, 0, 0, 0, 0xff, 6, 0, b'B', b'C', 2, 0]);
            out.extend_from_slice(&((bsize - 1) as u16).to_le_bytes());
            out.extend_from_slice(&body);
            let mut crc = flate2::Crc::new();
            crc.update(d);
            out.extend_from_slice(&crc.sum().to_le_bytes());
            out.extend_from_slice(&(d.len() as u32).to_le_bytes());
        };
        for c in data.chunks(step) {
            block(c);
        }
        block(b"");
        out
    }

    fn piped(gz: &[u8]) -> io::Result<Vec<u8>> {
        let dir = std::env::temp_dir().join(format!("msascope-gzip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{:x}.gz", flate2::Crc::new().sum() ^ gz.len() as u32));
        std::fs::write(&p, gz).unwrap();
        let mut out = Vec::new();
        let r = Piped::new(File::open(&p).unwrap(), None)?.read_to_end(&mut out);
        std::fs::remove_file(&p).unwrap();
        r.map(|_| out)
    }

    fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| b"ACGT-\n"[(i * 7 + i / 13) % 6]).collect()
    }

    #[test]
    fn bgzf_groups_and_fallback() {
        let d = data(3 * PIECE + 12345);
        let b = to_bgzf(&d, 65280);
        assert_eq!(piped(&b).unwrap(), d);
        // a plain gzip member after BGZF members is decoded in order
        let mut mixed = to_bgzf(&d[..70000], 30000);
        mixed.truncate(mixed.len() - 28); // no end-of-file member
        let mut e = flate2::write::GzEncoder::new(mixed, flate2::Compression::fast());
        e.write_all(&d[70000..]).unwrap();
        assert_eq!(piped(&e.finish().unwrap()).unwrap(), d);
    }

    #[test]
    fn bgzf_corrupt() {
        let d = data(200_000);
        let mut b = to_bgzf(&d, 65280);
        // a wrong CRC in the second member
        let second = u16::from_le_bytes([b[16], b[17]]) as usize + 1;
        let crc = second + u16::from_le_bytes([b[second + 16], b[second + 17]]) as usize + 1 - 8;
        b[crc] ^= 1;
        assert!(piped(&b).is_err());
        // cut short
        let b = to_bgzf(&d, 65280);
        assert!(piped(&b[..b.len() - 100]).is_err());
    }
}
