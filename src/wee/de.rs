use log::trace;
use serde::de::{self, DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::convert::TryInto;

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Message(String),
    Eof,
    Syntax,
    ExpectedType,
    ExpectedChar,
    ExpectedInteger,
    ExpectedNull,
    ExpectedLong,
    ExpectedArray,
    ExpectedString,
    ExpectedBuf,
    ExpectedPointer,
    ExpectedHdata,
    ExpectedHashtable,
    ExpectedTime,
    ExpectedInfo,
    ExpectedInfolist,
    BadUTF8,
    TrailingCharacters,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Message(msg) => formatter.write_str(msg),
            Error::Eof => formatter.write_str("unexpected end of input"),
            _ => formatter.write_str(&format!("{:?}", self)),
        }
    }
}

impl de::Error for Error {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Error::Message(msg.to_string())
    }
}

impl std::error::Error for Error {}

enum MsgPart<'de> {
    Struct,
    Id,
    Key,
    Data(&'de str),
}

pub struct DeMessage<'de> {
    input: &'de [u8],
    part: MsgPart<'de>,
}

impl<'de> DeMessage<'de> {
    pub fn from_bytes(input: &'de [u8]) -> Self {
        DeMessage {
            input,
            part: MsgPart::Struct,
        }
    }

    /// Read the 3byte data type marker.
    fn read_typ(&mut self) -> Result<&'de str> {
        let typ = &self.input[0..3];
        self.input = &self.input[3..];
        std::str::from_utf8(typ).or_else(|_| Err(Error::ExpectedType))
    }

    /// Read a wee byte array.
    fn read_buf(&mut self) -> Result<Option<&'de [u8]>> {
        let (lenb, tail) = self.input.split_at(4);
        let len = u32::from_be_bytes(lenb.try_into().or_else(|_| Err(Error::ExpectedInteger))?);
        match len {
            0xFFFFFFFF => {
                self.input = tail;
                Ok(None)
            }
            _ => {
                let (data, tail) = tail.split_at(len as usize);
                self.input = tail;
                Ok(Some(data))
            }
        }
    }

    /// Read a wee string.
    fn read_str(&mut self) -> Result<Option<&'de str>> {
        match self.read_buf()? {
            Some(data) => Ok(Some(std::str::from_utf8(data).or(Err(Error::BadUTF8))?)),
            None => Ok(None),
        }
    }

    fn read_ptr(&mut self) -> Result<&'de str> {
        let end = self.input[0] as usize + 1;
        let val = std::str::from_utf8(&self.input[1..end]).or(Err(Error::BadUTF8))?;
        self.input = &self.input[end..];
        Ok(val)
    }
}

/// Deserialize a Weechat relay message, including the id, but excluding
/// the compression flag and compression handling.
pub fn from_bytes<'a, T>(b: &'a [u8]) -> Result<T>
where
    T: Deserialize<'a>,
{
    let mut deserializer = DeMessage::from_bytes(b);
    let t = T::deserialize(&mut deserializer)?;
    if deserializer.input.len() == 0 {
        Ok(t)
    } else {
        Err(Error::TrailingCharacters)
    }
}

