use std::{collections::HashMap, path::Path, str::FromStr};

use clap::ValueEnum;
use indicatif::ProgressBar;
use serde::Deserialize;
use uuid::Uuid;

use crate::MULTI;

#[derive(Debug, Clone, Copy, ValueEnum)]
/// Specify the mapping kind
pub enum MappingKind {
    /// Read mapping from csv file, the first column is the original uuid, the second column is the new uuid
    /// The first line does not matter
    Csv,
    /// Read mapping from json file
    Json,
    /// Convert the following player to offline mode, each line is a player name
    ListToOffline,
    /// Convert the following player to online mode, each line is a player name
    ListToOnline,
    /// Convert the following player to offline mode, input is usercache.json
    UsercacheToOffline,
    /// Convert the following player to online mode, input is usercache.json
    UsercacheToOnline,
    /// Read two username from each line, the first is the original username, the second is the new username
    /// Can be used to rename players in offline mode.
    /// The first line does not matter
    OfflineRenameCsv,
}

fn load_csv(path: &Path) -> anyhow::Result<HashMap<Uuid, Uuid>> {
    let mut map = HashMap::new();
    for line in std::fs::read_to_string(path)?.lines().skip(1) {
        let mut iter = line.split(',');
        let Some(x) = iter.next().and_then(|x| Uuid::from_str(x).ok()) else {
            continue;
        };
        let Some(y) = iter.next().and_then(|y| Uuid::from_str(y).ok()) else {
            continue;
        };
        if iter.next().is_some() {
            continue;
        };
        map.insert(x, y);
    }
    Ok(map)
}

fn online_uuids<'a>(name: impl IntoIterator<Item = &'a String>) -> HashMap<String, Uuid> {
    #[derive(Deserialize)]
    struct Res {
        id: Uuid,
        name: String,
    }
    let mut ret = HashMap::new();
    let list = name.into_iter().collect::<Vec<_>>();
    let chunks = list.chunks(10); // Mojang API limit
    let pg = MULTI.add(ProgressBar::new(chunks.len() as u64));
    pg.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("Receiving online uuids: [{bar:60.cyan/blue}] {pos} chunks / {len} chunks")
            .unwrap()
            .progress_chars("#>-"),
    );
    for chunk in chunks {
        ret.extend(
            reqwest::blocking::Client::new()
                .post("https://api.mojang.com/profiles/minecraft")
                .json(&chunk.iter().collect::<Vec<_>>())
                .send()
                .ok()
                .and_then(|x| x.json::<Vec<Res>>().ok())
                .unwrap_or_default()
                .into_iter()
                .map(|x| (x.name, x.id)),
        );
        pg.inc(1);
    }
    ret
}

fn offline_uuid(name: &str) -> Uuid {
    let str = "OfflinePlayer:".to_owned() + name;
    let mut md5 = md5::compute(str.as_bytes());
    // Copied from JDK source code, don't know why
    md5[6] &= 0x0f; /* clear version        */
    md5[6] |= 0x30; /* set to version 3     */
    md5[8] &= 0x3f; /* clear variant        */
    md5[8] |= 0x80; /* set to IETF variant  */
    Uuid::from_bytes(md5.0)
}

fn offline_uuids<'a>(name: impl IntoIterator<Item = &'a String>) -> HashMap<String, Uuid> {
    name.into_iter()
        .map(|x| (x.to_string(), offline_uuid(x)))
        .collect()
}

// a_compose_b_inverse(a, b) = { (x, y) | exists z: a(z) = x and b(z) = y }
fn a_compose_b_inverse(
    a: &HashMap<String, Uuid>,
    b: &HashMap<String, Uuid>,
) -> HashMap<Uuid, Uuid> {
    let mut map = HashMap::new();
    for (z, x) in a {
        if let Some(y) = b.get(z) {
            map.insert(*x, *y);
        }
    }
    map
}

fn load_name_list(path: &Path) -> anyhow::Result<Vec<String>> {
    Ok(std::fs::read_to_string(path)?
        .lines()
        .map(|x| x.trim().to_string())
        .collect())
}

fn load_name_list_from_usercache(path: &Path) -> anyhow::Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Res {
        name: String,
    }
    Ok(
        serde_json::from_str::<Vec<Res>>(&std::fs::read_to_string(path)?)?
            .into_iter()
            .map(|x| x.name)
            .collect(),
    )
}

pub fn load_offline_rename(path: &Path) -> anyhow::Result<HashMap<Uuid, Uuid>> {
    let mut map = HashMap::new();
    for line in std::fs::read_to_string(path)?.lines().skip(1) {
        let mut iter = line.split(',');
        let Some(x) = iter.next() else {
            continue;
        };
        let Some(y) = iter.next() else {
            continue;
        };
        if iter.next().is_some() {
            continue;
        };
        map.insert(offline_uuid(x), offline_uuid(y));
    }
    Ok(map)
}

pub fn get_mapping(kind: MappingKind, path: &Path) -> anyhow::Result<HashMap<Uuid, Uuid>> {
    match kind {
        MappingKind::Csv => load_csv(path),
        MappingKind::Json => {
            let map = std::fs::read_to_string(path)?;
            Ok(serde_json::from_str(&map)?)
        },
        MappingKind::ListToOffline => {
            let names = load_name_list(path)?;
            Ok(a_compose_b_inverse(
                &online_uuids(&names),
                &offline_uuids(&names),
            ))
        },
        MappingKind::ListToOnline => {
            let names = load_name_list(path)?;
            Ok(a_compose_b_inverse(
                &offline_uuids(&names),
                &online_uuids(&names),
            ))
        },
        MappingKind::UsercacheToOffline => {
            let names = load_name_list_from_usercache(path)?;
            Ok(a_compose_b_inverse(
                &online_uuids(&names),
                &offline_uuids(&names),
            ))
        },
        MappingKind::UsercacheToOnline => {
            let names = load_name_list_from_usercache(path)?;
            Ok(a_compose_b_inverse(
                &offline_uuids(&names),
                &online_uuids(&names),
            ))
        },
        MappingKind::OfflineRenameCsv => load_offline_rename(path),
    }
}

#[cfg(test)]
#[test]
fn test_api() {
    println!(
        "{:?}",
        get_mapping(
            MappingKind::UsercacheToOnline,
            Path::new("test/usercache.json")
        )
        .unwrap()
    );
}
