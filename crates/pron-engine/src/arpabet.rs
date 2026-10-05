//! The ARPAbet symbols used by CMUdict, and the phone classes thresholds use.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Phone classes of ASSESSMENT_SPEC 7.1 step 6. Thresholds are kept per class
/// because one cut-off does not suit a stop and a vowel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneClass {
    Vowel,
    Stop,
    Fricative,
    Affricate,
    Nasal,
    Liquid,
    Glide,
}

impl PhoneClass {
    pub const ALL: [PhoneClass; 7] = [
        PhoneClass::Vowel,
        PhoneClass::Stop,
        PhoneClass::Fricative,
        PhoneClass::Affricate,
        PhoneClass::Nasal,
        PhoneClass::Liquid,
        PhoneClass::Glide,
    ];
}

macro_rules! arpabet {
    ($( $variant:ident => $text:literal, $class:ident; )+) => {
        /// One of the 39 ARPAbet symbols of CMUdict, without a stress digit.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum Arpabet {
            $( #[serde(rename = $text)] $variant, )+
        }

        impl Arpabet {
            /// Every symbol, in a fixed order that tests and reports rely on.
            pub const ALL: [Arpabet; 39] = [ $( Arpabet::$variant, )+ ];

            /// The upper-case symbol as CMUdict writes it.
            pub const fn as_str(self) -> &'static str {
                match self { $( Arpabet::$variant => $text, )+ }
            }

            pub const fn class(self) -> PhoneClass {
                match self { $( Arpabet::$variant => PhoneClass::$class, )+ }
            }
        }

        impl FromStr for Arpabet {
            type Err = UnknownArpabet;

            /// Parses an upper-case symbol with no stress digit.
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( $text => Ok(Arpabet::$variant), )+
                    other => Err(UnknownArpabet(other.to_owned())),
                }
            }
        }
    };
}

arpabet! {
    AA => "AA", Vowel;
    AE => "AE", Vowel;
    AH => "AH", Vowel;
    AO => "AO", Vowel;
    AW => "AW", Vowel;
    AY => "AY", Vowel;
    B => "B", Stop;
    CH => "CH", Affricate;
    D => "D", Stop;
    DH => "DH", Fricative;
    EH => "EH", Vowel;
    ER => "ER", Vowel;
    EY => "EY", Vowel;
    F => "F", Fricative;
    G => "G", Stop;
    HH => "HH", Fricative;
    IH => "IH", Vowel;
    IY => "IY", Vowel;
    JH => "JH", Affricate;
    K => "K", Stop;
    L => "L", Liquid;
    M => "M", Nasal;
    N => "N", Nasal;
    NG => "NG", Nasal;
    OW => "OW", Vowel;
    OY => "OY", Vowel;
    P => "P", Stop;
    R => "R", Liquid;
    S => "S", Fricative;
    SH => "SH", Fricative;
    T => "T", Stop;
    TH => "TH", Fricative;
    UH => "UH", Vowel;
    UW => "UW", Vowel;
    V => "V", Fricative;
    W => "W", Glide;
    Y => "Y", Glide;
    Z => "Z", Fricative;
    ZH => "ZH", Fricative;
}

impl fmt::Display for Arpabet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A symbol that is not one of the 39 ARPAbet symbols.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not an ARPAbet symbol")]
pub struct UnknownArpabet(pub String);

/// An ARPAbet symbol with the lexical stress CMUdict records for vowels.
///
/// Stress is kept because the lexicon is faithful to its source, but nothing in
/// alignment or scoring reads it: stress is not assessed (ASSESSMENT_SPEC 7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Phone {
    pub symbol: Arpabet,
    /// 0, 1 or 2 on vowels as CMUdict writes them, `None` on consonants.
    pub stress: Option<u8>,
}

impl Phone {
    /// Parses `AH0`, `K`, and so on. A stress digit other than 0, 1 or 2, or a
    /// digit on a consonant, is rejected rather than guessed at.
    pub fn parse(token: &str) -> Result<Phone, UnknownArpabet> {
        let (body, stress) = match token.char_indices().last() {
            Some((i, c)) if c.is_ascii_digit() => (&token[..i], Some(c)),
            _ => (token, None),
        };
        let symbol: Arpabet = body.parse()?;
        let stress = match stress {
            None => None,
            Some(d @ '0'..='2') if symbol.class() == PhoneClass::Vowel => Some(d as u8 - b'0'),
            Some(_) => return Err(UnknownArpabet(token.to_owned())),
        };
        Ok(Phone { symbol, stress })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_39_distinct_symbols_that_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for symbol in Arpabet::ALL {
            assert!(seen.insert(symbol.as_str()), "{symbol} listed twice");
            assert_eq!(symbol.as_str().parse::<Arpabet>(), Ok(symbol));
        }
        assert_eq!(seen.len(), 39);
    }

    #[test]
    fn classes_have_the_expected_sizes() {
        let count = |class| Arpabet::ALL.iter().filter(|a| a.class() == class).count();
        assert_eq!(count(PhoneClass::Vowel), 15);
        assert_eq!(count(PhoneClass::Stop), 6);
        assert_eq!(count(PhoneClass::Fricative), 9);
        assert_eq!(count(PhoneClass::Affricate), 2);
        assert_eq!(count(PhoneClass::Nasal), 3);
        assert_eq!(count(PhoneClass::Liquid), 2);
        assert_eq!(count(PhoneClass::Glide), 2);
    }

    #[test]
    fn phone_parse_table() {
        let ok = [
            ("AH0", Arpabet::AH, Some(0)),
            ("IY1", Arpabet::IY, Some(1)),
            ("ER2", Arpabet::ER, Some(2)),
            ("K", Arpabet::K, None),
            ("NG", Arpabet::NG, None),
            ("AA", Arpabet::AA, None),
        ];
        for (text, symbol, stress) in ok {
            assert_eq!(Phone::parse(text), Ok(Phone { symbol, stress }), "{text}");
        }
        for bad in ["", "ah0", "AH3", "K1", "XX", "AH00", "0", "SIL"] {
            assert!(Phone::parse(bad).is_err(), "`{bad}` must be rejected");
        }
    }
}