impl<'de, 'a> de::Deserializer<'de> for &'a mut DeMessage<'de> {
    type Error = Error;

    /// Deserializes a message, which is an {id: &str, typ: T}
    /// typ is a wee type name (str, htb, hda, etc.) and data.
    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.part {
            MsgPart::Struct => {
                self.part = MsgPart::Id;
                visitor.visit_map(self)
            }
            MsgPart::Id => {
                self.part = MsgPart::Data("str");
                visitor.visit_borrowed_str("id")
            }
            MsgPart::Key => {
                // return type marker as struct key
                let typ = self.read_typ()?;
                self.part = MsgPart::Data(typ);
                visitor.visit_borrowed_str(typ)
            }
            MsgPart::Data(typ) => match typ {
                "str" => match self.read_str()? {
                    Some(val) => visitor.visit_borrowed_str(val),
                    None => visitor.visit_none(),
                },
                "buf" => match self.read_buf()? {
                    Some(val) => visitor.visit_borrowed_bytes(val),
                    None => visitor.visit_none(),
                },
                "inf" => visitor.visit_seq(WeeSeq::new(self, "str", 2)),
                "chr" => {
                    let val = self.input[0];
                    self.input = &self.input[1..];
                    visitor.visit_i8(val as i8)
                }
                "int" => {
                    let (val, tail) = self.input.split_at(4);
                    self.input = tail;
                    visitor.visit_i32(i32::from_be_bytes(val.try_into().unwrap()))
                }
                "lon" | "ptr" | "tim" => {
                    // XXX This may need revisiting, if any of those types are
                    // used apart from just printing them.
                    //
                    // ptr may benefit from a "0x" prefix, if used.
                    // tim is probably better as a number or struct
                    // lon is probably better a number, if used in computation
                    visitor.visit_borrowed_str(self.read_ptr()?)
                }
                "arr" => {
                    let typ = self.read_typ()?;
                    let (lenb, tail) = self.input.split_at(4);
                    let len = u32::from_be_bytes(lenb.try_into().or(Err(Error::ExpectedInteger))?);
                    self.input = tail;
                    visitor.visit_seq(WeeSeq::new(self, typ, len as usize))
                }
                "htb" => {
                    let ktyp = self.read_typ()?;
                    let vtyp = self.read_typ()?;
                    let (lenb, tail) = self.input.split_at(4);
                    let len = u32::from_be_bytes(lenb.try_into().or(Err(Error::ExpectedInteger))?);
                    self.input = tail;

                    visitor.visit_map(WeeMap::new(self, ktyp, vtyp, len as usize))
                }
                "hda" => {
                    let hpath = self.read_str()?.map_or_else(
                        || vec![],
                        |h| h.split("/").map(|h| format!("ptr_{}", h)).collect(),
                    );
                    let header = self.read_str()?;
                    let key_types = header.map_or_else(
                        || vec![],
                        |h| {
                            h.split(',')
                                .map(|v| (&v[..v.len() - 4], &v[v.len() - 3..]))
                                .collect()
                        },
                    );
                    let (lenb, tail) = self.input.split_at(4);
                    let len = u32::from_be_bytes(lenb.try_into().or(Err(Error::ExpectedInteger))?);
                    self.input = tail;

                    visitor.visit_seq(WeeHda::new(self, key_types, len as usize, hpath))
                }
                // XXX Infolist implementation (inl) is intentionally left out.
                // According to the doc, infolist is just a less efficient
                // request method from hda, so I'm leaving this unimplemented
                // until there is a relevant use case for it.
                _ => unimplemented!("decoding of wee type {:?}", typ),
            },
        }
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.part {
            MsgPart::Struct => {
                self.part = MsgPart::Data("str");
                visitor.visit_seq(self)
            }
            _ => self.deserialize_any(visitor),
        }
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        match self.part {
            MsgPart::Data("str") | MsgPart::Data("buf") => {
                if self.input[0..4] == b"\xff\xff\xff\xff"[..] {
                    self.input = &self.input[4..];
                    visitor.visit_none()
                } else {
                    visitor.visit_some(self)
                }
            }
            MsgPart::Data("ptr") => {
                if self.input[0..2] == b"\x010"[..] {
                    self.input = &self.input[2..];
                    visitor.visit_none()
                } else {
                    visitor.visit_some(self)
                }
            }
            _ => unimplemented!(),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct newtype_struct seq tuple
        map struct enum identifier ignored_any
    }
}

/// Implementation for visiting a message as a seq.
impl<'de> de::SeqAccess<'de> for DeMessage<'de> {
    type Error = Error;

    fn next_element_seed<E>(&mut self, seed: E) -> Result<Option<E::Value>>
    where
        E: DeserializeSeed<'de>,
    {
        if self.input.len() == 0 {
            Ok(None)
        } else {
            if let MsgPart::Key = self.part {
                // skip keys/type marker
                let typ = self.read_typ()?;
                self.part = MsgPart::Data(typ);
            }
            let res = seed.deserialize(&mut *self).map(Some);
            self.part = MsgPart::Key;
            res
        }
    }
}

/// Implementation for visiting a message as a map.
impl<'de> de::MapAccess<'de> for DeMessage<'de> {
    type Error = Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>>
    where
        K: DeserializeSeed<'de>,
    {
        if self.input.len() == 0 {
            Ok(None)
        } else {
            seed.deserialize(self).map(Some)
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value>
    where
        V: DeserializeSeed<'de>,
    {
        let res = seed.deserialize(&mut *self);
        self.part = MsgPart::Key;
        res
    }
}

/// A sequence of known types (array, info) and known len.
struct WeeSeq<'a, 'de: 'a> {
    parent_typ: &'de str,
    de: &'a mut DeMessage<'de>,
    count: usize,
}

