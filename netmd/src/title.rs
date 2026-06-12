use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

use crate::util::encode_to_sjis;

pub fn half_width_to_full_width_range(range: &str) -> String {
    range
        .chars()
        .filter_map(|c| match c {
            '0' => Some('０'),
            '1' => Some('１'),
            '2' => Some('２'),
            '3' => Some('３'),
            '4' => Some('４'),
            '5' => Some('５'),
            '6' => Some('６'),
            '7' => Some('７'),
            '8' => Some('８'),
            '9' => Some('９'),
            '-' => Some('－'),
            '/' => Some('／'),
            ';' => Some('；'),
            _ => None,
        })
        .collect()
}

pub fn get_half_width_title_length(title: &str) -> usize {
    title.encode_utf16().count()
        + title
            .chars()
            .filter(|c| is_multi_byte_when_half_width_sanitized(*c))
            .count()
}

pub fn sanitize_half_width_title(title: &str) -> String {
    let original_title = title;
    let title = flatten_dakuten(&sanitize_full_width_title_remap(title));

    let new_title: String = title
        .chars()
        .map(|c| {
            if let Some(mapped) = map_to_half_width(c) {
                return mapped;
            }
            if c.is_ascii() || is_allowed_half_width_char(c) {
                return c.to_string();
            }

            let without_diacritics = remove_diacritics(c);
            if !without_diacritics.is_empty() && without_diacritics.chars().all(|c| c.is_ascii()) {
                without_diacritics
            } else {
                " ".to_string()
            }
        })
        .collect();

    match encode_to_sjis(&new_title) {
        Ok(encoded) if encoded.len() == get_half_width_title_length(original_title) => new_title,
        _ => aggressive_sanitize_title(original_title),
    }
}

pub fn sanitize_full_width_title(title: &str) -> String {
    let new_title = sanitize_full_width_title_remap(title);
    match encode_to_sjis(&new_title) {
        Ok(encoded) if encoded.len() == title.encode_utf16().count() * 2 => new_title,
        _ => aggressive_sanitize_title(title),
    }
}

fn sanitize_full_width_title_remap(title: &str) -> String {
    title
        .chars()
        .map(|c| {
            let jp = map_to_full_width_jp(c).unwrap_or_else(|| c.to_string());
            let ru = map_russian(&jp).unwrap_or(&jp).to_string();
            map_german(&ru).unwrap_or(&ru).to_string()
        })
        .collect()
}

fn aggressive_sanitize_title(title: &str) -> String {
    title
        .nfd()
        .filter(|c| c.is_ascii() && !is_combining_mark(*c))
        .collect()
}

fn remove_diacritics(c: char) -> String {
    c.to_string()
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect()
}

#[derive(Clone, Copy)]
enum MarkType {
    Normal,
    Dakuten,
    Handakuten,
}

fn flatten_dakuten(title: &str) -> String {
    let mut fixed = Vec::new();
    let mut mark = MarkType::Normal;

    for c in title.chars().rev() {
        match mark {
            MarkType::Dakuten if is_dakuten_possible(c) => {
                fixed.push(char::from_u32(c as u32 + 1).unwrap_or(c));
                mark = MarkType::Normal;
            }
            MarkType::Handakuten if is_handakuten_possible(c) => {
                fixed.push(char::from_u32(c as u32 + 2).unwrap_or(c));
                mark = MarkType::Normal;
            }
            _ => match c {
                '\u{309b}' | '\u{3099}' | '\u{ff9e}' => mark = MarkType::Dakuten,
                '\u{309c}' | '\u{309a}' | '\u{ff9f}' => mark = MarkType::Handakuten,
                _ => {
                    fixed.push(c);
                    mark = MarkType::Normal;
                }
            },
        }
    }

    fixed.into_iter().rev().collect()
}

