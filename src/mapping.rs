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
        }
        MappingKind::ListToOffline => {
            let names = load_name_list(path)?;
            Ok(a_compose_b_inverse(
                &online_uuids(&names),
                &offline_uuids(&names),
            ))
        }
        MappingKind::ListToOnline => {
            let names = load_name_list(path)?;
            Ok(a_compose_b_inverse(
                &offline_uuids(&names),
                &online_uuids(&names),
            ))
        }
        MappingKind::UsercacheToOffline => {
            let names = load_name_list_from_usercache(path)?;
            Ok(a_compose_b_inverse(
                &online_uuids(&names),
                &offline_uuids(&names),
            ))
        }
        MappingKind::UsercacheToOnline => {
            let names = load_name_list_from_usercache(path)?;
            Ok(a_compose_b_inverse(
                &offline_uuids(&names),
                &online_uuids(&names),
            ))
        }
        MappingKind::OfflineRenameCsv => load_offline_rename(path),
    }
}

#[cfg(test)]
#[test]
fn test() {
    use crate::setup_test_logger;

    setup_test_logger();

    let csv_file = "from,to\n\
    00000000-0000-0000-0000-000000000000,00000000-0000-0000-0000-000000000001\n\
    00000000-0000-0000-0000-000000000002,00000000-0000-0000-0000-000000000003";
    let csv_path = std::env::temp_dir().join("test.csv");
    std::fs::write(&csv_path, csv_file).unwrap();
    assert_eq!(
        get_mapping(MappingKind::Csv, &csv_path).unwrap(),
        vec![
            (
                Uuid::from_str("00000000-0000-0000-0000-000000000000").unwrap(),
                Uuid::from_str("00000000-0000-0000-0000-000000000001").unwrap()
            ),
            (
                Uuid::from_str("00000000-0000-0000-0000-000000000002").unwrap(),
                Uuid::from_str("00000000-0000-0000-0000-000000000003").unwrap()
            ),
        ]
        .into_iter()
        .collect()
    );
    std::fs::remove_file(csv_path).unwrap();

    let json_file = r#"{
        "00000000-0000-0000-0000-000000000000": "00000000-0000-0000-0000-000000000001",
        "00000000-0000-0000-0000-000000000002": "00000000-0000-0000-0000-000000000003"
    }"#;
    let json_path = std::env::temp_dir().join("test.json");
    std::fs::write(&json_path, json_file).unwrap();
    assert_eq!(
        get_mapping(MappingKind::Json, &json_path).unwrap(),
        vec![
            (
                Uuid::from_str("00000000-0000-0000-0000-000000000000").unwrap(),
                Uuid::from_str("00000000-0000-0000-0000-000000000001").unwrap()
            ),
            (
                Uuid::from_str("00000000-0000-0000-0000-000000000002").unwrap(),
                Uuid::from_str("00000000-0000-0000-0000-000000000003").unwrap()
            ),
        ]
        .into_iter()
        .collect()
    );
    std::fs::remove_file(json_path).unwrap();

    let list_file = "a\nb\nc";
    let list_path = std::env::temp_dir().join("test.list");
    std::fs::write(&list_path, list_file).unwrap();
    assert_eq!(
        load_name_list(&list_path).unwrap(),
        vec!["a".to_string(), "b".to_string(), "c".to_string(),]
    );
    std::fs::remove_file(list_path).unwrap();

    let usercache_file = r#"[{"name":"a"},{"name":"b"},{"name":"c"}]"#;
    let usercache_path = std::env::temp_dir().join("test.usercache.json");
    std::fs::write(&usercache_path, usercache_file).unwrap();
    assert_eq!(
        load_name_list_from_usercache(&usercache_path).unwrap(),
        vec!["a".to_string(), "b".to_string(), "c".to_string(),]
    );
    std::fs::remove_file(usercache_path).unwrap();

    let offline_rename_file = "from,to\na,b\nc,d";
    let offline_rename_path = std::env::temp_dir().join("test.offline_rename.csv");
    std::fs::write(&offline_rename_path, offline_rename_file).unwrap();
    assert_eq!(
        get_mapping(MappingKind::OfflineRenameCsv, &offline_rename_path).unwrap(),
        vec![
            (offline_uuid("a"), offline_uuid("b")),
            (offline_uuid("c"), offline_uuid("d")),
        ]
        .into_iter()
        .collect()
    );
    std::fs::remove_file(offline_rename_path).unwrap();

    assert_eq!(
        offline_uuid("CaveNightingale"),
        Uuid::from_str("2d318504-1a7b-39dc-8c18-44df798a5c06").unwrap()
    );
    let online_uuids_result = online_uuids(
        vec![
            "CaveNightingale".to_string(),
            "Notch".to_string(),
            "Dinnerbone".to_string(),
        ]
        .iter(),
    );
    assert_eq!(
        online_uuids_result.get("CaveNightingale").unwrap(),
        &Uuid::from_str("fb1ad51e-cf1f-41f7-8fd1-10dff164b17d").unwrap()
    );
    assert_eq!(
        online_uuids_result.get("Notch").unwrap(),
        &Uuid::from_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap()
    );
    assert_eq!(
        online_uuids_result.get("Dinnerbone").unwrap(),
        &Uuid::from_str("61699b2e-d327-4a01-9f1e-0ea8c3f06bc6").unwrap()
    );
    let composed = a_compose_b_inverse(
        &online_uuids_result,
        &offline_uuids(
            vec![
                "CaveNightingale".to_string(),
                "Notch".to_string(),
                "Dinnerbone".to_string(),
            ]
            .iter(),
        ),
    );
    assert_eq!(
        composed.get(&Uuid::from_str("fb1ad51e-cf1f-41f7-8fd1-10dff164b17d").unwrap()),
        Some(&offline_uuid("CaveNightingale"))
    );
    assert_eq!(
        composed.get(&Uuid::from_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap()),
        Some(&offline_uuid("Notch"))
    );
    assert_eq!(
        composed.get(&Uuid::from_str("61699b2e-d327-4a01-9f1e-0ea8c3f06bc6").unwrap()),
        Some(&offline_uuid("Dinnerbone"))
    );
}
