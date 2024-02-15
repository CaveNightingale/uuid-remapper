use std::collections::HashMap;

use crate::text::visit_text;

use anyhow::Ok;
use uuid::Uuid;

const TAG_END: u8 = 0;
const TAG_BYTE: u8 = 1;
const TAG_SHORT: u8 = 2;
const TAG_INT: u8 = 3;
const TAG_LONG: u8 = 4;
const TAG_FLOAT: u8 = 5;
const TAG_DOUBLE: u8 = 6;
const TAG_BYTE_ARRAY: u8 = 7;
const TAG_STRING: u8 = 8;
const TAG_LIST: u8 = 9;
const TAG_COMPOUND: u8 = 10;
const TAG_INT_ARRAY: u8 = 11;
const TAG_LONG_ARRAY: u8 = 12;

fn is_recursive(kind: u8) -> bool {
    kind == TAG_LIST || kind == TAG_COMPOUND
}

fn worth_visiting(kind: u8) -> bool {
    is_recursive(kind) || kind == TAG_INT_ARRAY
}

fn tag_size(kind: u8) -> Option<usize> {
    match kind {
        TAG_END => Some(0),
        TAG_BYTE => Some(1),
        TAG_SHORT => Some(2),
        TAG_INT => Some(4),
        TAG_LONG => Some(8),
        TAG_FLOAT => Some(4),
        TAG_DOUBLE => Some(8),
        _ => None,
    }
}

fn list_element_size(kind: u8) -> Option<usize> {
    match kind {
        TAG_BYTE_ARRAY => Some(1),
        TAG_INT_ARRAY => Some(4),
        TAG_LONG_ARRAY => Some(8),
        _ => None,
    }
}

