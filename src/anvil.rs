use anyhow::Context;
use flate2::{
    read::{GzDecoder, ZlibDecoder},
    write::ZlibEncoder,
};
use std::path::{Path, PathBuf};
use std::{
    fmt::Display,
    io::{Read, Write},
};

const SECTOR_SIZE: usize = 4096;
const MAX_CHUNK_NUM: usize = 1024;

const COMPRESSION_KIND_GZIP: u8 = 1;
const COMPRESSION_KIND_ZLIB: u8 = 2;
const COMPRESSION_KIND_RAW: u8 = 3;
const COMPRESSION_KIND_LZ4: u8 = 4;
const COMPRESSION_EXTERNAL: u8 = 128;

pub struct Anvil {
    path: PathBuf,
    content: Vec<u8>,
}

#[derive(Debug)]
pub struct Chunk {
    // Whether the chunk is stored in an external file originally
    // If so, the external chunk will be deleted when the chunk is written
    pub external: bool,
    pub location: (i32, i32),
    pub timestamp: i32,
    pub uncompressed: Vec<u8>,
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
        macro_rules! u32_at {
            ($pos:expr) => {
                u32::from_be_bytes(self.anvil.content[$pos..$pos + 4].try_into().unwrap())
            };
        }

        // Read chunk metadata
        while self.index < MAX_CHUNK_NUM && u32_at!(self.index * 4) == 0 {
            self.index += 1;
        }
        if self.index == MAX_CHUNK_NUM {
            return None;
        }
        let location = (
            (self.index & 0x1F) as i32,
            ((self.index >> 5) & 0x1F) as i32,
        );
        let offset = u32_at!(self.index * 4);
        let (offset, sector_count) = (offset >> 8, offset & 0xFF);
        let timestamp = u32_at!(self.index * 4 + SECTOR_SIZE) as i32;
        let start = offset as usize * SECTOR_SIZE;
        if start + SECTOR_SIZE * sector_count as usize > self.anvil.content.len() {
            self.index += 1;
            return Some(Err(anyhow::anyhow!("Invalid sector count")));
        }
        let chunk_len = u32_at!(start) as usize;
        if start + chunk_len + 4 > self.anvil.content.len() {
            self.index += 1;
            return Some(Err(anyhow::anyhow!("Invalid chunk length")));
        }

        // Uncompress chunk
        let mut uncompressed = Vec::new();
        let mut compression_type = self.anvil.content[start + 4];
        let mut external = false;
        let external_data;
        let compressed = if compression_type >= COMPRESSION_EXTERNAL {
            compression_type -= COMPRESSION_EXTERNAL;
            let Ok(external_path) = self.anvil.external_location(location) else {
                self.index += 1;
                return Some(Err(anyhow::anyhow!(
                    "Invalid global location for external chunk"
                )));
            };
            external_data = match std::fs::read(external_path) {
                Ok(data) => data,
                Err(err) => {
                    self.index += 1;
                    return Some(Err(err.into()));
                }
            };
            external = true;
            &external_data
        } else {
            &self.anvil.content[start + 5..start + chunk_len + 4]
        };
        match compression_type {
            COMPRESSION_KIND_GZIP => {
                let mut decoder = GzDecoder::new(compressed);
                if let Err(err) = decoder.read_to_end(&mut uncompressed) {
                    return Some(Err(err.into()));
                }
            }
            COMPRESSION_KIND_ZLIB => {
                let mut decoder = ZlibDecoder::new(compressed);
                if let Err(err) = decoder.read_to_end(&mut uncompressed) {
                    return Some(Err(err.into()));
                }
            }
            COMPRESSION_KIND_RAW => {
                uncompressed.extend_from_slice(compressed);
            }
            COMPRESSION_KIND_LZ4 => {
                let decoder = lz4::Decoder::new(compressed);
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
            external,
            location,
            timestamp,
            uncompressed,
        }))
    }
}

impl Anvil {
    /// Get the global location of the anvil file
    fn external_location(&self, local: (i32, i32)) -> anyhow::Result<PathBuf> {
        let filename = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .context("Invalid file name")?;
        let mut parts = filename.split('.').skip(1);
        let x = parts
            .next()
            .and_then(|s| s.parse::<i64>().ok())
            .context("Invalid x coordinate")?;
        let z = parts
            .next()
            .and_then(|s| s.parse::<i64>().ok())
            .context("Invalid z coordinate")?;
        Ok(self
            .path
            .with_file_name(format!("c.{}.{}.mcc", x * 32 + local.0 as i64, z * 32 + local.1 as i64)))
    }