fn is_dakuten_possible(c: char) -> bool {
    matches!(
        c,
        'か' | 'き'
            | 'く'
            | 'け'
            | 'こ'
            | 'さ'
            | 'し'
            | 'す'
            | 'せ'
            | 'そ'
            | 'た'
            | 'ち'
            | 'つ'
            | 'て'
            | 'と'
            | 'カ'
            | 'キ'
            | 'ク'
            | 'ケ'
            | 'コ'
            | 'サ'
            | 'シ'
            | 'ス'
            | 'セ'
            | 'ソ'
            | 'タ'
            | 'チ'
            | 'ツ'
            | 'テ'
            | 'ト'
            | 'は'
            | 'ひ'
            | 'ふ'
            | 'へ'
            | 'ほ'
            | 'ハ'
            | 'ヒ'
            | 'フ'
            | 'ヘ'
            | 'ホ'
    )
}

fn is_handakuten_possible(c: char) -> bool {
    matches!(
        c,
        'は' | 'ひ' | 'ふ' | 'へ' | 'ほ' | 'ハ' | 'ヒ' | 'フ' | 'ヘ' | 'ホ'
    )
}

fn is_multi_byte_when_half_width_sanitized(c: char) -> bool {
    matches!(
        c,
        'ガ' | 'ギ'
            | 'グ'
            | 'ゲ'
            | 'ゴ'
            | 'ザ'
            | 'ジ'
            | 'ズ'
            | 'ゼ'
            | 'ゾ'
            | 'ダ'
            | 'ヂ'
            | 'ヅ'
            | 'デ'
            | 'ド'
            | 'バ'
            | 'パ'
            | 'ビ'
            | 'ピ'
            | 'ブ'
            | 'プ'
            | 'ベ'
            | 'ペ'
            | 'ボ'
            | 'ポ'
            | 'ヮ'
            | 'ヰ'
            | 'ヱ'
            | 'ヵ'
            | 'ヶ'
            | 'ヴ'
            | 'ヽ'
            | 'ヾ'
            | 'が'
            | 'ぎ'
            | 'ぐ'
            | 'げ'
            | 'ご'
            | 'ざ'
            | 'じ'
            | 'ず'
            | 'ぜ'
            | 'ぞ'
            | 'だ'
            | 'ぢ'
            | 'づ'
            | 'で'
            | 'ど'
            | 'ば'
            | 'ぱ'
            | 'び'
            | 'ぴ'
            | 'ぶ'
            | 'ぷ'
            | 'べ'
            | 'ぺ'
            | 'ぼ'
            | 'ぽ'
            | 'ゎ'
            | 'ゐ'
            | 'ゑ'
            | 'ゕ'
            | 'ゖ'
            | 'ゔ'
            | 'ゝ'
            | 'ゞ'
    )
}

