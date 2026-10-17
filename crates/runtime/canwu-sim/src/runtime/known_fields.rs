//! The format 9 wire check of decoded documents, without a second document
//! tree.
//!
//! Decoding a wire document into its typed form drops any field the types do
//! not declare, and the wire contract refuses such a field: every object of
//! the input must have only keys that the typed value's own JSON encoding has
//! at the same place, and every array the encoding's length, wherever the
//! input and the encoding have the same kind of node (see
//! `reject_unknown_current_fields`). Building that encoding with
//! `serde_json::to_value` holds a second tree as large as the document. The
//! serializer here walks the typed value's encoding against the input
//! instead, allocating nothing per node, and answers whether the comparison
//! can fail. Only then does the caller build the encoding and compare, so a
//! refusal and its message are exactly the full comparison's.
//!
//! The walk is sound, that is, it never passes an input the full comparison
//! refuses, under three conditions on the wire types and the locked
//! `serde_json` (1.0.151):
//!
//! - Every `Serialize` implementation of the wire types propagates the
//!   errors its serializer returns. The walk reports a possible failure as an
//!   error; an implementation that swallowed one, for example from
//!   `serialize_entry`, would leave that object's matched keys on the shared
//!   stack and let [`Fields::finish`] of an enclosing object count them.
//! - [`KeyLookup`] writes a map key as `serde_json::value`'s map-key
//!   serializer does: strings, characters, `true` and `false`, decimal
//!   integers, unit variants by name, and newtype structs through their
//!   content; every other key kind, floats included, answers that the
//!   comparison may fail. A `serde_json` upgrade must recheck this list.
//! - `serde_json`'s `raw_value` feature is off, so no `RawValue` reaches the
//!   encoding as a token that `to_value` would treat specially.

use serde::Serialize;
use serde::ser::{self, Impossible};
use serde_json::{Map, Value};
use std::fmt::{self, Display, Write as _};

/// Whether the full comparison of `input` with `serde_json::to_value(value)`
/// passes. `true` means it does: the encoding succeeds, every object of
/// `input` has only keys of the encoding, and every array has the
/// encoding's length, wherever both have the same kind of node. `false`
/// means it may not, including when the walk cannot tell (a float map key,
/// an error from a `Serialize` implementation); the caller then runs the
/// full comparison. `true` is exact only under the three conditions of the
/// module documentation: `Serialize` implementations that propagate errors,
/// [`KeyLookup`] matching `serde_json`'s map-key serializer, and no
/// `RawValue`.
pub(super) fn input_fields_are_known<T: Serialize + ?Sized>(value: &T, input: &Value) -> bool {
    let mut matched = Vec::new();
    value
        .serialize(Walk {
            input: Some(input),
            matched: &mut matched,
        })
        .is_ok()
}

/// The walk found a place where the comparison may fail.
#[derive(Debug)]
struct MayFail;

impl Display for MayFail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the wire comparison may fail")
    }
}

impl std::error::Error for MayFail {}

impl ser::Error for MayFail {
    fn custom<T: Display>(_message: T) -> Self {
        Self
    }
}

type Outcome = Result<(), MayFail>;

/// The address of an input object's key, which identifies the key while the
/// input is borrowed.
fn key_address(key: &String) -> usize {
    std::ptr::from_ref(key).addr()
}

/// One node of the encoding and the input node at the same place, if the
/// input has one. `matched` is a stack of the input keys the encoding has
/// named in each object being walked.
struct Walk<'a, 'k> {
    input: Option<&'a Value>,
    matched: &'k mut Vec<usize>,
}

impl<'a, 'k> Walk<'a, 'k> {
    fn object(&self) -> Option<&'a Map<String, Value>> {
        self.input.and_then(Value::as_object)
    }

    fn elements(self) -> Elements<'a, 'k> {
        Elements {
            input: self.input.and_then(Value::as_array).map(Vec::as_slice),
            index: 0,
            matched: self.matched,
        }
    }

    fn fields(self) -> Fields<'a, 'k> {
        let start = self.matched.len();
        Fields {
            object: self.object(),
            start,
            pending: None,
            matched: self.matched,
        }
    }

    /// An encoding `{variant: inner}`: an input object may name only
    /// `variant`. Returns the walk of the inner node.
    fn variant(self, variant: &str) -> Result<Walk<'a, 'k>, MayFail> {
        let input = match self.object() {
            None => None,
            Some(object) => {
                let inner = object.get(variant);
                if object.len() != usize::from(inner.is_some()) {
                    return Err(MayFail);
                }
                inner
            }
        };
        Ok(Walk {
            input,
            matched: self.matched,
        })
    }
}

