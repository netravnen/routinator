//! Parsing BER encoded values.
//!
//! This is an internal module. Its public types are re-exported by the
//! parent.

use bytes::Bytes;
use super::error::Error;
use super::length::Length;
use super::source::{CaptureSource, LimitedSource, Source};
use super::tag::Tag;


//------------ Content -------------------------------------------------------

/// The content octets of a BER encoded value.
///
/// A value is either primitive, containing actual octets of an actual value,
/// or constructed, in which case its content contains additional BER encoded
/// values. This enum is useful for cases where a certain type may be encoded
/// as either a primitive value or a complex constructed value.
pub enum Content<'a, S: 'a> {
    /// The value is a primitive value.
    Primitive(Primitive<'a, S>),

    /// The value is a constructed value.
    Constructed(Constructed<'a, S>)
}

impl<'a, S: Source + 'a> Content<'a, S> {
    /// Checkes that the content has been parsed completely.
    ///
    /// Returns a malformed error if not.
    fn exhausted(self) -> Result<(), S::Err> {
        match self {
            Content::Primitive(inner) => inner.exhausted(),
            Content::Constructed(mut inner) => inner.exhausted()
        }
    }

    /// Returns the encoding mode used by the value.
    pub fn mode(&self) -> Mode {
        match *self {
            Content::Primitive(ref inner) => inner.mode(),
            Content::Constructed(ref inner) => inner.mode()
        }
    }

    /// Returns whether this value is a primitive value.
    pub fn is_primitive(&self) -> bool {
        match *self {
            Content::Primitive(_) => true,
            Content::Constructed(_) => false,
        }
    }

    /// Returns whether this value is a constructed value.
    pub fn is_constructed(&self) -> bool {
        match *self {
            Content::Primitive(_) => false,
            Content::Constructed(_) => true,
        }
    }

    /// Converts a reference into on to a primitive value or errors out.
    pub fn as_primitive(&mut self) -> Result<&mut Primitive<'a, S>, S::Err> {
        match *self {
            Content::Primitive(ref mut inner) => Ok(inner),
            Content::Constructed(_) => {
                xerr!(Err(Error::Malformed.into()))
            }
        }
    }

    /// Converts a reference into on to a constructed value or errors out.
    pub fn as_constructed(
        &mut self
    ) -> Result<&mut Constructed<'a, S>, S::Err> {
        match *self {
            Content::Primitive(_) => {
                xerr!(Err(Error::Malformed.into()))
            }
            Content::Constructed(ref mut inner) => Ok(inner),
        }
    }

    /// Converts the content into a bytes value.
    ///
    /// If the value is a constructed value, makes sure that all contained
    /// constructed values are properly encoded. Primitive values will only
    /// be checked for valid tag and length encodings.
    pub fn into_bytes(&mut self) -> Result<Bytes, S::Err> {
        match *self {
            Content::Primitive(ref mut inner) => inner.take_all(),
            Content::Constructed(ref mut inner) => inner.take_all(),
        }
    }
}

impl<'a, S: Source + 'a> Content<'a, S> {
    /// Converts content into a `u8`.
    ///
    /// If the content is not primitive or does not contain a single BER
    /// encoded INTEGER value between 0 and 256, returns a malformed error.
    pub fn to_u8(&mut self) -> Result<u8, S::Err> {
        if let Content::Primitive(ref mut prim) = *self {
            prim.to_u8()
        }
        else {
            xerr!(Err(Error::Malformed.into()))
        }
    }

    /// Skips over the content if it contains an INTEGER of value `expected`.
    ///
    /// The content needs to be primitive and contain a validly encoded
    /// integer of value `expected` or else a malformed error will be
    /// returned.
    pub fn skip_u8_if(&mut self, expected: u8) -> Result<(), S::Err> {
        let res = self.to_u8()?;
        if res == expected {
            Ok(())
        }
        else {
            xerr!(Err(Error::Malformed.into()))
        }
    }

    /// Converts content into a `u32`.
    ///
    /// If the content is not primitive or does not contain a single BER
    /// encoded INTEGER value between 0 and 2^32-1, returns a malformed error.
    pub fn to_u32(&mut self) -> Result<u32, S::Err> {
        if let Content::Primitive(ref mut prim) = *self {
            prim.to_u32()
        }
        else {
            xerr!(Err(Error::Malformed.into()))
        }
    }


    /// Converts content into a `u64`.
    ///
    /// If the content is not primitive or does not contain a single BER
    /// encoded INTEGER value between 0 and 2^64-1, returns a malformed error.
    pub fn to_u64(&mut self) -> Result<u64, S::Err> {
        if let Content::Primitive(ref mut prim) = *self {
            prim.to_u64()
        }
        else {
            xerr!(Err(Error::Malformed.into()))
        }
    }

    pub fn to_unsigned(&mut self) -> Result<Bytes, S::Err> {
        self.as_primitive()?.to_unsigned()
    }

    /// Converts the content into a NULL value.
    ///
    /// If the content isn’t primitive and contains a single BER encoded
    /// NULL value (i.e., nothing), returns a malformed error.
    pub fn to_null(&mut self) -> Result<(), S::Err> {
        if let Content::Primitive(ref mut prim) = *self {
            prim.to_null()
        }
        else {
            xerr!(Err(Error::Malformed.into()))
        }
    }
}