fn map_to_half_width(c: char) -> Option<String> {
    if c == '\u{3000}' {
        return Some(" ".to_string());
    }
    if ('\u{ff01}'..='\u{ff5e}').contains(&c) {
        return char::from_u32(c as u32 - 0xfee0).map(|ascii| ascii.to_string());
    }

    let mapped = match c {
        '－' | 'ｰ' | 'ー' => "-",
        'ァ' | 'ぁ' => "ｧ",
        'ア' | 'あ' => "ｱ",
        'ィ' | 'ぃ' => "ｨ",
        'イ' | 'い' => "ｲ",
        'ゥ' | 'ぅ' => "ｩ",
        'ウ' | 'う' => "ｳ",
        'ェ' | 'ぇ' => "ｪ",
        'エ' | 'え' => "ｴ",
        'ォ' | 'ぉ' => "ｫ",
        'オ' | 'お' => "ｵ",
        'カ' | 'か' => "ｶ",
        'ガ' | 'が' => "ｶﾞ",
        'キ' | 'き' => "ｷ",
        'ギ' | 'ぎ' => "ｷﾞ",
        'ク' | 'く' => "ｸ",
        'グ' | 'ぐ' => "ｸﾞ",
        'ケ' | 'け' => "ｹ",
        'ゲ' | 'げ' => "ｹﾞ",
        'コ' | 'こ' => "ｺ",
        'ゴ' | 'ご' => "ｺﾞ",
        'サ' | 'さ' => "ｻ",
        'ザ' | 'ざ' => "ｻﾞ",
        'シ' | 'し' => "ｼ",
        'ジ' | 'じ' => "ｼﾞ",
        'ス' | 'す' => "ｽ",
        'ズ' | 'ず' => "ｽﾞ",
        'セ' | 'せ' => "ｾ",
        'ゼ' | 'ぜ' => "ｾﾞ",
        'ソ' | 'そ' => "ｿ",
        'ゾ' | 'ぞ' => "ｿﾞ",
        'タ' | 'た' => "ﾀ",
        'ダ' | 'だ' => "ﾀﾞ",
        'チ' | 'ち' => "ﾁ",
        'ヂ' | 'ぢ' => "ﾁﾞ",
        'ッ' | 'っ' => "ｯ",
        'ツ' | 'つ' => "ﾂ",
        'ヅ' | 'づ' => "ﾂﾞ",
        'テ' | 'て' => "ﾃ",
        'デ' | 'で' => "ﾃﾞ",
        'ト' | 'と' => "ﾄ",
        'ド' | 'ど' => "ﾄﾞ",
        'ナ' | 'な' => "ﾅ",
        'ニ' | 'に' => "ﾆ",
        'ヌ' | 'ぬ' => "ﾇ",
        'ネ' | 'ね' => "ﾈ",
        'ノ' | 'の' => "ﾉ",
        'ハ' | 'は' => "ﾊ",
        'バ' | 'ば' => "ﾊﾞ",
        'パ' | 'ぱ' => "ﾊﾟ",
        'ヒ' | 'ひ' => "ﾋ",
        'ビ' | 'び' => "ﾋﾞ",
        'ピ' | 'ぴ' => "ﾋﾟ",
        'フ' | 'ふ' => "ﾌ",
        'ブ' | 'ぶ' => "ﾌﾞ",
        'プ' | 'ぷ' => "ﾌﾟ",
        'ヘ' | 'へ' => "ﾍ",
        'ベ' | 'べ' => "ﾍﾞ",
        'ペ' | 'ぺ' => "ﾍﾟ",
        'ホ' | 'ほ' => "ﾎ",
        'ボ' | 'ぼ' => "ﾎﾞ",
        'ポ' | 'ぽ' => "ﾎﾟ",
        'マ' | 'ま' => "ﾏ",
        'ミ' | 'み' => "ﾐ",
        'ム' | 'む' => "ﾑ",
        'メ' | 'め' => "ﾒ",
        'モ' | 'も' => "ﾓ",
        'ャ' | 'ゃ' => "ｬ",
        'ヤ' | 'や' => "ﾔ",
        'ュ' | 'ゅ' => "ｭ",
        'ユ' | 'ゆ' => "ﾕ",
        'ョ' | 'ょ' => "ｮ",
        'ヨ' | 'よ' => "ﾖ",
        'ラ' | 'ら' => "ﾗ",
        'リ' | 'り' => "ﾘ",
        'ル' | 'る' => "ﾙ",
        'レ' | 'れ' => "ﾚ",
        'ロ' | 'ろ' => "ﾛ",
        'ワ' | 'わ' => "ﾜ",
        'ヲ' | 'を' => "ｦ",
        'ン' | 'ん' => "ﾝ",
        'ヮ' | 'ゎ' => "ヮ",
        'ヰ' | 'ゐ' => "ヰ",
        'ヱ' | 'ゑ' => "ヱ",
        'ヵ' | 'ゕ' => "ヵ",
        'ヶ' | 'ゖ' => "ヶ",
        'ヴ' | 'ゔ' => "ｳﾞ",
        'ヽ' | 'ゝ' => "ヽ",
        'ヾ' | 'ゞ' => "ヾ",
        '・' => "･",
        '「' => "｢",
        '」' => "｣",
        '。' => "｡",
        '、' => "､",
        _ => return None,
    };
    Some(mapped.to_string())
}