impl<'a, 'k> ser::Serializer for Walk<'a, 'k> {
    type Ok = ();
    type Error = MayFail;
    type SerializeSeq = Elements<'a, 'k>;
    type SerializeTuple = Elements<'a, 'k>;
    type SerializeTupleStruct = Elements<'a, 'k>;
    type SerializeTupleVariant = Elements<'a, 'k>;
    type SerializeMap = Fields<'a, 'k>;
    type SerializeStruct = Fields<'a, 'k>;
    type SerializeStructVariant = Fields<'a, 'k>;

    fn serialize_bool(self, _value: bool) -> Outcome {
        Ok(())
    }

    fn serialize_i8(self, _value: i8) -> Outcome {
        Ok(())
    }

    fn serialize_i16(self, _value: i16) -> Outcome {
        Ok(())
    }

    fn serialize_i32(self, _value: i32) -> Outcome {
        Ok(())
    }

    fn serialize_i64(self, _value: i64) -> Outcome {
        Ok(())
    }

    /// `serde_json::to_value` refuses what neither `u64` nor `i64` holds.
    fn serialize_i128(self, value: i128) -> Outcome {
        if u64::try_from(value).is_ok() || i64::try_from(value).is_ok() {
            Ok(())
        } else {
            Err(MayFail)
        }
    }

    fn serialize_u8(self, _value: u8) -> Outcome {
        Ok(())
    }

    fn serialize_u16(self, _value: u16) -> Outcome {
        Ok(())
    }

    fn serialize_u32(self, _value: u32) -> Outcome {
        Ok(())
    }

    fn serialize_u64(self, _value: u64) -> Outcome {
        Ok(())
    }

    /// `serde_json::to_value` refuses what `u64` does not hold.
    fn serialize_u128(self, value: u128) -> Outcome {
        u64::try_from(value).map(drop).map_err(|_| MayFail)
    }

    fn serialize_f32(self, _value: f32) -> Outcome {
        Ok(())
    }

    fn serialize_f64(self, _value: f64) -> Outcome {
        Ok(())
    }

    fn serialize_char(self, _value: char) -> Outcome {
        Ok(())
    }

    fn serialize_str(self, _value: &str) -> Outcome {
        Ok(())
    }

    /// Encoded as an array of numbers.
    fn serialize_bytes(self, value: &[u8]) -> Outcome {
        match self.input.and_then(Value::as_array) {
            Some(items) if items.len() != value.len() => Err(MayFail),
            _ => Ok(()),
        }
    }

