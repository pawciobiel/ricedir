use iced::Color;
use serde::{Deserialize, Deserializer, de};

/// A colour written as `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba(Color);

impl Rgba {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self(Color::from_rgb(
            red as f32 / 255.0,
            green as f32 / 255.0,
            blue as f32 / 255.0,
        ))
    }

    pub const fn color(self) -> Color {
        self.0
    }
}

impl From<Rgba> for Color {
    fn from(value: Rgba) -> Self {
        value.0
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse(&text).map(Rgba).map_err(de::Error::custom)
    }
}

fn parse(text: &str) -> Result<Color, String> {
    let digits = text
        .strip_prefix('#')
        .ok_or_else(|| format!("`{text}` must start with `#`"))?;

    // Double every digit of the shorthand forms, so both lengths reach the loop
    // below as two digits per channel: `#f0a` becomes `ff00aa`.
    let expanded = match digits.len() {
        3 | 4 => digits.chars().flat_map(|digit| [digit, digit]).collect(),
        6 | 8 => digits.to_owned(),
        length => {
            return Err(format!(
                "`{text}` needs 3, 4, 6 or 8 hex digits, but has {length}"
            ));
        }
    };

    // Alpha stays opaque when the string omits it.
    let mut channels = [u8::MAX; 4];

    for (channel, pair) in channels.iter_mut().zip(expanded.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).map_err(|_| format!("`{text}` is not valid hex"))?;
        *channel = u8::from_str_radix(pair, 16)
            .map_err(|_| format!("`{text}` contains a digit that is not hex"))?;
    }

    let [red, green, blue, alpha] = channels;
    Ok(Color::from_rgba8(
        red,
        green,
        blue,
        f32::from(alpha) / 255.0,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_six_digits() {
        assert_eq!(parse("#1e1e2e"), Ok(Color::from_rgb8(0x1e, 0x1e, 0x2e)));
    }

    #[test]
    fn expands_shorthand() {
        assert_eq!(parse("#f0a"), parse("#ff00aa"));
    }

    #[test]
    fn defaults_to_opaque() {
        assert_eq!(parse("#1e1e2e").map(|color| color.a), Ok(1.0));
    }

    #[test]
    fn reads_alpha() {
        assert_eq!(parse("#1e1e2e00").map(|color| color.a), Ok(0.0));
        assert_eq!(parse("#1e1e2eff").map(|color| color.a), Ok(1.0));
    }

    #[test]
    fn shorthand_alpha_matches_longhand() {
        assert_eq!(parse("#f0a8"), parse("#ff00aa88"));
    }

    #[test]
    fn is_case_insensitive() {
        assert_eq!(parse("#ABCDEF"), parse("#abcdef"));
    }

    #[test]
    fn rejects_missing_hash() {
        assert!(parse("1e1e2e").is_err());
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(parse("#12345").is_err());
        assert!(parse("#").is_err());
    }

    #[test]
    fn rejects_non_hex() {
        assert!(parse("#gggggg").is_err());
    }
}