//------------ Primitive -----------------------------------------------------

/// The content octets of a primitive value.
///
/// You will receive a reference to a value of this type through a closure,
/// possibly wrapped in a `Content` value. Your task will be to read out all
/// the octets of the value before returning from the closure or produce an
/// error if the value isn’t correctly encoded. If you read less octets than
/// are available, whoever called the closure will produce an error after
/// you returned. Thus, you can read as many octets as you expect and not
/// bother to check whether that was all available octets.
///
/// The most basic way to do this is through the primitive’s implementation
/// of the `Source` trait. Thus, you can gain access to some or all of the
/// octets and mark them read by advancing over them. You can safely attempt
/// to read more octets than available as that will reliably result in a 
/// malformed error.
///
/// A number of methods are available to deal with the encodings defined for
/// various types. These are prefixed by `to_` to indicate that they are
/// intended to convert the content to a certain type. They all read exactly
/// one encoded value.
///
/// The value provides access to the decoding mode via the `mode` method.
/// All methodes that decode data will honour the decoding mode and enforce
/// that data is encoded according to the mode.
pub struct Primitive<'a, S: 'a> {
    /// The underlying source limited to the length of the value.
    source: &'a mut LimitedSource<S>,

    /// The decoding mode to operate in.
    mode: Mode,
}

/// # Value Management
///
impl<'a, S: 'a> Primitive<'a, S> {
    /// Creates a new primitive from the given source and mode.
    fn new(source: &'a mut LimitedSource<S>, mode: Mode) -> Self {
        Primitive { source, mode }
    }

    /// Returns the current decoding mode.
    ///
    /// The higher-level `to_` methods will use this mode to enforce that
    /// data is encoded correctly.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Sets the current decoding mode.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode
    }
}

