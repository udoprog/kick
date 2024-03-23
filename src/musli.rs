pub(crate) mod string {
    use std::fmt;
    use std::str::FromStr;

    use musli::{Context, Decoder, Encoder};

    pub(crate) fn encode<T, E>(doc: &T, _: &E::Cx, encoder: E) -> Result<E::Ok, E::Error>
    where
        T: fmt::Display,
        E: Encoder,
    {
        encoder.collect_string(doc)
    }

    pub(crate) fn decode<'de, T, D>(cx: &D::Cx, decoder: D) -> Result<T, D::Error>
    where
        T: FromStr,
        T::Err: fmt::Display,
        D: Decoder<'de>,
    {
        decoder.decode_string(musli::utils::visit_owned_fn(
            "a value decoded from a string",
            |string: &str| string.parse().map_err(cx.map_message()),
        ))
    }
}