    fn serialize_none(self) -> Outcome {
        Ok(())
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Outcome {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Outcome {
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Outcome {
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
    ) -> Outcome {
        Ok(())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Outcome {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Outcome {
        value.serialize(self.variant(variant)?)
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Elements<'a, 'k>, MayFail> {
        Ok(self.elements())
    }

    fn serialize_tuple(self, _len: usize) -> Result<Elements<'a, 'k>, MayFail> {
        Ok(self.elements())
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Elements<'a, 'k>, MayFail> {
        Ok(self.elements())
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Elements<'a, 'k>, MayFail> {
        Ok(self.variant(variant)?.elements())
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Fields<'a, 'k>, MayFail> {
        Ok(self.fields())
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Fields<'a, 'k>, MayFail> {
        Ok(self.fields())
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Fields<'a, 'k>, MayFail> {
        Ok(self.variant(variant)?.fields())
    }

    /// Encoded as a string, which nothing compares.
    fn collect_str<T: Display + ?Sized>(self, _value: &T) -> Outcome {
        Ok(())
    }
}

/// The elements of an encoded array and the input array at the same place,
/// if the input has one there.
struct Elements<'a, 'k> {
    input: Option<&'a [Value]>,
    index: usize,
    matched: &'k mut Vec<usize>,
}

impl Elements<'_, '_> {
    fn element<T: Serialize + ?Sized>(&mut self, value: &T) -> Outcome {
        let input = self.input.and_then(|items| items.get(self.index));
        self.index += 1;
        value.serialize(Walk {
            input,
            matched: &mut *self.matched,
        })
    }

    fn finish(self) -> Outcome {
        match self.input {
            Some(items) if items.len() != self.index => Err(MayFail),
            _ => Ok(()),
        }
    }
}

impl ser::SerializeSeq for Elements<'_, '_> {
    type Ok = ();
    type Error = MayFail;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Outcome {
        self.element(value)
    }

    fn end(self) -> Outcome {
        self.finish()
    }
}

impl ser::SerializeTuple for Elements<'_, '_> {
    type Ok = ();
    type Error = MayFail;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Outcome {
        self.element(value)
    }

    fn end(self) -> Outcome {
        self.finish()
    }
}

impl ser::SerializeTupleStruct for Elements<'_, '_> {
    type Ok = ();
    type Error = MayFail;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Outcome {
        self.element(value)
    }

    fn end(self) -> Outcome {
        self.finish()
    }
}

impl ser::SerializeTupleVariant for Elements<'_, '_> {
    type Ok = ();
    type Error = MayFail;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Outcome {
        self.element(value)
    }

    fn end(self) -> Outcome {
        self.finish()
    }
}

/// The entries of an encoded object and the input object at the same place,
/// if the input has one there. The input keys the encoding names are pushed
/// on `matched` from `start`.
struct Fields<'a, 'k> {
    object: Option<&'a Map<String, Value>>,
    start: usize,
    /// The input value under the map key last serialized, if any.
    pending: Option<&'a Value>,
    matched: &'k mut Vec<usize>,
}

impl<'a> Fields<'a, '_> {
    /// Records that the encoding names `key` here and returns the input value
    /// under it.
    fn named(&mut self, key: &str) -> Option<&'a Value> {
        let (stored, value) = self.object?.get_key_value(key)?;
        self.matched.push(key_address(stored));
        Some(value)
    }

    fn entry<T: Serialize + ?Sized>(&mut self, input: Option<&'a Value>, value: &T) -> Outcome {
        value.serialize(Walk {
            input,
            matched: &mut *self.matched,
        })
    }

    /// Every input key must be one the encoding named. The encoding may name
    /// a key twice, so the named keys are counted once each.
    fn finish(self) -> Outcome {
        let Some(object) = self.object else {
            return Ok(());
        };
        let named = &mut self.matched[self.start..];
        let distinct = if named.is_empty() || named.len() < object.len() {
            named.len()
        } else {
            named.sort_unstable();
            1 + named.windows(2).filter(|pair| pair[0] != pair[1]).count()
        };
        self.matched.truncate(self.start);
        if distinct == object.len() {
            Ok(())
        } else {
            Err(MayFail)
        }
    }
}

impl ser::SerializeMap for Fields<'_, '_> {
    type Ok = ();
    type Error = MayFail;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Outcome {
        let named = key.serialize(KeyLookup {
            object: self.object,
        })?;
        self.pending = named.map(|(stored, value)| {
            self.matched.push(key_address(stored));
            value
        });
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Outcome {
        let input = self.pending.take();
        self.entry(input, value)
    }

    fn end(self) -> Outcome {
        self.finish()
    }
}

impl ser::SerializeStruct for Fields<'_, '_> {
    type Ok = ();
    type Error = MayFail;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Outcome {
        let input = self.named(key);
        self.entry(input, value)
    }

    fn end(self) -> Outcome {
        self.finish()
    }
}

impl ser::SerializeStructVariant for Fields<'_, '_> {
    type Ok = ();
    type Error = MayFail;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Outcome {
        let input = self.named(key);
        self.entry(input, value)
    }

    fn end(self) -> Outcome {
        self.finish()
    }
}

/// A map key, encoded as `serde_json::to_value` encodes it, looked up in the
/// input object. Key kinds the encoding refuses, and float keys (whose text
/// is not reproduced here), answer [`MayFail`].
struct KeyLookup<'a> {
    object: Option<&'a Map<String, Value>>,
}

type Named<'a> = Option<(&'a String, &'a Value)>;

impl<'a> KeyLookup<'a> {
    // `Result` so that every key kind answers in the serializer's type.
    #[allow(clippy::unnecessary_wraps)]
    fn find(&self, key: &str) -> Result<Named<'a>, MayFail> {
        Ok(self.object.and_then(|object| object.get_key_value(key)))
    }

    fn integer(&self, value: impl Display) -> Result<Named<'a>, MayFail> {
        let mut text = KeyText::default();
        write!(text, "{value}").map_err(|_| MayFail)?;
        self.find(text.as_str())
    }
}

/// The decimal text of an integer key, on the stack.
struct KeyText {
    bytes: [u8; 40],
    len: usize,
}

impl Default for KeyText {
    fn default() -> Self {
        Self {
            bytes: [0; 40],
            len: 0,
        }
    }
}

impl KeyText {
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or_default()
    }
}

impl fmt::Write for KeyText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len.checked_add(text.len()).ok_or(fmt::Error)?;
        self.bytes
            .get_mut(self.len..end)
            .ok_or(fmt::Error)?
            .copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

impl<'a> ser::Serializer for KeyLookup<'a> {
    type Ok = Named<'a>;
    type Error = MayFail;
    type SerializeSeq = Impossible<Named<'a>, MayFail>;
    type SerializeTuple = Impossible<Named<'a>, MayFail>;
    type SerializeTupleStruct = Impossible<Named<'a>, MayFail>;
    type SerializeTupleVariant = Impossible<Named<'a>, MayFail>;
    type SerializeMap = Impossible<Named<'a>, MayFail>;
    type SerializeStruct = Impossible<Named<'a>, MayFail>;
    type SerializeStructVariant = Impossible<Named<'a>, MayFail>;

    fn serialize_bool(self, value: bool) -> Result<Named<'a>, MayFail> {
        self.find(if value { "true" } else { "false" })
    }

    fn serialize_i8(self, value: i8) -> Result<Named<'a>, MayFail> {
        self.integer(value)
    }

    fn serialize_i16(self, value: i16) -> Result<Named<'a>, MayFail> {
        self.integer(value)
    }

    fn serialize_i32(self, value: i32) -> Result<Named<'a>, MayFail> {
        self.integer(value)
    }

    fn serialize_i64(self, value: i64) -> Result<Named<'a>, MayFail> {
        self.integer(value)
    }

    fn serialize_i128(self, value: i128) -> Result<Named<'a>, MayFail> {
        self.integer(value)
    }