/// # High-level Decoding
///
impl<'a, S: Source + 'a> Primitive<'a, S> {
    /// Parses the primitive value as a BOOLEAN value.
    ///
    /// A boolean value is encoded as a primitive value with exactly one
    /// octet of content. If the octet is 0, the result is `false`, otherwise
    /// it is `true`. In DER mode, the octet has to be `0` for a value of
    /// `false`, `0xFF` for a value of `true`, and all other values are not
    /// permitted.
    pub fn to_bool(&mut self) -> Result<bool, S::Err> {
        let res = self.take_u8()?;
        if self.mode == Mode::Der {
            match res {
                0 => Ok(false),
                0xFF => Ok(true),
                _ => {
                    xerr!(Err(Error::Malformed.into()))
                }
            }
        }
        else {
            Ok(res != 0)
        }
    }

    /// Parses the primitive value as an INTEGER limited to a `u32`.
    pub fn to_u8(&mut self) -> Result<u8, S::Err> {
        self.check_int_head()?;
        self.check_unsigned()?;
        match self.remaining() {
            1 => self.take_u8(), // sign bit has been checked above.
            2 => {
                // First byte must be 0x00, second is the result.
                if self.take_u8()? != 0 {
                    xerr!(Err(Error::Malformed.into()))
                }
                else {
                    self.take_u8()
                }
            }
            _ => xerr!(Err(Error::Malformed.into()))
        }
    }

    /// Parses the primitive value as an INTEGER limited to a `u32`.
    pub fn to_u32(&mut self) -> Result<u32, S::Err> {
        self.check_int_head()?;
        self.check_unsigned()?;
        match self.remaining() {
            1 => Ok(self.take_u8()? as u32),
            2 => {
                Ok(
                    (self.take_u8()? as u32) << 8
                    | (self.take_u8()? as u32)
                )
            }
            3 => {
                Ok(
                    (self.take_u8()? as u32) << 16
                    | (self.take_u8()? as u32) << 8
                    | (self.take_u8()? as u32)
                )
            }
            4 => {
                Ok(
                    (self.take_u8()? as u32) << 24
                    | (self.take_u8()? as u32) << 16
                    | (self.take_u8()? as u32) << 8
                    | (self.take_u8()? as u32)
                )
            }
            5 => {
                if self.take_u8()? != 0 {
                    xerr!(return Err(Error::Malformed.into()));
                }
                Ok(
                    (self.take_u8()? as u32) << 24
                    | (self.take_u8()? as u32) << 16
                    | (self.take_u8()? as u32) << 8
                    | (self.take_u8()? as u32)
                )
            }
            _ => xerr!(Err(Error::Malformed.into()))
        }
    }

    /// Parses the primitive value as a INTEGER value limited to a `u64`.
    pub fn to_u64(&mut self) -> Result<u64, S::Err> {
        self.check_int_head()?;
        self.check_unsigned()?;
        if self.remaining() == 9 {
            if self.take_u8()? != 0 {
                xerr!(return Err(Error::Malformed.into()))
            }
        }
        let mut res = 0;
        for _ in 0..8 {
            if self.remaining() == 0 {
                break
            }
            res = res << 8 | (self.take_u8()? as u64);
        }
        Ok(res)
    }

    /// Parses the primitive value as an INTEGER of unlimited length.
    pub fn to_unsigned(&mut self) -> Result<Bytes, S::Err> {
        self.check_int_head()?;
        self.check_unsigned()?;
        self.take_all()
    }

    /// Checks that an integer is started correctly.
    ///
    /// Specifically, checks that there is at least one octet and that the
    /// first nine bits of a multi-octet integer are not all the same.
    ///
    /// The latter ensures that an integer is encoded in the smallest possible
    /// number of octets.
    fn check_int_head(&mut self) -> Result<(), S::Err> {
        if self.request(2)? == 0 {
            xerr!(return Err(Error::Malformed.into()))
        }
        let slice = self.slice();
        match (slice.get(0), slice.get(1).map(|x| x & 0x80 != 0)) {
            (Some(0), Some(false)) => {
                xerr!(Err(Error::Malformed.into()))
            }
            (Some(0xFF), Some(true)) => {
                xerr!(Err(Error::Malformed.into()))
            }
            (Some(x), _) if x & 0x80 != 0 => {
                xerr!(Err(Error::Malformed.into()))
            }
            _ => Ok(())
        }
    }

    /// Checks that an integer is unsigned.
    ///
    /// Always call this after `check_int_head`.
    fn check_unsigned(&self) -> Result<(), S::Err> {
        if self.slice().get(0).unwrap() & 0x80 != 0 {
            xerr!(Err(Error::Malformed.into()))
        }
        else {
            Ok(())
        }
    }

    pub fn to_null(&mut self) -> Result<(), S::Err> {
        // The rest is taken care of by the exhausted check later ...
        Ok(())
    }
}

impl<'a, S: Source + 'a> Primitive<'a, S> {
    pub fn remaining(&self) -> usize {
        self.source.limit().unwrap()
    }

    pub fn skip_all(&mut self) -> Result<(), S::Err> {
        self.source.skip_all()
    }

    pub fn take_all(&mut self) -> Result<Bytes, S::Err> {
        self.source.take_all()
    }

    pub fn slice_all(&mut self) -> Result<&[u8], S::Err> {
        let remaining = self.remaining();
        self.source.request(remaining)?;
        Ok(&self.source.slice()[..remaining])
    }

    fn exhausted(self) -> Result<(), S::Err> {
        self.source.exhausted()
    }
}


impl<'a, S: Source + 'a> Source for Primitive<'a, S> {
    type Err = S::Err;

    fn request(&mut self, len: usize) -> Result<usize, Self::Err> {
        self.source.request(len)
    }

    fn advance(&mut self, len: usize) -> Result<(), Self::Err> {
        self.source.advance(len)
    }

    fn slice(&self) -> &[u8] {
        self.source.slice()
    }

    fn bytes(&self, start: usize, end: usize) -> Bytes {
        self.source.bytes(start, end)
    }
}


//------------ Constructed ---------------------------------------------------

#[derive(Debug)]
pub struct Constructed<'a, S: 'a> {
    source: &'a mut LimitedSource<S>,
    state: State,
    mode: Mode,
}


