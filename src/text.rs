use uuid::Uuid;

// Remap UUIDs in a text buffer
// Don't use &str since performance is critical here
#[allow(clippy::manual_is_ascii_check)]
pub fn visit_text(text: &mut [u8], cb: &impl Fn(Uuid) -> Option<Uuid>) {
    #[inline]
    fn is_digit(c: u8) -> bool {
        (b'0'..=b'9').contains(&c) || (b'a'..=b'f').contains(&c)
    }
    #[inline]
    fn from_hex_char(c: u8) -> u32 {
        if (b'0'..=b'9').contains(&c) {
            (c - b'0') as u32
        } else if (b'a'..=b'f').contains(&c) {
            (c - b'a' + 10) as u32
        } else {
            u32::MAX
        }
    }
    #[inline]
    fn from_hex(str: &[u8]) -> u128 {
        let mut ret = 0;
        for c in str {
            if is_digit(*c) {
                ret = (ret << 4) | from_hex_char(*c) as u128;
            }
        }
        ret
    }
    #[inline]
    fn to_hex_char(c: u32) -> u8 {
        if c < 10 {
            b'0' + c as u8
        } else {
            b'a' + c as u8 - 10
        }
    }
    // Pattern: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
    let mut matched = 0;
    for i in 0..text.len() {
        if !is_digit(text[i]) && text[i] != b'-' {
            matched = 0;
            continue;
        }
        // DFA matching
        macro_rules! dfa_trans_table {
            {$( $current:pat => $next:expr; $other:expr; )*} => {
                matched = match matched {
                    $($current => if text[i] == b'-' {
                        $next
                    } else {
                        $other
                    },)*
                    _ => if is_digit(text[i]) {
                        matched + 1
                    } else {
                        0
                    },
                }
            }
        }
        dfa_trans_table! {
            8 => 9; 8;
            13 => 14; 5;
            18 => 19; 5;
            23 => 24; 5;
        };
        if matched == 36 {
            matched = 0;
            let uuid = Uuid::from_u128(from_hex(&text[i - 35..i + 1]));
            if let Some(new_uuid) = cb(uuid) {
                let new_uuid = new_uuid.as_bytes();
                let mut ptr = 0;
                for c in text[i - 35..i + 1].iter_mut() {
                    if *c == b'-' {
                        continue;
                    }
                    if (ptr & 1) == 0 {
                        *c = to_hex_char((new_uuid[ptr >> 1] >> 4) as u32);
                    } else {
                        *c = to_hex_char((new_uuid[ptr >> 1] & 0xF) as u32);
                    }
                    ptr += 1;
                }
            }
        }
    }

    // Pattern: xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
    let mut matched = 0;
    for i in 0..text.len() {
        if !is_digit(text[i]) {
            matched = 0;
            continue;
        }
        matched += 1;
        if matched == 32 {
            matched = 0;
            let uuid = Uuid::from_u128(from_hex(&text[i - 31..i + 1]));
            if let Some(new_uuid) = cb(uuid) {
                let new_uuid = new_uuid.as_bytes();
                for (ptr, c) in text[i - 31..i + 1].iter_mut().enumerate() {
                    if (ptr & 1) == 0 {
                        *c = to_hex_char((new_uuid[ptr >> 1] >> 4) as u32);
                    } else {
                        *c = to_hex_char((new_uuid[ptr >> 1] & 0xF) as u32);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[test]
fn test_visit_text() {
    use std::str::FromStr;

    use crate::setup_test_logger;

    setup_test_logger();

    let mut text = b"12345678-1234-5678-1234-567812345678".to_vec();
    visit_text(&mut text, &mut |_| {
        Some(Uuid::from_str("00000000-0000-0000-0000-000000000000").unwrap())
    });
    assert_eq!(text, b"00000000-0000-0000-0000-000000000000".to_vec());
    let mut text = b"12345678123456781234567812345678".to_vec();
    visit_text(&mut text, &mut |_| {
        Some(Uuid::from_str("00000000-0000-0000-0000-000000000000").unwrap())
    });
    assert_eq!(text, b"00000000000000000000000000000000".to_vec());
    let mut text = b"12345678-1234-5678-1234-5678-12345678".to_vec();
    visit_text(&mut text, &mut |_| {
        panic!("visit_text() claims to have found a UUID, but it shouldn't have");
    });
    assert_eq!(text, b"12345678-1234-5678-1234-5678-12345678".to_vec());
    let text = br#"{"name":"CaveNightingale", "uuid":"2d318504-1a7b-39dc-8c18-44df798a5c06"}"#;
    let mut text = text.to_vec();
    visit_text(&mut text, &mut |uuid| {
        if uuid == Uuid::from_str("2d318504-1a7b-39dc-8c18-44df798a5c06").unwrap() {
            Some(Uuid::from_str("00000000-0000-0000-0000-000000000000").unwrap())
        } else {
            None
        }
    });
    assert_eq!(
        text,
        br#"{"name":"CaveNightingale", "uuid":"00000000-0000-0000-0000-000000000000"}"#
    );
}