impl<'a, 'de> WeeSeq<'a, 'de> {
    fn new(de: &'a mut DeMessage<'de>, typ: &'de str, count: usize) -> Self {
        if let MsgPart::Data(parent_typ) = de.part {
            de.part = MsgPart::Data(typ);
            WeeSeq {
                parent_typ,
                de,
                count,
            }
        } else {
            unimplemented!()
        }
    }
}

impl<'de, 'a> SeqAccess<'de> for WeeSeq<'a, 'de> {
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: DeserializeSeed<'de>,
    {
        if self.count == 0 {
            // reset parent
            self.de.part = MsgPart::Data(self.parent_typ);
            Ok(None)
        } else {
            self.count -= 1;
            seed.deserialize(&mut *self.de).map(Some)
        }
    }
}

/// A map of fixed types and len.
struct WeeMap<'a, 'de: 'a> {
    parent_typ: &'de str,
    ktyp: &'de str,
    vtyp: &'de str,
    de: &'a mut DeMessage<'de>,
    count: usize,
}

impl<'a, 'de> WeeMap<'a, 'de> {
    fn new(de: &'a mut DeMessage<'de>, ktyp: &'de str, vtyp: &'de str, count: usize) -> Self {
        if let MsgPart::Data(parent_typ) = de.part {
            WeeMap {
                parent_typ,
                de,
                ktyp,
                vtyp,
                count,
            }
        } else {
            unimplemented!()
        }
    }
}

impl<'de, 'a> MapAccess<'de> for WeeMap<'a, 'de> {
    type Error = Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>>
    where
        K: DeserializeSeed<'de>,
    {
        if self.count == 0 {
            // reset parent
            self.de.part = MsgPart::Data(self.parent_typ);
            Ok(None)
        } else {
            self.count -= 1;
            self.de.part = MsgPart::Data(self.ktyp);
            seed.deserialize(&mut *self.de).map(Some)
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value>
    where
        V: DeserializeSeed<'de>,
    {
        self.de.part = MsgPart::Data(self.vtyp);
        seed.deserialize(&mut *self.de)
    }
}

/// Accessor for hdata, which are sequences of structs.
struct WeeHda<'a, 'de: 'a> {
    parent_typ: &'de str,
    key_types: Vec<(&'de str, &'de str)>,
    de: &'a mut DeMessage<'de>,
    // number of structs left to unpack
    count: usize,
    // volatile index of fields
    kv_idx: usize,
    // volatile index of p-path
    ptr_idx: usize,
    hpath: Vec<String>,
}

impl<'a, 'de> WeeHda<'a, 'de> {
    fn new(
        de: &'a mut DeMessage<'de>,
        key_types: Vec<(&'de str, &'de str)>,
        count: usize,
        hpath: Vec<String>,
    ) -> Self {
        if let MsgPart::Data(parent_typ) = de.part {
            WeeHda {
                parent_typ,
                de,
                key_types,
                count,
                kv_idx: 0,
                ptr_idx: 0,
                hpath,
            }
        } else {
            unimplemented!()
        }
    }
}

impl<'de, 'a> MapAccess<'de> for WeeHda<'a, 'de> {
    type Error = Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>>
    where
        K: DeserializeSeed<'de>,
    {
        if self.kv_idx == self.key_types.len() {
            Ok(None)
        } else if self.ptr_idx < self.hpath.len() {
            seed.deserialize(self.hpath[self.ptr_idx].clone().into_deserializer())
                .map(Some)
        } else {
            let typ = self.key_types[self.kv_idx].1;
            self.de.part = MsgPart::Data(typ);
            let res = seed.deserialize(&mut *self).map(Some);
            self.kv_idx += 1;
            res
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value>
    where
        V: DeserializeSeed<'de>,
    {
        if self.ptr_idx < self.hpath.len() {
            self.de.part = MsgPart::Data("ptr");
            self.ptr_idx += 1;
            seed.deserialize(&mut *self.de)
        } else {
            seed.deserialize(&mut *self.de).or_else(|e| {
                trace!("while unpacking hda {} ", self.key_types[self.kv_idx - 1].0);
                Err(e)
            })
        }
    }
}

impl<'de, 'a> SeqAccess<'de> for WeeHda<'a, 'de> {
    type Error = Error;

    fn next_element_seed<V>(&mut self, seed: V) -> Result<Option<V::Value>>
    where
        V: DeserializeSeed<'de>,
    {
        if self.count == 0 {
            // reset parent
            self.de.part = MsgPart::Data(self.parent_typ);
            Ok(None)
        } else {
            self.ptr_idx = 0;
            self.count -= 1;
            self.kv_idx = 0;
            seed.deserialize(self).map(Some)
        }
    }
}