impl<'a, S: Source + 'a> Constructed<'a, S> {
    fn new(
        source: &'a mut LimitedSource<S>,
        state: State,
        mode: Mode
    ) -> Self {
        Constructed { source, state, mode }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode
    }
}

impl<'a, S: Source + 'a> Constructed<'a, S> {
    fn exhausted(&mut self) -> Result<(), S::Err> {
        match self.state {
            State::Done => Ok(()),
            State::Definite => {
                self.source.exhausted()
            }
            State::Indefinite => {
                let (tag, constructed) = Tag::take_from(self.source)?;
                if tag != Tag::END_OF_VALUE {
                    xerr!(Err(Error::Malformed.into()))
                }
                else if constructed {
                    xerr!(Err(Error::Malformed.into()))
                }
                else if !Length::take_from(self.source)?.is_zero() {
                    xerr!(Err(Error::Malformed.into()))
                }
                else {
                    Ok(())
                }
            }
            State::Unbounded => Ok(())
        }
    }

    fn is_exhausted(&self) -> bool {
        match self.state {
            State::Definite => {
                self.source.limit().unwrap() == 0
            }
            State::Indefinite => false,
            State::Done => true,
            State::Unbounded => false,
        }
    }

    fn take_value<F, T>(
        &mut self,
        expected: Option<Tag>,
        op: F
    ) -> Result<Option<T>, S::Err>
    where F: FnOnce(Tag, &mut Content<S>) -> Result<T, S::Err> {
        if self.is_exhausted() {
            return Ok(None)
        }
        let (tag, constructed) = if let Some(expected) = expected {
            (
                expected,
                match expected.take_from_if(self.source)? {
                    Some(compressed) => compressed,
                    None => return Ok(None)
                }
            )
        }
        else {
            Tag::take_from(self.source)?
        };
        let length = Length::take_from(self.source)?;

        if tag == Tag::END_OF_VALUE {
            if let State::Indefinite = self.state {
                if constructed {
                    xerr!(return Err(Error::Malformed.into()))
                }
                if !length.is_zero() {
                    xerr!(return Err(Error::Malformed.into()))
                }
                self.state = State::Done;
                return Ok(None)
            }
            else {
                xerr!(return Err(Error::Malformed.into()))
            }
        }

        match length {
            Length::Definite(len) => {
                let old_limit = self.source.limit_further(Some(len));
                let res = {
                    let mut content = if constructed {
                        // Definite length constructed values are not allowed
                        // in CER.
                        if self.mode == Mode::Cer {
                            xerr!(return Err(Error::Malformed.into()))
                        }
                        Content::Constructed(
                            Constructed::new(
                                self.source, State::Definite, self.mode
                            )
                        )
                    }
                    else {
                        Content::Primitive(
                            Primitive::new(self.source, self.mode)
                        )
                    };
                    let res = op(tag, &mut content)?;
                    content.exhausted()?;
                    res
                };
                self.source.set_limit(old_limit.map(|x| x - len));
                Ok(Some(res))
            }
            Length::Indefinite => {
                if !constructed {
                    xerr!(return Err(Error::Malformed.into()))
                }
                else if self.mode == Mode::Der {
                    xerr!(return Err(Error::Malformed.into()))
                }
                let mut content = Content::Constructed(
                    Constructed::new(self.source, State::Indefinite, self.mode)
                );
                let res = op(tag, &mut content)?;
                content.exhausted()?;
                Ok(Some(res))
            }
        }
    }
}

