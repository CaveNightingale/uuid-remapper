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

type UuidBitLoc<'a> = Option<&'a mut [u8]>;

enum VisitFrame<'a> {
    Compound(HashMap<&'a [u8], (UuidBitLoc<'a>, UuidBitLoc<'a>)>),
    List { kind: u8, index: usize, len: usize },
}

struct NbtReader<'a, 'b, F: Fn(Uuid) -> Option<Uuid>> {
    nbt: &'a mut [u8],
    callback: &'b F,
}

impl<'a, 'b, F: Fn(Uuid) -> Option<Uuid>> NbtReader<'a, 'b, F> {
    fn new(nbt: &'a mut [u8], callback: &'b F) -> Self {
        Self { nbt, callback }
    }

    fn take(&mut self, len: usize) -> anyhow::Result<&'a mut [u8]> {
        if len > self.nbt.len() {
            anyhow::bail!("Malformed NBT: Unexpected EOF");
        }
        let (head, tail) = std::mem::take(&mut self.nbt).split_at_mut(len);
        self.nbt = tail;
        Ok(head)
    }

    fn take_str(&mut self) -> anyhow::Result<&'a mut [u8]> {
        let len = u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as usize;
        self.take(len)
    }

    fn visit_str(&mut self) -> anyhow::Result<()> {
        visit_text(self.take_str()?, self.callback);
        Ok(())
    }

    fn visit_uuid(&self, most: &mut [u8], least: &mut [u8]) -> anyhow::Result<()> {
        let omost = u64::from_be_bytes(most.try_into().unwrap());
        let oleast = u64::from_be_bytes(least.try_into().unwrap());
        let uuid = Uuid::from_u64_pair(omost, oleast);
        if let Some(new_uuid) = (self.callback)(uuid) {
            let (nmost, nleast) = new_uuid.as_u64_pair();
            most.copy_from_slice(&nmost.to_be_bytes());
            least.copy_from_slice(&nleast.to_be_bytes());
        }
        Ok(())
    }

    fn visit_value(&mut self, stack: &mut Vec<VisitFrame<'a>>, kind: u8) -> anyhow::Result<()> {
        if kind == TAG_INT_ARRAY {
            let count = u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize;
            if count == 4 {
                let most = self.take(8)?;
                let least = self.take(8)?;
                self.visit_uuid(most, least)?;
            } else {
                self.take(count * 4)?;
            }
        } else if let Some(size) = tag_size(kind) {
            self.take(size)?;
        } else if let Some(element_size) = list_element_size(kind) {
            let count = u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize;
            self.take(count * element_size)?;
        } else if kind == TAG_COMPOUND {
            stack.push(VisitFrame::Compound(HashMap::new()));
        } else if kind == TAG_LIST {
            let ele_kind = self.take(1)?[0];
            let count = u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize;
            if let Some(size) = tag_size(ele_kind) {
                self.take(size * count)?;
            } else {
                stack.push(VisitFrame::List {
                    kind: ele_kind,
                    index: 0,
                    len: count,
                });
            }
        } else if kind == TAG_STRING {
            self.visit_str()?;
        } else {
            anyhow::bail!("Malformed NBT: Unknown tag type {}", kind);
        }
        Ok(())
    }

    fn step(&mut self, stack: &mut Vec<VisitFrame<'a>>) -> anyhow::Result<bool> {
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

        let Some(top) = stack.last_mut() else {
            return Ok(false);
        };
        match top {
            VisitFrame::Compound(map) => {
                let kind = self.take(1)?[0];
                if kind == TAG_END {
                    let Some(VisitFrame::Compound(map)) = stack.pop() else {
                        unreachable!();
                    };
                    for uuid in map.into_values() {
                        if let (Some(most_p), Some(least_p)) = uuid {
                            self.visit_uuid(most_p, least_p)?;
                        }
                    }
                } else {
                    let name = self.take_str()?;
                    if kind == TAG_LONG {
                        if let Some(field) = strip_postfix!(name, b"UUIDMost") {
                            if let Some((pos, _)) = map.get_mut(field) {
                                *pos = Some(self.take(8)?);
                            } else {
                                map.insert(field, (Some(self.take(8)?), None));
                            };
                        } else if let Some(field) = strip_postfix!(name, b"UUIDLeast") {
                            if let Some((_, pos)) = map.get_mut(field) {
                                *pos = Some(self.take(8)?);
                            } else {
                                map.insert(field, (None, Some(self.take(8)?)));
                            };
                        } else {
                            self.visit_value(stack, kind)?;
                        }
                    } else {
                        self.visit_value(stack, kind)?;
                    }
                }
            }
            VisitFrame::List { kind, index, len } => {
                if *index == *len {
                    stack.pop();
                } else {
                    *index += 1;
                    let kind = *kind;
                    self.visit_value(stack, kind)?;
                }
            }
        }
        Ok(true)
    }

    fn process(&mut self) -> anyhow::Result<()> {
        let mut stack = Vec::with_capacity(32);
        let root_kind = self.take(1)?[0];
        self.take_str()?;
        self.visit_value(&mut stack, root_kind)?;
        while self.step(&mut stack)? {}
        if !self.nbt.is_empty() {
            anyhow::bail!("Malformed NBT: Unexpected trailing data");
        }
        Ok(())
    }
}

pub(crate) fn visit_nbt(nbt: &mut [u8], cb: &impl Fn(Uuid) -> Option<Uuid>) -> anyhow::Result<()> {
    NbtReader::new(nbt, cb).process()
}

#[cfg(test)]
#[test]
fn test_visit_nbt() {
    use std::collections::HashMap;
    use valence_nbt::{binary::to_binary, from_binary, snbt::from_snbt_str, Compound, Value};

    use crate::setup_test_logger;

    setup_test_logger();

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