fn is_allowed_half_width_char(c: char) -> bool {
    matches!(
        c,
        '\u{ff61}'..='\u{ff9f}' | 'ヮ' | 'ヰ' | 'ヱ' | 'ヵ' | 'ヶ' | 'ヽ' | 'ヾ'
    )
}

fn map_to_full_width_jp(c: char) -> Option<String> {
    if c == ' ' {
        return Some("　".to_string());
    }
    if ('!'..='~').contains(&c) {
        return char::from_u32(c as u32 + 0xfee0).map(|full| full.to_string());
    }

    let mapped = match c {
        'ｧ' => "ァ",
        'ｱ' => "ア",
        'ｨ' => "ィ",
        'ｲ' => "イ",
        'ｩ' => "ゥ",
        'ｳ' => "ウ",
        'ｪ' => "ェ",
        'ｴ' => "エ",
        'ｫ' => "ォ",
        'ｵ' => "オ",
        'ｶ' => "カ",
        'ｷ' => "キ",
        'ｸ' => "ク",
        'ｹ' => "ケ",
        'ｺ' => "コ",
        'ｻ' => "サ",
        'ｼ' => "シ",
        'ｽ' => "ス",
        'ｾ' => "セ",
        'ｿ' => "ソ",
        'ﾀ' => "タ",
        'ﾁ' => "チ",
        'ｯ' => "ッ",
        'ﾂ' => "ツ",
        'ﾃ' => "テ",
        'ﾄ' => "ト",
        'ﾅ' => "ナ",
        'ﾆ' => "ニ",
        'ﾇ' => "ヌ",
        'ﾈ' => "ネ",
        'ﾉ' => "ノ",
        'ﾊ' => "ハ",
        'ﾋ' => "ヒ",
        'ﾌ' => "フ",
        'ﾍ' => "ヘ",
        'ﾎ' => "ホ",
        'ﾏ' => "マ",
        'ﾐ' => "ミ",
        'ﾑ' => "ム",
        'ﾒ' => "メ",
        'ﾓ' => "モ",
        'ｬ' => "ャ",
        'ﾔ' => "ヤ",
        'ｭ' => "ュ",
        'ﾕ' => "ユ",
        'ｮ' => "ョ",
        'ﾖ' => "ヨ",
        'ﾗ' => "ラ",
        'ﾘ' => "リ",
        'ﾙ' => "ル",
        'ﾚ' => "レ",
        'ﾛ' => "ロ",
        'ﾜ' => "ワ",
        'ｦ' => "ヲ",
        'ﾝ' => "ン",
        'ｰ' => "ー",
        'ヮ' => "ヮ",
        'ヰ' => "ヰ",
        'ヱ' => "ヱ",
        'ヵ' => "ヵ",
        'ヶ' => "ヶ",
        'ヽ' => "ヽ",
        'ヾ' => "ヾ",
        '･' => "・",
        '｢' => "「",
        '｣' => "」",
        '｡' => "。",
        '､' => "、",
        _ => return None,
    };
    Some(mapped.to_string())
}