    /// Open an anvil file
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let mut inner = std::fs::read(path)?;
        inner.resize(
            (inner.len() + SECTOR_SIZE - 1) / SECTOR_SIZE * SECTOR_SIZE,
            0,
        );
        if inner.len() < 2 * SECTOR_SIZE {
            return Err(anyhow::anyhow!("Invalid file size"));
        }
        Ok(Self {
            path: path.to_path_buf(),
            content: inner,
        })
    }

    /// Save the anvil file, except for the external chunks, which is saved when the chunk is written
    pub fn save(&self) -> anyhow::Result<()> {
        std::fs::write(&self.path, &self.content)?;
        Ok(())
    }

    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            content: vec![0; SECTOR_SIZE * 2],
        }
    }

    pub fn align(&mut self) -> usize {
        let len = self.content.len();
        let align = (len + SECTOR_SIZE - 1) / SECTOR_SIZE * SECTOR_SIZE;
        self.content.resize(align, 0);
        align
    }

    pub fn iter(&self) -> AnvilIter {
        AnvilIter {
            index: 0,
            anvil: self,
        }
    }

    pub fn write(&mut self, chunk: &Chunk) -> anyhow::Result<()> {
        let Chunk {
            external,
            location,
            timestamp,
            uncompressed,
        } = chunk;
        let index = location.1 as usize * 32 + location.0 as usize;
        self.content[index * 4 + SECTOR_SIZE..index * 4 + SECTOR_SIZE + 4]
            .copy_from_slice(&timestamp.to_be_bytes());
        self.content.extend_from_slice(&0u32.to_be_bytes());
        let start = self.content.len();
        self.content.push(COMPRESSION_KIND_ZLIB);
        let mut encoder = ZlibEncoder::new(&mut self.content, flate2::Compression::default());
        encoder.write_all(uncompressed)?;
        encoder.finish()?;
        let end = self.content.len();
        let mut len = end - start;
        let mut sector_count = len.div_ceil(SECTOR_SIZE);
        // Unlikely: If the chunk is too large, we need to move it to external file
        if sector_count > u8::MAX as usize {
            let external_path = self.external_location(*location)?;
            log::info!(
                "Chunk is too large, moved to external file {}",
                external_path.display()
            );
            std::fs::write(&external_path, &self.content[start + 1..end])?;
            self.content.truncate(start);
            self.content
                .push(COMPRESSION_EXTERNAL + COMPRESSION_KIND_ZLIB);
            sector_count = 1;
            len = 1;
        } else if *external {
            let external_path = self.external_location(*location)?;
            log::info!(
                "Chunk is previously in external file {}, but now moved to internal",
                external_path.display()
            );
            std::fs::remove_file(&external_path)?;
        };
        self.content[start - 4..start].copy_from_slice(&(len as u32).to_be_bytes());
        self.content[index * 4..index * 4 + 4].copy_from_slice(
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
            external: false,
            location: loc,
            timestamp: rng.gen(),
            uncompressed: uncompressed,
        }
    };

    let mut anvil = Anvil::new(Path::new("r.0.0.mca"));
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
    assert_eq!(false, chunk1_read.external);
    assert_eq!(chunk2.location, chunk2_read.location);
    assert_eq!(chunk2.timestamp, chunk2_read.timestamp);
    assert_eq!(chunk2.uncompressed, chunk2_read.uncompressed);
    assert_eq!(false, chunk2_read.external);

    let rand_large_chunk = |rng: &mut rand::rngs::ThreadRng, loc: (i32, i32)| -> Chunk {
        let mut uncompressed = vec![0; 8 * 1024 * 1024];
        rng.fill(&mut uncompressed[..]);
        Chunk {
            external: false,
            location: loc,
            timestamp: rng.gen(),
            uncompressed: uncompressed,
        }
    };
    let mut anvil = Anvil::new(Path::new("r.-1.-1.mca"));
    let chunk = rand_large_chunk(&mut rand::thread_rng(), (0, 0));
    anvil.write(&chunk).unwrap();
    assert!(Path::new("c.-32.-32.mcc").exists()); // External file
    let mut iter = anvil.iter();
    let chunk_read = iter.next().unwrap().unwrap();
    assert_eq!(chunk.location, chunk_read.location);
    assert_eq!(chunk.timestamp, chunk_read.timestamp);
    assert_eq!(chunk.uncompressed, chunk_read.uncompressed);
    assert_eq!(true, chunk_read.external);
    anvil.save().unwrap();
    anvil = Anvil::open(Path::new("r.-1.-1.mca")).unwrap();
    let mut iter = anvil.iter();
    let chunk_read = iter.next().unwrap().unwrap();
    assert_eq!(chunk.location, chunk_read.location);
    assert_eq!(chunk.timestamp, chunk_read.timestamp);
    assert_eq!(chunk.uncompressed, chunk_read.uncompressed);
    assert_eq!(true, chunk_read.external);
    anvil.write(&Chunk {
        external: true,
        location: (0, 0),
        timestamp: 0,
        uncompressed: vec![0; 1024],
    }).unwrap();
    assert!(!Path::new("c.-32.-32.mcc").exists());

    // TODO: Poor test coverage

    std::fs::remove_file("r.-1.-1.mca").unwrap();
}