/// Deserializes the map and its keys out of the HDA sequence.
impl<'a, 'de: 'a> de::Deserializer<'de> for &mut WeeHda<'a, 'de> {
    type Error = Error;

    fn deserialize_any<V>(self, _visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        unimplemented!()
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.key_types[self.kv_idx].0)
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.key_types[self.kv_idx].0)
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_string(String::from(self.key_types[self.kv_idx].0))
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_map(self)
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value>
    where
        V: Visitor<'de>,
    {
        visitor.visit_map(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char
        bytes byte_buf unit unit_struct newtype_struct seq tuple
        enum ignored_any option tuple_struct
    }
}

pub fn peek_str<'a>(buf: &'a [u8]) -> Result<Option<&'a str>> {
    let (lenb, tail) = buf.split_at(4);
    let len = u32::from_be_bytes(lenb.try_into().or(Err(Error::ExpectedInteger))?);
    match len {
        0xFFFFFFFF => Ok(None),
        _ => {
            let (data, _) = tail.split_at(len as usize);
            Ok(Some(std::str::from_utf8(data).or(Err(Error::BadUTF8))?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::iter::FromIterator;

    #[test]
    fn test_deserialize_version() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Msg<'a> {
            id: &'a str,
            inf: (&'a str, &'a str),
        };
        let encoded = b"\x00\x00\x00\rversion_checkinf\x00\x00\x00\x07version\x00\x00\x00\x032.9";
        assert_eq!(Some("version_check"), peek_str(encoded).unwrap());

        let expected = Msg {
            id: "version_check",
            inf: ("version", "2.9"),
        };
        assert_eq!(expected, from_bytes(encoded).unwrap());
    }

    // Basic types can be deserialized
    #[test]
    fn test_deserialize_test() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Msg<'a>(
            &'a str,          // id
            i8,               // chr
            i32,              // int
            i32,              // int
            &'a str,          // lon
            &'a str,          // lon
            Option<&'a str>,  // str
            Option<&'a str>,  // str
            Option<&'a str>,  // str
            Option<&'a [u8]>, // buf
            Option<&'a [u8]>, // buf
            Option<&'a str>,  // ptr
            Option<&'a str>,  // ptr
            &'a str,          // tim
            (String, String), // arr
            (i32, i32, i32),  // arr
        );
        let encoded = b"\x00\x00\x00\x08test_msgchrAint\x00\x01\xe2@int\xff\xfe\x1d\xc0\
            lon\n1234567890lon\x0b-1234567890str\x00\x00\x00\x08a stringstr\x00\x00\x00\
            \x00str\xff\xff\xff\xffbuf\x00\x00\x00\x06bufferbuf\xff\xff\xff\xffptr\x081\
            234abcdptr\x010tim\n1321993456arrstr\x00\x00\x00\x02\x00\x00\x00\x03abc\x00\
            \x00\x00\x02dearrint\x00\x00\x00\x03\x00\x00\x00{\x00\x00\x01\xc8\x00\x00\
            \x03\x15";
        let expected = Msg(
            "test_msg",
            'A' as i8,
            123456,
            -123456,
            "1234567890",
            "-1234567890",
            Some("a string"),
            Some(""),
            None,
            Some(b"buffer"),
            None,
            Some("1234abcd"),
            None,
            "1321993456",
            (String::from("abc"), String::from("de")),
            (123, 456, 789),
        );
        assert_eq!(expected, from_bytes(encoded).unwrap());
    }

    // Hashtable can be deserialized
    #[test]
    fn test_deserialize_handshake() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Msg<'a> {
            id: &'a str,
            htb: HashMap<&'a str, &'a str>,
        };
        let encoded = b"\x00\x00\x00\thandshakehtbstrstr\x00\x00\x00\x05\x00\
            \x00\x00\x04totp\x00\x00\x00\x03off\x00\x00\x00\x12password_hash\
            _algo\x00\x00\x00\x05plain\x00\x00\x00\x05nonce\x00\x00\x00 6357\
            5E447831AC0055D72561270EBE54\x00\x00\x00\x18password_hash_iterat\
            ions\x00\x00\x00\x06100000\x00\x00\x00\x0bcompression\x00\x00\x00\
            \x03off";
        let expected = Msg {
            id: "handshake",
            htb: vec![
                ("password_hash_iterations", "100000"),
                ("totp", "off"),
                ("nonce", "63575E447831AC0055D72561270EBE54"),
                ("compression", "off"),
                ("password_hash_algo", "plain"),
            ]
            .into_iter()
            .collect(),
        };

        assert_eq!(expected, from_bytes(encoded).unwrap());
    }

    #[test]
    fn test_deserialize_hda() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Hda<'a> {
            ptr_bufs: &'a str,
            number: i32,
            full_name: &'a str,
        }

        #[derive(Deserialize, Debug, PartialEq)]
        struct Msg<'a> {
            id: &'a str,
            hda: Vec<Hda<'a>>,
        };
        let encoded = b"\0\0\0\x07buffershda\0\0\0\x04bufs\0\0\0\x18number:int\
            ,full_name:str\0\0\0\x02\x040123\0\0\0\x01\0\0\0\x0ccore.weechat\
            \x03567\0\0\0\x02\0\0\0\x06potato";
        let expected = Msg {
            id: "buffers",
            hda: vec![
                Hda {
                    ptr_bufs: "0123",
                    number: 1,
                    full_name: "core.weechat",
                },
                Hda {
                    ptr_bufs: "567",
                    number: 2,
                    full_name: "potato",
                },
            ],
        };

        assert_eq!(expected, from_bytes(encoded).unwrap());
    }

    /// Mostly useless test to verify decoding to map.
    /// It is only there to exercise the deserialization paths, as there
    /// are realistically no messages where values are of a single type.
    #[test]
    fn test_deserialize_hda_to_map() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Msg<'a> {
            id: &'a str,
            // Map identifiers are owned strings because the pointer part
            // is formatted in to remove collisions.
            hda: Vec<HashMap<String, &'a str>>,
        };
        let encoded = b"\0\0\0\x07buffershda\0\0\0\x04bufs\0\0\0\x18number:lon\
            ,full_name:str\0\0\0\x02\x040123\x011\0\0\0\x0ccore.weechat\
            \x03567\x012\0\0\0\x06potato";
        let expected = Msg {
            id: "buffers",
            hda: vec![
                HashMap::from_iter(vec![
                    (String::from("ptr_bufs"), "0123"),
                    (String::from("number"), "1"),
                    (String::from("full_name"), "core.weechat"),
                ]),
                HashMap::from_iter(vec![
                    (String::from("ptr_bufs"), "567"),
                    (String::from("number"), "2"),
                    (String::from("full_name"), "potato"),
                ]),
            ],
        };

        assert_eq!(expected, from_bytes(encoded).unwrap());
    }

    #[test]
    fn test_deserialize_empty_hda() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Msg<'a> {
            id: &'a str,
            hda: Vec<HashMap<&'a str, &'a str>>,
        };
        let encoded = b"\0\0\0\x07buffershda\xFF\xFF\xFF\xFF\xFF\xFF\xFF\xFF\
            \0\0\0\0";
        let expected = Msg {
            id: "buffers",
            hda: vec![],
        };

        assert_eq!(expected, from_bytes(encoded).unwrap());
    }

    #[test]
    fn test_deserialize_skipped() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Hda {
            number: i32,
            // skip full_name
        }

        #[derive(Deserialize, Debug, PartialEq)]
        struct Msg<'a> {
            id: &'a str,
            hda: Vec<Hda>,
        };
        let encoded = b"\0\0\0\x07buffershda\0\0\0\x04bufs\0\0\0\x18number:int\
            ,full_name:str\0\0\0\x02\x040123\0\0\0\x01\0\0\0\x0ccore.weechat\
            \x03567\0\0\0\x02\0\0\0\x06potato";
        let expected = Msg {
            id: "buffers",
            hda: vec![Hda { number: 1 }, Hda { number: 2 }],
        };

        assert_eq!(expected, from_bytes(encoded).unwrap());
    }
}