    fn serialize_u8(self, value: u8) -> Result<Named<'a>, MayFail> {
        self.integer(value)
    }

    fn serialize_u16(self, value: u16) -> Result<Named<'a>, MayFail> {
        self.integer(value)
    }

    fn serialize_u32(self, value: u32) -> Result<Named<'a>, MayFail> {
        self.integer(value)
    }

    fn serialize_u64(self, value: u64) -> Result<Named<'a>, MayFail> {
        self.integer(value)
    }

    fn serialize_u128(self, value: u128) -> Result<Named<'a>, MayFail> {
        self.integer(value)
    }

    fn serialize_f32(self, _value: f32) -> Result<Named<'a>, MayFail> {
        Err(MayFail)
    }

    fn serialize_f64(self, _value: f64) -> Result<Named<'a>, MayFail> {
        Err(MayFail)
    }

    fn serialize_char(self, value: char) -> Result<Named<'a>, MayFail> {
        let mut buffer = [0; 4];
        self.find(value.encode_utf8(&mut buffer))
    }

    fn serialize_str(self, value: &str) -> Result<Named<'a>, MayFail> {
        self.find(value)
    }

    fn serialize_bytes(self, _value: &[u8]) -> Result<Named<'a>, MayFail> {
        Err(MayFail)
    }

    fn serialize_none(self) -> Result<Named<'a>, MayFail> {
        Err(MayFail)
    }

    fn serialize_some<T: Serialize + ?Sized>(self, _value: &T) -> Result<Named<'a>, MayFail> {
        Err(MayFail)
    }

    fn serialize_unit(self) -> Result<Named<'a>, MayFail> {
        Err(MayFail)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Named<'a>, MayFail> {
        Err(MayFail)
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<Named<'a>, MayFail> {
        self.find(variant)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Named<'a>, MayFail> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Named<'a>, MayFail> {
        Err(MayFail)
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, MayFail> {
        Err(MayFail)
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, MayFail> {
        Err(MayFail)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, MayFail> {
        Err(MayFail)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, MayFail> {
        Err(MayFail)
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, MayFail> {
        Err(MayFail)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, MayFail> {
        Err(MayFail)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, MayFail> {
        Err(MayFail)
    }

    fn collect_str<T: Display + ?Sized>(self, value: &T) -> Result<Named<'a>, MayFail> {
        // Rare: a key that serializes through `Display`. Its text is
        // compared as the encoding would write it.
        let text = value.to_string();
        self.find(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::input_fields_are_known;
    use serde::Serialize;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    #[derive(Serialize)]
    struct Inner {
        a: u64,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        list: Vec<u8>,
    }

    #[derive(Serialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Tagged {
        One { inner: Inner },
        Two(Inner),
    }

    #[derive(Serialize)]
    enum External {
        Unit,
        Newtype(Inner),
        Tuple(u8, u8),
        Struct { b: u8 },
    }

    #[derive(Serialize)]
    struct Outer {
        inner: Inner,
        tagged: Vec<Tagged>,
        external: Vec<External>,
        by_id: BTreeMap<u64, Inner>,
        payload: Value,
        missing: Option<u8>,
    }

    fn outer() -> Outer {
        Outer {
            inner: Inner {
                a: 1,
                list: vec![1, 2],
            },
            tagged: vec![
                Tagged::One {
                    inner: Inner {
                        a: 2,
                        list: Vec::new(),
                    },
                },
                Tagged::Two(Inner {
                    a: 3,
                    list: vec![4],
                }),
            ],
            external: vec![
                External::Unit,
                External::Newtype(Inner {
                    a: 4,
                    list: Vec::new(),
                }),
                External::Tuple(1, 2),
                External::Struct { b: 5 },
            ],
            by_id: BTreeMap::from([(
                7,
                Inner {
                    a: 5,
                    list: Vec::new(),
                },
            )]),
            payload: json!({ "k": [1, { "z": null }] }),
            missing: None,
        }
    }

    /// Checks the walk against the full comparison on `value`'s own
    /// encoding and on every input made from it by one change at one node:
    /// an unknown field added to an object, a field removed, an array
    /// lengthened, or a node replaced by one of another kind. Returns the
    /// number of inputs checked.
    fn sweep<T: Serialize>(value: &T) -> usize {
        let encoded = serde_json::to_value(value).unwrap();
        assert!(input_fields_are_known(value, &encoded));
        let mut checked = 0;
        visit_mutations(&encoded, &encoded, &mut Vec::new(), &mut |input| {
            let full = super::super::reject_unknown_current_fields(input, &encoded, "").is_ok();
            assert_eq!(input_fields_are_known(value, input), full, "{input}");
            checked += 1;
        });
        checked
    }

    /// The walk agrees with the full comparison on a value with every kind
    /// of node the wire types use.
    #[test]
    fn walk_agrees_with_the_full_comparison() {
        assert!(sweep(&outer()) > 40);
    }

    /// The same on a real snapshot: a run that ordered a movement, advanced
    /// past the arrival, and settled a boundary.
    #[test]
    fn walk_agrees_with_the_full_comparison_on_a_snapshot() {
        let (scenario, ids) = crate::demo_scenario();
        let mut simulation = crate::Simulation::new(35, scenario).unwrap();
        simulation
            .submit(crate::CommandEnvelope::new(
                crate::Issuer::Actor(ids.commander),
                crate::Command::OrderMovement {
                    subject: canwu_core::EntityRef::Army(ids.army),
                    destination: ids.eastern_territory,
                    cargo: Vec::new(),
                },
            ))
            .unwrap();
        simulation
            .advance(canwu_time::SimDuration::days(4))
            .unwrap();
        simulation
            .settle_boundary(crate::BoundaryRequest::at(simulation.time()))
            .unwrap();
        assert!(sweep(&simulation.snapshot()) > 1_000);
    }

    /// Calls `check` with every input made from `root` by one change at the
    /// node `value`, at `path`, or below it.
    fn visit_mutations(
        root: &Value,
        value: &Value,
        path: &mut Vec<PathStep>,
        check: &mut dyn FnMut(&Value),
    ) {
        let mut changed = |change: &dyn Fn(&mut Value)| {
            let mut input = root.clone();
            with_node(&mut input, path, change);
            check(&input);
        };
        match value {
            Value::Object(object) => {
                changed(&|node| {
                    node.as_object_mut()
                        .unwrap()
                        .insert("unknown".to_owned(), json!(1));
                });
                for key in object.keys() {
                    changed(&|node| {
                        node.as_object_mut().unwrap().remove(key);
                    });
                }
                changed(&|node| *node = json!([1]));
                for (key, child) in object {
                    path.push(PathStep::Key(key.clone()));
                    visit_mutations(root, child, path, check);
                    path.pop();
                }
            }
            Value::Array(items) => {
                changed(&|node| node.as_array_mut().unwrap().push(json!({ "x": 1 })));
                changed(&|node| *node = json!({ "y": 1 }));
                for (index, child) in items.iter().enumerate() {
                    path.push(PathStep::Index(index));
                    visit_mutations(root, child, path, check);
                    path.pop();
                }
            }
            _ => changed(&|node| *node = json!({ "w": [1] })),
        }
    }

    enum PathStep {
        Key(String),
        Index(usize),
    }

    fn with_node(root: &mut Value, path: &[PathStep], change: &dyn Fn(&mut Value)) {
        let mut node = root;
        for step in path {
            node = match step {
                PathStep::Key(key) => node.get_mut(key).unwrap(),
                PathStep::Index(index) => node.get_mut(*index).unwrap(),
            };
        }
        change(node);
    }
}
