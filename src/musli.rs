pub(crate) mod document {
    use musli::{Context, Decode, Decoder, Encoder, Mode};
    use toml_edit::Document;

    pub(crate) fn encode<M, C, E>(doc: &Document, cx: &mut C, encoder: E) -> Result<E::Ok, C::Error>
    where
        M: Mode,
        C: Context<Input = E::Error>,
        E: Encoder,
    {
        let string = doc.to_string();
        encoder.encode_string(cx, &string)
    }

    pub(crate) fn decode<'de, M, C, D>(cx: &mut C, decoder: D) -> Result<Document, C::Error>
    where
        M: Mode,
        C: Context<Input = D::Error>,
        D: Decoder<'de>,
    {
        let string = <String as Decode<M>>::decode(cx, decoder)?;
        let doc = string.parse().map_err(|error| cx.custom(error))?;
        Ok(doc)
    }
}

pub(crate) mod relative_path {
    use musli::{Context, Decode, Decoder, Encoder, Mode};
    use relative_path::{RelativePath, RelativePathBuf};

    pub(crate) fn encode<M, C, E>(
        path: &RelativePath,
        cx: &mut C,
        encoder: E,
    ) -> Result<E::Ok, C::Error>
    where
        M: Mode,
        C: Context<Input = E::Error>,
        E: Encoder,
    {
        encoder.encode_string(cx, path.as_str())
    }

    pub(crate) fn decode<'de, M, C, D>(cx: &mut C, decoder: D) -> Result<RelativePathBuf, C::Error>
    where
        M: Mode,
        C: Context<Input = D::Error>,
        D: Decoder<'de>,
    {
        let string = <String as Decode<M>>::decode(cx, decoder)?;
        Ok(RelativePathBuf::from(string))
    }
}

pub(crate) mod version {
    use musli::{Context, Decode, Decoder, Encoder, Mode};
    use semver::Version;

    pub(crate) fn encode<M, C, E>(
        version: &Version,
        cx: &mut C,
        encoder: E,
    ) -> Result<E::Ok, C::Error>
    where
        M: Mode,
        C: Context<Input = E::Error>,
        E: Encoder,
    {
        let version = version.to_string();
        encoder.encode_string(cx, &version)
    }

    pub(crate) fn decode<'de, M, C, D>(cx: &mut C, decoder: D) -> Result<Version, C::Error>
    where
        M: Mode,
        C: Context<Input = D::Error>,
        D: Decoder<'de>,
    {
        let string = <String as Decode<M>>::decode(cx, decoder)?;
        let version = string.parse().map_err(|error| cx.custom(error))?;
        Ok(version)
    }
}

pub(crate) mod url {
    use musli::{Context, Decode, Decoder, Encoder, Mode};
    use url::Url;

    pub(crate) fn encode<M, C, E>(url: &Url, cx: &mut C, encoder: E) -> Result<E::Ok, C::Error>
    where
        M: Mode,
        C: Context<Input = E::Error>,
        E: Encoder,
    {
        encoder.encode_string(cx, url.as_str())
    }

    pub(crate) fn decode<'de, M, C, D>(cx: &mut C, decoder: D) -> Result<Url, C::Error>
    where
        M: Mode,
        C: Context<Input = D::Error>,
        D: Decoder<'de>,
    {
        let string = <String as Decode<M>>::decode(cx, decoder)?;
        let url = Url::parse(&string).map_err(|error| cx.custom(error))?;
        Ok(url)
    }
}

pub(crate) mod serde {
    use musli::{Context, Decode, Decoder, Encoder, Mode};
    use serde::{Deserialize, Serialize};

    pub(crate) fn encode<M, C, E, T>(value: &T, cx: &mut C, encoder: E) -> Result<E::Ok, C::Error>
    where
        M: Mode,
        C: Context<Input = E::Error>,
        E: Encoder,
        T: Serialize,
    {
        todo!()
    }

    pub(crate) fn decode<'de, M, C, D, T>(cx: &mut C, decoder: D) -> Result<T, C::Error>
    where
        M: Mode,
        C: Context<Input = D::Error>,
        D: Decoder<'de>,
        T: Deserialize<'de>,
    {
        todo!()
    }
}