fn map_russian(s: &str) -> Option<&'static str> {
    match s {
        "а" => Some("a"),
        "б" => Some("b"),
        "в" => Some("v"),
        "г" => Some("g"),
        "д" => Some("d"),
        "е" | "ё" => Some("e"),
        "ж" => Some("zh"),
        "з" => Some("z"),
        "и" | "й" => Some("i"),
        "к" => Some("k"),
        "л" => Some("l"),
        "м" => Some("m"),
        "н" => Some("n"),
        "о" => Some("o"),
        "п" => Some("p"),
        "р" => Some("r"),
        "с" => Some("s"),
        "т" => Some("t"),
        "у" => Some("u"),
        "ф" => Some("f"),
        "х" => Some("kh"),
        "ц" => Some("tc"),
        "ч" => Some("ch"),
        "ш" => Some("sh"),
        "щ" => Some("shch"),
        "ъ" => Some(""),
        "ы" => Some("y"),
        "ь" => Some("'"),
        "э" => Some("e"),
        "ю" => Some("iu"),
        "я" => Some("ia"),
        "А" => Some("A"),
        "Б" => Some("B"),
        "В" => Some("V"),
        "Г" => Some("G"),
        "Д" => Some("D"),
        "Е" | "Ё" => Some("E"),
        "Ж" => Some("Zh"),
        "З" => Some("Z"),
        "И" | "Й" => Some("I"),
        "К" => Some("K"),
        "Л" => Some("L"),
        "М" => Some("M"),
        "Н" => Some("N"),
        "О" => Some("O"),
        "П" => Some("P"),
        "Р" => Some("R"),
        "С" => Some("S"),
        "Т" => Some("T"),
        "У" => Some("U"),
        "Ф" => Some("F"),
        "Х" => Some("Kh"),
        "Ц" => Some("Tc"),
        "Ч" => Some("Ch"),
        "Ш" => Some("Sh"),
        "Щ" => Some("Shch"),
        "Ъ" => Some(""),
        "Ы" => Some("Y"),
        "Ь" => Some("'"),
        "Э" => Some("E"),
        "Ю" => Some("Iu"),
        "Я" => Some("Ia"),
        _ => None,
    }
}

fn map_german(s: &str) -> Option<&'static str> {
    match s {
        "Ä" => Some("Ae"),
        "ä" => Some("ae"),
        "Ö" => Some("Oe"),
        "ö" => Some("oe"),
        "Ü" => Some("Ue"),
        "ü" => Some("ue"),
        "ß" => Some("ss"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_width_range_maps_known_group_chars() {
        assert_eq!(half_width_to_full_width_range("1-3;4/5x"), "１－３；４／５");
    }

    #[test]
    fn half_width_sanitize_maps_full_width_ascii_and_kana() {
        assert_eq!(sanitize_half_width_title("ＡＢＣ　１２３！？"), "ABC 123!?");
        assert_eq!(sanitize_half_width_title("ソニー"), "ｿﾆ-");
        assert_eq!(sanitize_half_width_title("こんにちは"), "ｺﾝﾆﾁﾊ");
    }

    #[test]
    fn half_width_sanitize_flattens_dakuten_marks() {
        assert_eq!(sanitize_half_width_title("カ\u{309b} ハ\u{309c}"), "ｶﾞ ﾊﾟ");
        assert_eq!(sanitize_half_width_title("ｶﾞ ﾊﾟ"), "ｶﾞ ﾊﾟ");
    }

    #[test]
    fn half_width_length_accounts_for_voiced_kana() {
        assert_eq!(get_half_width_title_length("カ"), 1);
        assert_eq!(get_half_width_title_length("ガ"), 2);
        assert_eq!(get_half_width_title_length("がぱ"), 4);
    }

    #[test]
    fn half_width_sanitize_uses_ascii_fallback_for_diacritics() {
        assert_eq!(sanitize_half_width_title("Café"), "Cafe");
        assert_eq!(sanitize_half_width_title("😀"), "");
    }

    #[test]
    fn half_width_sanitize_falls_back_when_safe_length_check_fails() {
        assert_eq!(sanitize_half_width_title("😀A"), "A");
    }

    #[test]
    fn full_width_sanitize_maps_ascii_to_full_width() {
        assert_eq!(sanitize_full_width_title("ABC 123!?"), "ＡＢＣ　１２３！？");
    }

    #[test]
    fn full_width_sanitize_preserves_full_width_kana_and_german_expansions() {
        assert_eq!(sanitize_full_width_title("ソニー"), "ソニー");
        assert_eq!(sanitize_full_width_title("Ä"), "Ae");
        assert_eq!(sanitize_full_width_title("ß"), "ss");
    }

    #[test]
    fn full_width_sanitize_falls_back_when_encoded_length_is_not_full_width() {
        assert_eq!(sanitize_full_width_title("Привет"), "");
        assert_eq!(sanitize_full_width_title("😀"), "");
    }
}