impl<'a, S: Source + 'a> Constructed<'a, S> {
    pub fn value<F, T>(&mut self, op: F) -> Result<T, S::Err>
    where F: FnOnce(Tag, &mut Content<S>) -> Result<T, S::Err> {
        match self.take_value(None, op)? {
            Some(res) => Ok(res),
            None => {
                xerr!(Err(Error::Malformed.into()))
            }
        }
    }

    pub fn opt_value<F, T>(&mut self, op: F) -> Result<Option<T>, S::Err>
    where F: FnOnce(Tag, &mut Content<S>) -> Result<T, S::Err> {
        self.take_value(None, op)
    }

    pub fn value_if<F, T>(&mut self, expected: Tag, op: F) -> Result<T, S::Err>
    where F: FnOnce(&mut Content<S>) -> Result<T, S::Err> {
        let res = self.take_value(Some(expected), |_, content| {
            op(content)
        })?;
        match res {
            Some(res) => Ok(res),
            None => {
                xerr!(Err(Error::Malformed.into()))
            }
        }
    }

    pub fn opt_value_if<F, T>(
        &mut self,
        expected: Tag,
        op: F
    ) -> Result<Option<T>, S::Err>
    where F: FnOnce(&mut Content<S>) -> Result<T, S::Err> {
        self.take_value(Some(expected), |_, content| op(content))
    }

    pub fn constructed<F, T>(&mut self, op: F) -> Result<T, S::Err>
    where F: FnOnce(Tag, &mut Constructed<S>) -> Result<T, S::Err> {
        match self.opt_constructed(op)? {
            Some(res) => Ok(res),
            None => {
                xerr!(Err(Error::Malformed.into()))
            }
        }
    }

    pub fn opt_constructed<F, T>(&mut self, op: F) -> Result<Option<T>, S::Err>
    where F: FnOnce(Tag, &mut Constructed<S>) -> Result<T, S::Err> {
        self.take_value(None, |tag, content| {
            op(tag, content.as_constructed()?)
        })
    }

    pub fn constructed_if<F, T>(
        &mut self,
        expected: Tag,
        op: F
    ) -> Result<T, S::Err>
    where F: FnOnce(&mut Constructed<S>) -> Result<T, S::Err> {
        match self.opt_constructed_if(expected, op)? {
            Some(res) => Ok(res),
            None => {
                xerr!(Err(Error::Malformed.into()))
            }
        }
    }

    pub fn opt_constructed_if<F, T>(
        &mut self,
        expected: Tag,
        op: F
    ) -> Result<Option<T>, S::Err>
    where F: FnOnce(&mut Constructed<S>) -> Result<T, S::Err> {
        self.take_value(Some(expected), |_, content| {
            op(content.as_constructed()?)
        })
    }

    pub fn primitive<F, T>(&mut self, op: F) -> Result<T, S::Err>
    where F: FnOnce(Tag, &mut Primitive<S>) -> Result<T, S::Err> {
        match self.opt_primitive(op)? {
            Some(res) => Ok(res),
            None => {
                xerr!(Err(Error::Malformed.into()))
            }
        }
    }

    pub fn opt_primitive<F, T>(&mut self, op: F) -> Result<Option<T>, S::Err>
    where F: FnOnce(Tag, &mut Primitive<S>) -> Result<T, S::Err> {
        self.take_value(None, |tag, content| {
            op(tag, content.as_primitive()?)
        })
    }

    pub fn primitive_if<F, T>(
        &mut self,
        expected: Tag,
        op: F
    ) -> Result<T, S::Err>
    where F: FnOnce(&mut Primitive<S>) -> Result<T, S::Err> {
        match self.opt_primitive_if(expected, op)? {
            Some(res) => Ok(res),
            None => {
                xerr!(Err(Error::Malformed.into()))
            }
        }
    }

    pub fn opt_primitive_if<F, T>(
        &mut self,
        expected: Tag,
        op: F
    ) -> Result<Option<T>, S::Err>
    where F: FnOnce(&mut Primitive<S>) -> Result<T, S::Err> {
        self.take_value(Some(expected), |_, content| {
            op(content.as_primitive()?)
        })
    }

    pub fn capture<F>(&mut self, op: F) -> Result<Bytes, S::Err>
    where
        F: FnOnce(
            &mut Constructed<CaptureSource<LimitedSource<S>>>
        ) -> Result<(), S::Err>
    {
        let limit = self.source.limit();
        let mut source = LimitedSource::new(CaptureSource::new(self.source));
        source.set_limit(limit);
        {
            let mut constructed = Constructed::new(
                &mut source, self.state, self.mode
            );
            op(&mut constructed)?
        }
        Ok(source.unwrap().into_bytes())
    }

    pub fn take_one(&mut self) -> Result<Bytes, S::Err> {
        self.capture(|cons| {
            match cons.skip_one()? {
                Some(()) => Ok(()),
                None => {
                    xerr!(Err(Error::Malformed.into()))
                }
            }
        })
    }

    pub fn take_all(&mut self) -> Result<Bytes, S::Err> {
        self.capture(|cons| cons.skip_all())
    }

    fn skip_all(&mut self) -> Result<(), S::Err> {
        while let Some(()) = self.skip_one()? { }
        Ok(())
    }

    fn skip_one(&mut self) -> Result<Option<()>, S::Err> {
        self.opt_value(|_tag, content| {
            match *content {
                Content::Primitive(ref mut inner) => {
                    inner.skip_all()
                }
                Content::Constructed(ref mut inner) => {
                    inner.skip_all()?;
                    Ok(())
                }
            }
        })
    }
}


