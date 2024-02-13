use std::ffi::OsString;
#[allow(unused_imports)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::{io::Read, path::Path};

use std::io::Write;

use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::{GzEncoder, ZlibEncoder};
use uuid::Uuid;

use crate::{anvil::Anvil, nbt::visit_nbt, text::visit_text};

fn remap_mca(path: &Path, cb: &impl Fn(Uuid) -> Option<Uuid>) -> anyhow::Result<()> {
    let input = Anvil::open(path)?;
    let mut output = Anvil::new();
    for block in input.iter() {
        if let Err(err) = (|| -> anyhow::Result<()> {
            let mut chunk = block?;
            visit_nbt(&mut chunk.uncompressed, cb)?;
            output.write(&chunk)?;
            Ok(())
        })() {
            log::error!("Failed to visit chunk {:#?}", err);
        }
    }
    output.save(path)?;
    Ok(())
}

fn remap_mcc(path: &Path, cb: &impl Fn(Uuid) -> Option<Uuid>) -> anyhow::Result<()> {
    let mut chunk = std::fs::read(path)?;
    log::warn!("A external chunk (outside of anvil) is found. We only support Zlib compression in this case. At {}", path.display());
    let mut decoder = ZlibDecoder::<&[u8]>::new(&chunk);
    let mut uncompressed = Vec::new();
    decoder.read_to_end(&mut uncompressed)?;
    chunk.clear();
    visit_nbt(&mut uncompressed, cb)?;
    let mut encoder = ZlibEncoder::new(&mut chunk, flate2::Compression::default());
    encoder.write_all(&uncompressed)?;
    encoder.finish()?;
    std::fs::write(path, &chunk)?;
    Ok(())
}

fn remap_dat(path: &Path, cb: &impl Fn(Uuid) -> Option<Uuid>) -> anyhow::Result<()> {
    let mut chunk = std::fs::read(path)?;
    let mut decoder = GzDecoder::<&[u8]>::new(&chunk);
    let mut uncompressed = Vec::new();
    if decoder.read_to_end(&mut uncompressed).is_err() {
        // Not a Gzip file? try raw nbt
        visit_nbt(&mut chunk, cb)?;
        std::fs::write(path, &chunk)?;
        return Ok(());
    };
    chunk.clear();
    visit_nbt(&mut uncompressed, cb)?;
    let mut encoder = GzEncoder::new(&mut chunk, flate2::Compression::default());
    encoder.write_all(&uncompressed)?;
    encoder.finish()?;
    std::fs::write(path, &chunk)?;
    Ok(())
}

fn remap_nbt(path: &Path, cb: &impl Fn(Uuid) -> Option<Uuid>) -> anyhow::Result<()> {
    let mut chunk = std::fs::read(path)?;
    visit_nbt(&mut chunk, cb)?;
    std::fs::write(path, &chunk)?;
    Ok(())
}

fn remap_text(path: &Path, cb: &impl Fn(Uuid) -> Option<Uuid>) -> anyhow::Result<()> {
    let mut text = std::fs::read(path)?;
    visit_text(&mut text, cb);
    std::fs::write(path, &text)?;
    Ok(())
}

pub fn remap_file(
    world: &Path,
    path: &Path,
    cb: &impl Fn(Uuid) -> Option<Uuid>,
) -> anyhow::Result<()> {
    let concated = world.join(path);
    if concated.is_file() {
        // Remap the file content
        match path.extension().and_then(|s| s.to_str()) {
            Some("mca") => remap_mca(&concated, cb)?,
            Some("mcc") => remap_mcc(&concated, cb)?,
            Some("dat") => remap_dat(&concated, cb)?,
            Some("nbt") => remap_nbt(&concated, cb)?,
            Some("txt" | "json" | "json5") => remap_text(&concated, cb)?,
            _ => log::warn!("Unsupported file type: {}", concated.display()),
        }

        // Remap the file name
        let path = path.as_os_str().to_os_string();

        #[cfg(not(target_family = "windows"))]
        let mut new_path = path.into_vec();
        #[cfg(target_family = "windows")]
        let mut newpath = if let Some(path) = path.to_str() {
            path.as_bytes().to_vec()
        } else {
            anyhow::bail!("Illegal character in file name {}", path.to_string_lossy())
        };

        visit_text(&mut new_path, cb);
        let new_concated = world.join(OsString::from_vec(new_path));
        let new_concated = Path::new(&new_concated);
        if new_concated != concated {
            std::fs::rename(&concated, new_concated)?;
        }
    } else {
        log::warn!("Unsupported file type: {}", concated.display());
    }
    Ok(())
}

/// Check if the file requires remapping
pub fn require_remapping(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("mca" | "mcc" | "dat" | "nbt" | "txt" | "json" | "json5")
    ) && std::fs::metadata(path)
        .map(|m| m.is_file() && m.len() > 0 && !m.permissions().readonly())
        .unwrap_or(false)
}

#[cfg(test)]
#[test]
fn test() {
    use std::{path::PathBuf, str::FromStr};

    env_logger::init();

    let path = PathBuf::from("test.mca");
    remap_mca(&path, &|_| None).unwrap();

    remap_file(
        &PathBuf::from("test/stats"),
        &PathBuf::from("2d318504-1a7b-39dc-8c18-44df798a5c06.json"),
        &|uuid| {
            if uuid == Uuid::from_str("2d318504-1a7b-39dc-8c18-44df798a5c06").unwrap() {
                Some(Uuid::from_str("00000000-0000-0000-0000-000000000000").unwrap())
            } else {
                None
            }
        },
    ).unwrap();
}