pub(crate) fn visit_nbt(nbt: &mut [u8], cb: &impl Fn(Uuid) -> Option<Uuid>) -> anyhow::Result<()> {
    let mut cursor = 0;
    let len = nbt.len();

    macro_rules! check_len {
        ($len:expr) => {
            if (len - cursor) < $len {
                anyhow::bail!("Malformed NBT: Unexpected EOF");
            }
        };
    }

    macro_rules! read {
        ($ty:ty) => {{
            let len = std::mem::size_of::<$ty>();
            check_len!(len);
            let ret = <$ty>::from_be_bytes(nbt[cursor..cursor + len].try_into().unwrap());
            cursor += len;
            ret
        }};
    }

    macro_rules! read_str {
        () => {{
            let len = read!(u16) as usize;
            check_len!(len);
            let ret = &mut nbt[cursor..cursor + len];
            cursor += len;
            ret
        }};
    }

    macro_rules! visit_str {
        () => {{
            visit_text(read_str!(), cb);
        }};
    }

    macro_rules! strip_postfix {
        ($str:expr, $pre:expr) => {{
            let len = $str.len();
            if len >= $pre.len() && &$str[len - $pre.len()..] == $pre {
                Some(&$str[..len - $pre.len()])
            } else {
                None
            }
        }};
    }

    enum VisitFrame {
        // Break the borrow checker since we need to borrow different parts of nbt at the same time
        Compound(HashMap<&'static [u8], (usize, usize)>),
        List { kind: u8, index: usize, len: usize },
    }

    let mut stack = Vec::with_capacity(32);

    macro_rules! visit_compound {
        () => {{
            stack.push(VisitFrame::Compound(HashMap::new()))
        }};
    }

    macro_rules! visit_list {
        () => {
            let ele_kind = read!(u8);
            if let Some(size) = tag_size(ele_kind) {
                cursor += size * read!(u32) as usize;
            } else {
                stack.push(VisitFrame::List {
                    kind: ele_kind,
                    index: 0,
                    len: read!(u32) as usize,
                });
            }
        };
    }

    macro_rules! visit_int_arr {
        () => {{
            let arrlen = read!(u32) as usize;
            if (arrlen == 4) {
                let uuid = Uuid::from_slice(&nbt[cursor..cursor + 16]).unwrap();
                if let Some(new_uuid) = cb(uuid) {
                    nbt[cursor..cursor + 16].copy_from_slice(new_uuid.as_bytes());
                }
            }
            cursor += arrlen * 4;
        }};
    }

    macro_rules! visit_value {
        ($kind:expr) => {{
            if $kind == TAG_INT_ARRAY {
                visit_int_arr!();
            } else if let Some(size) = tag_size($kind) {
                cursor += size;
            } else if let Some(element_size) = list_element_size($kind) {
                let count = read!(u32) as usize;
                cursor += count * element_size;
            } else if $kind == TAG_COMPOUND {
                visit_compound!();
            } else if $kind == TAG_LIST {
                visit_list!();
            } else if $kind == TAG_STRING {
                visit_str!();
            } else {
                anyhow::bail!("Malformed NBT: Unknown tag type {}", $kind);
            }
        }};
    }

    let root_kind = read!(u8);
    read_str!();
    match root_kind {
        TAG_COMPOUND => visit_compound!(),
        TAG_LIST => {
            let kind = read!(u8);
            if worth_visiting(kind) {
                stack.push(VisitFrame::List {
                    kind,
                    index: 0,
                    len: read!(u32) as usize,
                });
            }
        }
        TAG_INT_ARRAY => visit_int_arr!(),
        _ => {}
    }
    if stack.is_empty() {
        return Ok(());
    }
    while let Some(top) = stack.last_mut() {
        match top {
            VisitFrame::Compound(map) => {
                let kind = read!(u8);
                if kind == TAG_END {
                    for (most_p, least_p) in map.values().copied() {
                        let most = nbt[most_p..most_p + 8].try_into().unwrap();
                        let least = nbt[least_p..least_p + 8].try_into().unwrap();
                        let uuid = Uuid::from_u64_pair(
                            u64::from_be_bytes(most),
                            u64::from_be_bytes(least),
                        );
                        if let Some(new_uuid) = cb(uuid) {
                            let (most, least) = new_uuid.as_u64_pair();
                            nbt[most_p..most_p + 8].copy_from_slice(&most.to_be_bytes());
                            nbt[least_p..least_p + 8].copy_from_slice(&least.to_be_bytes());
                        }
                    }
                    stack.pop();
                } else {
                    let name = read_str!();
                    if kind == TAG_LONG {
                        if let Some(field) = strip_postfix!(name, b"UUIDMost") {
                            if let Some((pos, _)) = map.get_mut(field) {
                                *pos = cursor;
                            } else {
                                map.insert(unsafe { std::mem::transmute(field) }, (cursor, 0));
                            };
                        } else if let Some(field) = strip_postfix!(name, b"UUIDLeast") {
                            if let Some((_, pos)) = map.get_mut(field) {
                                *pos = cursor;
                            } else {
                                map.insert(unsafe { std::mem::transmute(field) }, (0, cursor));
                            };
                        }
                    }
                    visit_value!(kind);
                }
            }
            VisitFrame::List { kind, index, len } => {
                if *index == *len {
                    stack.pop();
                } else {
                    *index += 1;
                    visit_value!(*kind);
                }
            }
        }
    }
    if cursor != len {
        anyhow::bail!("Malformed NBT: Unexpected trailing data");
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn test_visit_nbt() {
    use std::collections::HashMap;
    use valence_nbt::{binary::to_binary, from_binary, snbt::from_snbt_str, Compound, Value};

    // Positive test
    // Nbt parsing test
    const FROM: Uuid = Uuid::from_u128(0x1234567890abcdef1234567890abcdef);
    const TO: Uuid = Uuid::from_u128(0xabcdef1234567890abcdef1234567890);
    let replacement = HashMap::from([(FROM, TO)]);
    let snbt = r#"{
        A: "B",
        B: 123b,
        C: 123s,
        D: 123,
        E: 123L,
        F: 123.456,
        G: 123.456f,
        H: [1, 2, 3],
        I: [1L, 2L, 3L],
        J: [1.0, 2.0, 3.0],
        K: [{x: 1, y: 2}, {x: 3, y: 4}],
        L: {x: 6, y: 6},
        M: [L; 4L],
        N: [B; 4b],
        O: [I; 1, 2, 3],
        P: [],
        Q: [I;],
        R: [[]],
        S: {z: 1, w: []},
    }"#;
    let mut nbt = vec![];
    let Value::Compound(mut nbtc) = from_snbt_str(snbt).unwrap() else {
        panic!()
    };
    to_binary(&nbtc, &mut nbt, "").unwrap();
    visit_nbt(&mut nbt, &mut |_| {
        panic!("visit_nbt() claimed to be able to replace UUIDs")
    })
    .unwrap();
    // Nbt pattern matching test
    nbtc.insert(
        "OwnerUUIDMost".to_string(),
        Value::Long((FROM.as_u128() >> 64) as u64 as i64),
    );
    nbtc.insert(
        "OwnerUUIDLeast".to_string(),
        Value::Long(FROM.as_u128() as u64 as i64),
    );
    nbtc.insert("id".to_string(), Value::IntArray(vec![1, 2, 3, 4]));
    let uuid_to_i32_4 = |uuid: Uuid| -> [i32; 4] {
        let u = uuid.as_u128();
        [
            (u >> 96) as i32,
            (u >> 64) as i32,
            (u >> 32) as i32,
            u as i32,
        ]
    };
    nbtc.insert(
        "id1".to_string(),
        Value::IntArray(uuid_to_i32_4(FROM).into()),
    );
    nbtc.insert(
        "UUIDMost".to_string(),
        Value::Long((FROM.as_u128() >> 64) as u64 as i64),
    );
    nbtc.insert(
        "UUIDLeast".to_string(),
        Value::Long(FROM.as_u128() as u64 as i64),
    );
    let mut nbt2 = vec![];
    to_binary(&nbtc, &mut nbt2, "").unwrap();
    visit_nbt(&mut nbt2, &mut |uuid| replacement.get(&uuid).cloned()).unwrap();
    let (de, _): (Compound<String>, String) = from_binary(&mut nbt2.as_slice()).unwrap();
    assert_eq!(
        de.get("OwnerUUIDMost"),
        Some(&Value::Long((TO.as_u128() >> 64) as u64 as i64))
    );
    assert_eq!(
        de.get("OwnerUUIDLeast"),
        Some(&Value::Long(TO.as_u128() as u64 as i64))
    );
    assert_eq!(
        de.get("UUIDMost"),
        Some(&Value::Long((TO.as_u128() >> 64) as u64 as i64))
    );
    assert_eq!(
        de.get("UUIDLeast"),
        Some(&Value::Long(TO.as_u128() as u64 as i64))
    );
    assert_eq!(de.get("id"), Some(&Value::IntArray(vec![1, 2, 3, 4])));
    assert_eq!(
        de.get("id1"),
        Some(&Value::IntArray(uuid_to_i32_4(TO).into()))
    );

    // Negative test
    // Inconsistent string length
    let mut nbt = vec![TAG_COMPOUND, 0, 30, 0];
    assert!(visit_nbt(&mut nbt, &mut |_| None).is_err());
    // Inconsistent list length
    let mut nbt = vec![TAG_COMPOUND, 0, 0, TAG_LIST, 0, 255, 255, 255, 255];
    assert!(visit_nbt(&mut nbt, &mut |_| None).is_err());
    let mut nbt = vec![TAG_COMPOUND, 0, 0, TAG_LIST, 1, 255, 255, 255, 255];
    assert!(visit_nbt(&mut nbt, &mut |_| None).is_err());
    // Illegal tag type
    let mut nbt = vec![TAG_COMPOUND, 0, 0, 255, 0];
    assert!(visit_nbt(&mut nbt, &mut |_| None).is_err());
    // Trailing data
    let mut nbt = vec![TAG_COMPOUND, 0, 0, TAG_END, 0, 0, 0, 0];
    assert!(visit_nbt(&mut nbt, &mut |_| None).is_err());
    // Unpaired UUIDMost/UUIDLeast
    let mut nbtc = Compound::<String>::new();
    nbtc.insert(
        "xxUUIDMost".to_string(),
        Value::Long(FROM.as_u64_pair().0 as i64),
    );
    nbtc.insert(
        "yyUUIDLeast".to_string(),
        Value::Long(FROM.as_u64_pair().1 as i64),
    );
    let mut nbt = vec![];
    to_binary(&nbtc, &mut nbt, "").unwrap();
    assert!(visit_nbt(&mut nbt, &mut |_| None).is_ok());
    let (de, _) = from_binary::<String>(&mut nbt.as_slice()).unwrap();
    assert_eq!(
        de.get("xxUUIDMost"),
        Some(&Value::Long(FROM.as_u64_pair().0 as i64))
    );
    assert_eq!(
        de.get("yyUUIDLeast"),
        Some(&Value::Long(FROM.as_u64_pair().1 as i64))
    ); // Should not be replaced
       // No root tag
    let mut nbt = vec![];
    assert!(visit_nbt(&mut nbt, &mut |_| None).is_err());
    // Non-long UUIDMost/UUIDLeast
    let mut nbtc = Compound::<String>::new();
    nbtc.insert("UUIDMost".to_string(), Value::Int(7));
    nbtc.insert("UUIDLeast".to_string(), Value::Int(32));
    let mut nbt = vec![];
    to_binary(&nbtc, &mut nbt, "").unwrap();
    assert!(visit_nbt(&mut nbt, &mut |_| None).is_ok());
    let (de, _) = from_binary::<String>(&mut nbt.as_slice()).unwrap();
    assert_eq!(de.get("UUIDMost"), Some(&Value::Int(7)));
    assert_eq!(de.get("UUIDLeast"), Some(&Value::Int(32))); // Should not be replaced
}
