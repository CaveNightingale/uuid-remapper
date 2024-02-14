use flate2::{
    read::{GzDecoder, ZlibDecoder},
    write::ZlibEncoder,
};
use std::path::Path;
use std::{
    fmt::Display,
    io::{Read, Write},
};

const SECTOR_SIZE: usize = 4096;
const MAX_CHUNK_NUM: usize = 1024;

pub struct Anvil {
    inner: Vec<u8>,
}

#[derive(Debug)]
pub struct Chunk {
    pub location: (i32, i32),
    pub timestamp: i32,
    pub uncompressed: Option<Vec<u8>>,
}

impl Display for Chunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Chunk({}, {})", self.location.0, self.location.1)
    }
}

pub struct AnvilIter<'a> {
    index: usize,
    anvil: &'a Anvil,
}

impl<'a> Iterator for AnvilIter<'a> {
    type Item = anyhow::Result<Chunk>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < MAX_CHUNK_NUM
            && self.anvil.inner[self.index * 4..self.index * 4 + 4] == [0; 4]
        {
            self.index += 1;
        }
        if self.index == MAX_CHUNK_NUM {
            return None;
        }
        let location = (
            (self.index & 0x1F) as i32,
            ((self.index >> 5) & 0x1F) as i32,
        );
        let offset = u32::from_be_bytes(
            self.anvil.inner[self.index * 4..self.index * 4 + 4]
                .try_into()
                .unwrap(),
        );
        let (offset, sector_count) = (offset >> 8, offset & 0xFF);
        let timestamp = i32::from_be_bytes(
            self.anvil.inner[self.index * 4 + SECTOR_SIZE..self.index * 4 + SECTOR_SIZE + 4]
                .try_into()
                .unwrap(),
        );
        let mut uncompressed = Vec::new();
        let start_index = offset as usize * SECTOR_SIZE;
        if start_index + SECTOR_SIZE * sector_count as usize > self.anvil.inner.len() {
            self.index += 1;
            return Some(Err(anyhow::anyhow!("Invalid sector count")));
        }
        let chunk_len = u32::from_be_bytes(
            self.anvil.inner[start_index..start_index + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let compression_type = self.anvil.inner[start_index + 4];
        if compression_type >= 128 {
            self.index += 1;
            log::warn!("External chunks are not fully supported");
            return Some(Ok(Chunk {
                location,
                timestamp,
                uncompressed: None,
            }));
        }
        if start_index + chunk_len + 4 > self.anvil.inner.len() {
            self.index += 1;
            return Some(Err(anyhow::anyhow!("Invalid chunk length")));
        }
        match compression_type {
            1 => {
                let mut decoder =
                    GzDecoder::new(&self.anvil.inner[start_index + 5..start_index + chunk_len + 4]);
                if let Err(err) = decoder.read_to_end(&mut uncompressed) {
                    return Some(Err(err.into()));
                }
            }
            2 => {
                let mut decoder = ZlibDecoder::new(
                    &self.anvil.inner[start_index + 5..start_index + chunk_len + 4],
                );
                if let Err(err) = decoder.read_to_end(&mut uncompressed) {
                    return Some(Err(err.into()));
                }
            }
            3 => {
                uncompressed.extend_from_slice(
                    &self.anvil.inner[start_index + 5..start_index + chunk_len + 4],
                );
            }
            4 => {
                let decoder = lz4::Decoder::new(
                    &self.anvil.inner[start_index + 5..start_index + chunk_len + 4],
                );
                let mut decoder = match decoder {
                    Ok(decoder) => decoder,
                    Err(err) => return Some(Err(err.into())),
                };
                if let Err(err) = decoder.read_to_end(&mut uncompressed) {
                    return Some(Err(err.into()));
                }
            }
            _ => return Some(Err(anyhow::anyhow!("Unknown compression type"))),
        }
        self.index += 1;
        Some(Ok(Chunk {
            location,
            timestamp,
            uncompressed: Some(uncompressed),
        }))
    }
}

impl Anvil {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let mut inner = std::fs::read(path)?;
        inner.resize(
            (inner.len() + SECTOR_SIZE - 1) / SECTOR_SIZE * SECTOR_SIZE,
            0,
        );
        if inner.len() < 2 * SECTOR_SIZE {
            return Err(anyhow::anyhow!("Invalid file size"));
        }
        Ok(Self { inner })
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        std::fs::write(path, &self.inner)?;
        Ok(())
    }

    pub fn new() -> Self {
        Self {
            inner: vec![0; SECTOR_SIZE * 2],
        }
    }

    pub fn align(&mut self) -> usize {
        let len = self.inner.len();
        let align = (len + SECTOR_SIZE - 1) / SECTOR_SIZE * SECTOR_SIZE;
        self.inner.resize(align, 0);
        align
    }

    pub fn iter(&self) -> AnvilIter {
        AnvilIter {
            index: 0,
            anvil: self,
        }
    }

    pub fn write(&mut self, chunk: &Chunk) -> anyhow::Result<()> {
        let (location, timestamp, Some(uncompressed)) =
            (chunk.location, chunk.timestamp, &chunk.uncompressed)
        else {
            // External chunk
            self.inner.extend_from_slice(&[0, 0, 0, 1, 2]);
            self.align();
            return Ok(());
        };
        let index = location.1 as usize * 32 + location.0 as usize;
        self.inner[index * 4 + SECTOR_SIZE..index * 4 + SECTOR_SIZE + 4]
            .copy_from_slice(&timestamp.to_be_bytes());
        let start = self.inner.len();
        self.inner.extend_from_slice(&0u32.to_be_bytes());
        self.inner.push(2);
        let mut encoder = ZlibEncoder::new(&mut self.inner, flate2::Compression::default());
        encoder.write_all(uncompressed)?;
        encoder.finish()?;
        let end = self.inner.len();
        self.inner[start..start + 4].copy_from_slice(&((end - start - 4) as u32).to_be_bytes());
        let sector_count = (end - start + SECTOR_SIZE - 1) / SECTOR_SIZE;
        self.inner[index * 4..index * 4 + 4].copy_from_slice(
            &((((start / SECTOR_SIZE) as u32) << 8) | sector_count as u32).to_be_bytes(),
        );
        self.align();
        Ok(())
    }
}

#[cfg(test)]
#[test]
fn test() {
    use rand::Rng;

    let rand_chunk = |rng: &mut rand::rngs::ThreadRng, loc: (i32, i32)| -> Chunk {
        let mut uncompressed = vec![0; 1024];
        rng.fill(&mut uncompressed[..]);
        Chunk {
            location: loc,
            timestamp: rng.gen(),
            uncompressed: Some(uncompressed),
        }
    };

    let mut anvil = Anvil::new();
    let chunk1 = rand_chunk(&mut rand::thread_rng(), (0, 0));
    let chunk2 = rand_chunk(&mut rand::thread_rng(), (20, 20));
    anvil.write(&chunk1).unwrap();
    anvil.write(&chunk2).unwrap();
    let mut iter = anvil.iter();
    let chunk1_read = iter.next().unwrap().unwrap();
    let chunk2_read = iter.next().unwrap().unwrap();
    assert_eq!(chunk1.location, chunk1_read.location);
    assert_eq!(chunk1.timestamp, chunk1_read.timestamp);
    assert_eq!(chunk1.uncompressed, chunk1_read.uncompressed);
    assert_eq!(chunk2.location, chunk2_read.location);
    assert_eq!(chunk2.timestamp, chunk2_read.timestamp);
    assert_eq!(chunk2.uncompressed, chunk2_read.uncompressed);
}