impl<'a, S: Source + 'a> Constructed<'a, S> {
    pub fn take_bool(&mut self) -> Result<bool, S::Err> {
        self.primitive_if(Tag::BOOLEAN, |prim| prim.to_bool())
    }

    pub fn take_opt_bool(&mut self) -> Result<Option<bool>, S::Err> {
        self.opt_primitive_if(Tag::BOOLEAN, |prim| prim.to_bool())
    }

    pub fn skip_opt_null(&mut self) -> Result<(), S::Err> {
        self.opt_primitive_if(Tag::NULL, |_| Ok(())).map(|_| ())
    }

    pub fn take_opt_u8(&mut self) -> Result<Option<u8>, S::Err> {
        self.opt_primitive_if(Tag::INTEGER, |prim| prim.take_u8())
    }

    pub fn skip_u8_if(&mut self, expected: u8) -> Result<(), S::Err> {
        self.primitive_if(Tag::INTEGER, |prim| {
            let got = prim.take_u8()?;
            if got != expected {
                xerr!(Err(Error::Malformed.into()))
            }
            else {
                Ok(())
            }
        })
    }

    pub fn skip_opt_u8_if(&mut self, expected: u8) -> Result<(), S::Err> {
        self.opt_primitive_if(Tag::INTEGER, |prim| {
            let got = prim.take_u8()?;
            if got != expected {
                xerr!(Err(Error::Malformed.into()))
            }
            else {
                Ok(())
            }
        }).map(|_| ())
    }

    pub fn take_u32(&mut self) -> Result<u32, S::Err> {
        self.primitive_if(Tag::INTEGER, |prim| prim.to_u32())
    }

    pub fn take_u64(&mut self) -> Result<u64, S::Err> {
        self.primitive_if(Tag::INTEGER, |prim| prim.to_u64())
    }

    pub fn take_unsigned(&mut self) -> Result<Bytes, S::Err> {
        self.primitive_if(Tag::INTEGER, |prim| prim.to_unsigned())
    }

    pub fn sequence<F, T>(&mut self, op: F) -> Result<T, S::Err>
    where F: FnOnce(&mut Constructed<S>) -> Result<T, S::Err> {
        self.constructed_if(Tag::SEQUENCE, op)
    }

    pub fn opt_sequence<F, T>(&mut self, op: F) -> Result<Option<T>, S::Err>
    where F: FnOnce(&mut Constructed<S>) -> Result<T, S::Err> {
        self.opt_constructed_if(Tag::SEQUENCE, op)
    }

    pub fn set<F, T>(&mut self, op: F) -> Result<T, S::Err>
    where F: FnOnce(&mut Constructed<S>) -> Result<T, S::Err> {
        self.constructed_if(Tag::SET, op)
    }

    pub fn opt_set<F, T>(&mut self, op: F) -> Result<Option<T>, S::Err>
    where F: FnOnce(&mut Constructed<S>) -> Result<T, S::Err> {
        self.opt_constructed_if(Tag::SET, op)
    }
}


//------------ Mode ----------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    /// Basic Encoding Rules.
    ///
    /// These are the most flexible rules, allowing alternative encodings for
    /// some types as well as indefinite length values.
    Ber,

    /// Canonical Encoding Rules.
    ///
    /// These rules always employ indefinite length encoding for constructed
    /// values and the shortest possible form for primitive values.  There
    /// are additional restrictions for certain types.
    Cer,

    /// Distinguished Encoding Rules.
    ///
    /// These rules always employ definite length values and require the
    /// shortest possible encoding. Additional rules apply to some types.
    Der,
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Ber
    }
}

impl Mode {
    pub fn decode<S, F, T>(self, source: S, op: F) -> Result<T, S::Err>
    where
        S: Source,
        F: FnOnce(&mut Constructed<S>) -> Result<T, S::Err>
    {
        let mut source = LimitedSource::new(source);
        let mut cons = Constructed::new(&mut source, State::Unbounded, self);
        let res = op(&mut cons)?;
        cons.exhausted()?;
        Ok(res)
    }
}


//------------ State ---------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum State {
    /// We are reading until the end of the reader.
    Definite,

    /// Indefinite value, we haven’t reached the end yet.
    Indefinite,

    /// End of indefinite value reached.
    Done,

    /// Unbounded value: read as far as we get.
    Unbounded,
}

